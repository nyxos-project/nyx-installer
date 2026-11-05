//! Carrying out a plan's steps, reporting each as it goes. A failure stops everything and
//! says which step and why; what it printed is in the log.

use crate::plan::Step;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Step `index` of `total` started.
    Step {
        index: usize,
        total: usize,
        title: String,
    },
    /// A line a command printed.
    Line(String),
    Done,
    Failed {
        title: String,
        why: String,
    },
}

/// Run `steps`, sending events to `send`. With `dry`, only describe them.
pub fn run(steps: &[Step], dry: bool, send: &mut dyn FnMut(Event)) -> Result<()> {
    let total = steps.len();
    for (index, step) in steps.iter().enumerate() {
        let title = match step {
            Step::Run { title, .. } | Step::RunTo { title, .. } => title.clone(),
            Step::Write { path, .. } => format!("Writing {}", path.rsplit('/').next().unwrap_or(path)),
        };
        send(Event::Step { index, total, title: title.clone() });
        let result = if dry { describe(step, send) } else { one(step, send) };
        if let Err(e) = result {
            let why = format!("{e:#}");
            send(Event::Failed { title, why: why.clone() });
            bail!(why);
        }
    }
    send(Event::Done);
    Ok(())
}

fn describe(step: &Step, send: &mut dyn FnMut(Event)) -> Result<()> {
    send(Event::Line(crate::plan::Plan::describe(std::slice::from_ref(step))));
    Ok(())
}

fn one(step: &Step, send: &mut dyn FnMut(Event)) -> Result<()> {
    match step {
        Step::Write { path, contents } => {
            if let Some(dir) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, contents).with_context(|| format!("can't write {path}"))
        }
        Step::RunTo { args, path, .. } => {
            let out = Command::new(&args[0])
                .args(&args[1..])
                .stdin(Stdio::null())
                .output()
                .with_context(|| format!("can't start {}", args[0]))?;
            let err = String::from_utf8_lossy(&out.stderr);
            for l in err.lines() {
                send(Event::Line(l.to_string()));
            }
            if !out.status.success() {
                // The first error says why; a build's last lines are often just its output path.
                let why = err
                    .lines()
                    .find(|l| l.trim_start().starts_with("error:"))
                    .map(|l| l.trim().to_string())
                    .unwrap_or_else(|| last_line(&err));
                bail!("{} failed: {why}", args[0]);
            }
            std::fs::write(path, &out.stdout).with_context(|| format!("can't write {path}"))
        }
        Step::Run { args, input, .. } => {
            send(Event::Line(format!("$ {}", args.join(" "))));
            let mut child = Command::new(&args[0])
                .args(&args[1..])
                .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .with_context(|| format!("can't start {}", args[0]))?;
            if let Some(text) = input {
                let mut stdin = child.stdin.take().context("no stdin")?;
                stdin.write_all(text.as_bytes())?;
            }
            let err = child.stderr.take().context("no stderr")?;
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            let tx2 = tx.clone();
            let errors = std::thread::spawn(move || {
                let mut last = String::new();
                for l in BufReader::new(err).lines().map_while(Result::ok) {
                    if !l.trim().is_empty() {
                        last = l.clone();
                    }
                    let _ = tx2.send(l);
                }
                last
            });
            let out = child.stdout.take().context("no stdout")?;
            let outs = std::thread::spawn(move || {
                for l in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(l);
                }
            });
            for l in rx {
                send(Event::Line(l));
            }
            let _ = outs.join();
            let last = errors.join().unwrap_or_default();
            let status = child.wait()?;
            if !status.success() {
                bail!("{} failed: {}", args[0], if last.is_empty() { status.to_string() } else { last_line(&last) });
            }
            Ok(())
        }
    }
}

fn last_line(text: &str) -> String {
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_run_in_order_and_a_failure_stops_them() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("a/b.txt").display().to_string();
        let out = t.path().join("echo.txt").display().to_string();
        let steps = vec![
            Step::Write { path: file.clone(), contents: "hi".into() },
            Step::RunTo { title: "Echo".into(), args: vec!["echo".into(), "made".into()], path: out.clone() },
            Step::Run { title: "Read".into(), args: vec!["cat".into()], input: Some("secret\n".into()) },
            Step::Run { title: "Fail".into(), args: vec!["sh".into(), "-c".into(), "echo nope >&2; exit 3".into()], input: None },
            Step::Write { path: t.path().join("never").display().to_string(), contents: String::new() },
        ];
        let mut events = vec![];
        let r = run(&steps, false, &mut |e| events.push(e));
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "made\n");
        assert!(events.contains(&Event::Line("secret".into())));
        assert!(events.contains(&Event::Failed { title: "Fail".into(), why: "sh failed: nope".into() }));
        assert!(!t.path().join("never").exists());
        assert!(!events.contains(&Event::Done));
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("x").display().to_string();
        let mut lines = vec![];
        run(&[Step::Write { path: file.clone(), contents: "x".into() }], true, &mut |e| lines.push(e)).unwrap();
        assert!(!std::path::Path::new(&file).exists());
        assert!(lines.contains(&Event::Done));
    }
}

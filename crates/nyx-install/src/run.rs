//! Carrying out a plan's steps, reporting each as it goes. A failure stops everything and
//! says which step and why, in the installer's words; what the tools printed goes to the
//! log file ([`LOG`]), not the screen.

use crate::plan::Step;
use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// What every step's tools printed. The plan copies it to the installed system.
pub const LOG: &str = "/tmp/nyxos-install.log";

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Step `index` of `total` started.
    Step {
        index: usize,
        total: usize,
        title: String,
    },
    /// What a dry run would do.
    Line(String),
    Done,
    Failed {
        title: String,
        why: String,
    },
}

/// Run `steps`, sending events to `send` and the tools' output to [`LOG`]. With `dry`,
/// only describe them.
pub fn run(steps: &[Step], dry: bool, send: &mut dyn FnMut(Event)) -> Result<()> {
    run_logged(steps, dry, Path::new(LOG), send)
}

pub fn run_logged(steps: &[Step], dry: bool, log: &Path, send: &mut dyn FnMut(Event)) -> Result<()> {
    let mut log = File::create(log).with_context(|| format!("can't write {}", log.display()))?;
    let total = steps.len();
    for (index, step) in steps.iter().enumerate() {
        let title = match step {
            Step::Run { title, .. } | Step::RunTo { title, .. } => title.clone(),
            Step::Write { path, .. } => format!("Writing {}", path.rsplit('/').next().unwrap_or(path)),
        };
        send(Event::Step { index, total, title: title.clone() });
        let _ = writeln!(log, "== {title}");
        let result = if dry { describe(step, send) } else { one(step, &mut log) };
        if let Err(e) = result {
            let why = plain(&format!("{e:#}"));
            let _ = writeln!(log, "!! {why}");
            send(Event::Failed { title, why: why.clone() });
            bail!(why);
        }
    }
    send(Event::Done);
    Ok(())
}

/// A tool's complaint in the installer's words: no "error:", no store paths' hashes, and
/// no names of the tools underneath.
pub fn plain(why: &str) -> String {
    let mut out = why.trim().trim_start_matches("error:").trim().to_string();
    while let Some(i) = out.find("/nix/store/") {
        let rest = &out[i + "/nix/store/".len()..];
        let skip = rest.find('-').filter(|&n| n == 32).map_or(0, |n| n + 1);
        out = format!("{}{}", &out[..i], &rest[skip..]);
    }
    for tool in ["nixos-generate-config", "nixos-install", "nixos-enter"] {
        out = out.replace(&format!("{tool} failed: "), "").replace(tool, "the installer");
    }
    out
}

/// What the hardware detection writes starts with its own comment (about another
/// system's configuration file); NyxOS's says what the file is.
fn tidy(path: &str, out: &[u8]) -> Vec<u8> {
    if !path.ends_with("hardware-configuration.nix") {
        return out.to_vec();
    }
    let text = String::from_utf8_lossy(out);
    let body: Vec<&str> = text.lines().skip_while(|l| l.starts_with('#')).collect();
    format!(
        "# This computer's hardware, as the NyxOS installer found it. NyxOS's own settings are\n\
         # in host.nix beside it.\n{}\n",
        body.join("\n")
    )
    .into_bytes()
}

fn describe(step: &Step, send: &mut dyn FnMut(Event)) -> Result<()> {
    send(Event::Line(crate::plan::Plan::describe(std::slice::from_ref(step))));
    Ok(())
}

fn one(step: &Step, log: &mut File) -> Result<()> {
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
            let _ = log.write_all(err.as_bytes());
            if !out.status.success() {
                // Nix's last "error:" with words says why (the first is often bare).
                let why = err
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| l.starts_with("error:") && l.len() > "error:".len())
                    .map(String::from)
                    .unwrap_or_else(|| last_line(&err));
                bail!("{} failed: {why}", args[0]);
            }
            std::fs::write(path, tidy(path, &out.stdout)).with_context(|| format!("can't write {path}"))
        }
        Step::Run { args, input, .. } => {
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
                let _ = writeln!(log, "{l}");
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
        let log = t.path().join("install.log");
        let r = run_logged(&steps, false, &log, &mut |e| events.push(e));
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hi");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "made\n");
        let log = std::fs::read_to_string(&log).unwrap();
        assert!(log.contains("== Read\nsecret\n") && log.contains("nope"), "the tools' output goes to the log");
        assert!(!events.iter().any(|e| matches!(e, Event::Line(_))), "and not to the screen");
        assert!(events.contains(&Event::Failed { title: "Fail".into(), why: "sh failed: nope".into() }));
        assert!(!t.path().join("never").exists());
        assert!(!events.contains(&Event::Done));
    }

    #[test]
    fn complaints_in_the_installers_words() {
        assert_eq!(plain("nixos-install failed: can't mount /mnt"), "can't mount /mnt");
        assert_eq!(
            plain("error: builder for '/nix/store/0123456789abcdfghijklmnpqrsvwxyz-nyxos-system-x-0.1.0.drv' failed"),
            "builder for 'nyxos-system-x-0.1.0.drv' failed"
        );
        assert_eq!(plain("can't start nixos-generate-config"), "can't start the installer");
    }

    #[test]
    fn the_hardware_file_says_what_it_is() {
        let generated = b"# Do not modify this file!  It was generated by 'nixos-generate-config'\n# and may be overwritten.\n{ config, ... }:\n{\n  # A comment inside stays.\n}\n";
        let file = String::from_utf8(tidy("/mnt/etc/nyxos/flake/hardware-configuration.nix", generated)).unwrap();
        assert!(file.starts_with("# This computer's hardware, as the NyxOS installer found it."));
        assert!(!file.contains("nixos-generate-config") && file.contains("{ config, ... }:\n{\n  # A comment inside stays.\n}\n"));
        assert_eq!(tidy("/mnt/x/resume-offset", b"123\n"), b"123\n");
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("x").display().to_string();
        let mut lines = vec![];
        let log = t.path().join("log");
        run_logged(&[Step::Write { path: file.clone(), contents: "x".into() }], true, &log, &mut |e| lines.push(e)).unwrap();
        assert!(!std::path::Path::new(&file).exists());
        assert!(lines.contains(&Event::Done));
    }
}

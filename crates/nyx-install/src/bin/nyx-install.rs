//! nyx-install: NyxOS's installer without a window, for automated installs and testing.
//!
//!   nyx-install disks                 The disks, as JSON
//!   nyx-install plan PLAN.json        The steps it would run (nothing is changed)
//!   nyx-install run PLAN.json --yes   Install (everything on the target goes)
//!   nyx-install remove-leftovers DISK --yes
//!                                     Take away the partitions an install that didn't
//!                                     finish left beside another system

use anyhow::{Context, Result, bail};
use nyx_install::{disks, plan::Plan, run};

fn load(path: &str) -> Result<(Plan, disks::Disk)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("can't read {path}"))?;
    let plan: Plan = serde_json::from_str(&text).context("the plan isn't one nyx-install knows")?;
    let disk = disks::list()?
        .into_iter()
        .find(|d| d.path == plan.target.disk())
        .with_context(|| format!("there's no disk {}", plan.target.disk()))?;
    nyx_install::plan::check(&plan, &disk)?;
    Ok((plan, disk))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["disks"] => println!("{}", serde_json::to_string_pretty(&disks::list()?)?),
        ["plan", path] => {
            let (plan, disk) = load(path)?;
            println!("{}", Plan::describe(&plan.steps(&disk)?));
        }
        ["run", path, "--yes"] => {
            anyhow::ensure!(unsafe { libc_geteuid() } == 0, "run it as root");
            let (plan, disk) = load(path)?;
            let steps = plan.steps(&disk)?;
            run::run(&steps, false, &mut |e| match e {
                run::Event::Step { index, total, title } => eprintln!("[{}/{total}] {title}", index + 1),
                run::Event::Line(l) => println!("  {l}"),
                run::Event::Done => eprintln!("NyxOS is installed. Restart to use it."),
                run::Event::Failed { title, why } => eprintln!("{title} didn't work: {why}\nEverything it did is in {}.", run::LOG),
            })?;
        }
        ["remove-leftovers", path, "--yes"] => {
            anyhow::ensure!(unsafe { libc_geteuid() } == 0, "run it as root");
            let disk = disks::list()?.into_iter().find(|d| d.path == *path).with_context(|| format!("there's no disk {path}"))?;
            let cmds = nyx_install::plan::leftover_removal(&disk);
            anyhow::ensure!(!cmds.is_empty(), "{path} has nothing an unfinished install left");
            // What the install left mounted or open holds its partitions.
            let _ = std::process::Command::new("umount").args(["--recursive", nyx_install::plan::ROOT]).status();
            let _ = std::process::Command::new("cryptsetup").args(["close", "cryptroot"]).status();
            for cmd in cmds {
                eprintln!("$ {}", cmd.join(" "));
                let status = std::process::Command::new(&cmd[0]).args(&cmd[1..]).status().with_context(|| format!("can't run {}", cmd[0]))?;
                anyhow::ensure!(status.success(), "{} failed", cmd[0]);
            }
        }
        _ => bail!("usage: nyx-install disks | plan PLAN.json | run PLAN.json --yes | remove-leftovers DISK --yes"),
    }
    Ok(())
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

//! nyx-install: NyxOS's installer without a window, for automated installs and testing.
//!
//!   nyx-install disks                 The disks, as JSON
//!   nyx-install plan PLAN.json        The steps it would run (nothing is changed)
//!   nyx-install run PLAN.json --yes   Install (everything on the target goes)

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
                run::Event::Failed { title, why } => eprintln!("{title} didn't work: {why}"),
            })?;
        }
        _ => bail!("usage: nyx-install disks | plan PLAN.json | run PLAN.json --yes"),
    }
    Ok(())
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

//! What to send when an install fails on a computer: the install's log and a report of the
//! hardware (graphics, input devices, the kernel's messages), saved to a USB stick from
//! the installer, so nobody needs a console to get them.

use crate::disks;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// The last `lines` lines of the install's log.
pub fn log_tail(lines: usize) -> String {
    let text = std::fs::read_to_string(crate::run::LOG).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn command(args: &[&str]) -> String {
    match Command::new(args[0]).args(&args[1..]).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Err(e) => format!("({} didn't run: {e})\n", args[0]),
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| format!("({path}: {e})\n"))
}

/// This computer's hardware, as far as it matters to an install: model, kernel, graphics,
/// screens, input devices, memory and disks, and the kernel's messages.
pub fn hardware() -> String {
    let mut s = String::new();
    let mut section = |title: &str, body: String| {
        s.push_str(&format!("== {title}\n{}\n", body.trim_end()));
    };
    section(
        "computer",
        ["sys_vendor", "product_name", "product_version", "board_name", "bios_version"]
            .iter()
            .map(|f| format!("{f}: {}", read(&format!("/sys/class/dmi/id/{f}")).trim()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    section("kernel", format!("{}{}", command(&["uname", "-a"]), read("/proc/cmdline")));
    section("graphics", command(&["lspci", "-nnk"]));
    let mut screens = String::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for e in entries.flatten() {
            let modes = e.path().join("modes");
            if let Ok(m) = std::fs::read_to_string(&modes) {
                let status = std::fs::read_to_string(e.path().join("status")).unwrap_or_default();
                screens.push_str(&format!(
                    "{} ({}): {}\n",
                    e.file_name().to_string_lossy(),
                    status.trim(),
                    m.lines().take(3).collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    section("screens", screens);
    section("input devices", read("/proc/bus/input/devices"));
    section("memory", command(&["free", "-h"]));
    section("disks", command(&["lsblk", "-o", "NAME,SIZE,TYPE,TRAN,RM,FSTYPE,MOUNTPOINTS"]));
    section("space", command(&["df", "-h"]));
    section("kernel messages", command(&["dmesg"]));
    s
}

/// A USB stick to save to: a USB disk that isn't the live system's (that one is busy) with a
/// partition in a file system Linux writes. Its partition and a name for it.
fn stick() -> Result<(String, String)> {
    let writable = ["vfat", "exfat", "ntfs", "ntfs3", "ext4", "ext3", "ext2", "btrfs", "xfs"];
    for d in disks::list()? {
        if d.transport != "usb" || d.busy {
            continue;
        }
        if let Some(p) = d.partitions.iter().find(|p| !p.mounted && writable.contains(&p.fstype.as_str())) {
            return Ok((p.path.clone(), d.name()));
        }
    }
    bail!("No USB stick to save to: plug one in (FAT, exFAT, NTFS or ext4) and try again")
}

/// Saves the install's log and the hardware report to a USB stick, in a folder named for
/// the time. Returns where they went.
pub fn save_to_usb() -> Result<String> {
    let (partition, name) = stick()?;
    let mount = Path::new("/run/nyx-installer/report-usb");
    std::fs::create_dir_all(mount)?;
    let ran = Command::new("mount").arg(&partition).arg(mount).status().context("mount didn't run")?;
    if !ran.success() {
        bail!("{name} couldn't be opened ({partition})");
    }
    let stamp = command(&["date", "+%Y-%m-%d-%H%M%S"]).trim().to_string();
    let folder = mount.join(format!("nyxos-install-{stamp}"));
    let written = (|| -> Result<()> {
        std::fs::create_dir_all(&folder)?;
        if Path::new(crate::run::LOG).exists() {
            std::fs::copy(crate::run::LOG, folder.join("nyxos-install.log"))?;
        }
        std::fs::write(folder.join("hardware.txt"), hardware())?;
        Ok(())
    })();
    let _ = Command::new("sync").status();
    let _ = Command::new("umount").arg(mount).status();
    written.with_context(|| format!("writing to {name} didn't work"))?;
    Ok(format!("Saved to {name}, in nyxos-install-{stamp}. You can unplug it."))
}

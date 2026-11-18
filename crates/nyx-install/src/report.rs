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

/// Where to save: a partition, a name for it, and whether it has to be opened through a
/// loop device (Ventoy's: its device mapper holds the partition to serve the ISO, so it
/// can't be mounted directly; new files only take free space, never the ISO's).
struct Target {
    partition: String,
    name: String,
    loop_device: bool,
}

/// A USB stick to save to: a USB disk that isn't the live system's (that one is busy) with a
/// partition in a file system Linux writes; failing that, the Ventoy stick the live system
/// started from.
fn stick() -> Result<Target> {
    let writable = ["vfat", "exfat", "ntfs", "ntfs3", "ext4", "ext3", "ext2", "btrfs", "xfs"];
    let all = disks::list()?;
    for d in &all {
        if d.transport != "usb" || d.busy {
            continue;
        }
        if let Some(p) = d.partitions.iter().find(|p| !p.mounted && writable.contains(&p.fstype.as_str())) {
            return Ok(Target { partition: p.path.clone(), name: d.name(), loop_device: false });
        }
    }
    for d in &all {
        if let Some(p) = d.partitions.iter().find(|p| p.label == "Ventoy" && writable.contains(&p.fstype.as_str())) {
            return Ok(Target { partition: p.path.clone(), name: format!("{} (Ventoy)", d.name()), loop_device: true });
        }
    }
    bail!("No USB stick to save to: plug one in (FAT, exFAT, NTFS or ext4) and try again")
}

/// Saves the install's log and the hardware report to a USB stick, in a folder named for
/// the time. Returns where they went.
pub fn save_to_usb() -> Result<String> {
    let Target { partition, name, loop_device } = stick()?;
    let mount = Path::new("/run/nyx-installer/report-usb");
    std::fs::create_dir_all(mount)?;
    let device = if loop_device {
        let out = Command::new("losetup").args(["--find", "--show", &partition]).output().context("losetup didn't run")?;
        if !out.status.success() {
            bail!("{name} couldn't be opened ({partition}): {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        partition.clone()
    };
    let detach = || {
        if loop_device {
            let _ = Command::new("losetup").args(["--detach", &device]).status();
        }
    };
    let ran = Command::new("mount").arg(&device).arg(mount).status().context("mount didn't run")?;
    if !ran.success() {
        detach();
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
    detach();
    written.with_context(|| format!("writing to {name} didn't work"))?;
    Ok(format!("Saved to {name}, in nyxos-install-{stamp}. You can unplug it."))
}

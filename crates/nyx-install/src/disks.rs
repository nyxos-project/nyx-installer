//! The disks NyxOS can be installed on: their partitions, the EFI ones, Windows, and the
//! free space between partitions. Read from `lsblk --json` and `sfdisk --json`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

pub const ESP_TYPE: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
const MS_BASIC_DATA: &str = "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7";
const MS_RESERVED: &str = "e3c9e316-0b5c-4db8-817d-f92df00215ae";
const MS_RECOVERY: &str = "de94bba4-06d1-4d40-a16a-bfd50179d6ac";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Partition {
    pub path: String,
    pub size: u64,
    pub fstype: String,
    pub label: String,
    /// The GPT partition type GUID, lowercase.
    pub parttype: String,
    /// The GPT partition's own GUID (PARTUUID), lowercase.
    pub partuuid: String,
    pub mounted: bool,
}

impl Partition {
    pub fn is_esp(&self) -> bool {
        self.parttype == ESP_TYPE
    }

    /// Windows' own: its system, reserved and recovery partitions, NTFS and BitLocker.
    pub fn is_windows(&self) -> bool {
        [MS_BASIC_DATA, MS_RESERVED, MS_RECOVERY].contains(&self.parttype.as_str()) && self.fstype != "vfat"
            || matches!(self.fstype.as_str(), "ntfs" | "BitLocker")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    /// In sectors.
    pub start: u64,
    pub sectors: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Disk {
    pub path: String,
    pub model: String,
    pub size: u64,
    pub removable: bool,
    /// `nvme`, `sata`, `usb`, …
    pub transport: String,
    /// `gpt`, `dos`, or empty for none.
    pub table: String,
    pub partitions: Vec<Partition>,
    /// The largest free space between (or after) partitions.
    pub free: Option<Gap>,
    /// Something on it is mounted (the live system's medium, a mounted partition).
    pub busy: bool,
}

impl Disk {
    pub fn windows(&self) -> bool {
        self.partitions.iter().any(Partition::is_windows)
    }

    /// The EFI partition holding Windows Boot Manager, by its partition GUID.
    pub fn windows_esp(&self) -> Option<&Partition> {
        self.windows().then(|| self.partitions.iter().find(|p| p.is_esp())).flatten()
    }

    pub fn name(&self) -> String {
        let model = self.model.trim();
        let size = human(self.size);
        if model.is_empty() { format!("{} ({size})", self.path) } else { format!("{model} ({size})") }
    }
}

pub fn human(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1000.0 { format!("{:.1} TB", gb / 1000.0) } else { format!("{gb:.0} GB") }
}

#[derive(Deserialize)]
struct Lsblk {
    blockdevices: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tran: Option<String>,
    #[serde(default)]
    rm: Option<bool>,
    #[serde(default)]
    ro: Option<bool>,
    #[serde(default)]
    pttype: Option<String>,
    #[serde(default)]
    fstype: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    parttype: Option<String>,
    #[serde(default)]
    partuuid: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Block>,
}

fn mounted(b: &Block) -> bool {
    b.mountpoints.iter().flatten().next().is_some() || b.children.iter().any(mounted)
}

/// Disks from `lsblk --json --tree --bytes -o PATH,TYPE,SIZE,MODEL,TRAN,RM,RO,PTTYPE,FSTYPE,LABEL,PARTTYPE,PARTUUID,MOUNTPOINTS`:
/// whole writable disks, not loop devices, CD-ROMs or zram.
pub fn parse_lsblk(json: &str) -> Result<Vec<Disk>> {
    let l: Lsblk = serde_json::from_str(json).context("lsblk printed something unexpected")?;
    Ok(l.blockdevices
        .iter()
        .filter(|b| b.kind == "disk" && !b.ro.unwrap_or(false) && !b.path.contains("zram"))
        .map(|b| Disk {
            path: b.path.clone(),
            model: b.model.clone().unwrap_or_default().trim().to_string(),
            size: b.size.unwrap_or(0),
            removable: b.rm.unwrap_or(false),
            transport: b.tran.clone().unwrap_or_default(),
            table: b.pttype.clone().unwrap_or_default(),
            partitions: b
                .children
                .iter()
                .filter(|c| c.kind == "part")
                .map(|c| Partition {
                    path: c.path.clone(),
                    size: c.size.unwrap_or(0),
                    fstype: c.fstype.clone().unwrap_or_default(),
                    label: c.label.clone().unwrap_or_default(),
                    parttype: c.parttype.clone().unwrap_or_default().to_lowercase(),
                    partuuid: c.partuuid.clone().unwrap_or_default().to_lowercase(),
                    mounted: mounted(c),
                })
                .collect(),
            free: None,
            busy: mounted(b),
        })
        .collect())
}

#[derive(Deserialize)]
struct Sfdisk {
    partitiontable: Table,
}

#[derive(Deserialize)]
struct Table {
    #[serde(default)]
    firstlba: u64,
    #[serde(default)]
    lastlba: u64,
    #[serde(default = "sector")]
    sectorsize: u64,
    #[serde(default)]
    partitions: Vec<Part>,
}

fn sector() -> u64 {
    512
}

#[derive(Deserialize)]
struct Part {
    start: u64,
    size: u64,
}

/// The largest free space, aligned to 1 MiB, from `sfdisk --json DISK`; at least 1 GiB to
/// count.
pub fn largest_gap(sfdisk_json: &str) -> Option<Gap> {
    let t = serde_json::from_str::<Sfdisk>(sfdisk_json).ok()?.partitiontable;
    let align = (1 << 20) / t.sectorsize;
    let mut parts: Vec<(u64, u64)> = t.partitions.iter().map(|p| (p.start, p.start + p.size)).collect();
    parts.sort();
    let mut gaps = Vec::new();
    let mut cursor = t.firstlba.max(align);
    for (start, end) in parts {
        if start > cursor {
            gaps.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if t.lastlba + 1 > cursor {
        gaps.push((cursor, t.lastlba + 1));
    }
    gaps.into_iter()
        .map(|(s, e)| {
            let s = s.div_ceil(align) * align;
            let e = e / align * align;
            (s, e.saturating_sub(s))
        })
        .filter(|(_, n)| n * t.sectorsize >= 1 << 30)
        .max_by_key(|(_, n)| *n)
        .map(|(start, sectors)| Gap { start, sectors, bytes: sectors * t.sectorsize })
}

fn output(args: &[&str]) -> Result<String> {
    let out = Command::new(args[0]).args(&args[1..]).output().with_context(|| format!("can't run {}", args[0]))?;
    anyhow::ensure!(out.status.success(), "{} failed: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// This computer's disks, with their free space.
pub fn list() -> Result<Vec<Disk>> {
    let json = output(&[
        "lsblk",
        "--json",
        "--tree",
        "--bytes",
        "-o",
        "PATH,TYPE,SIZE,MODEL,TRAN,RM,RO,PTTYPE,FSTYPE,LABEL,PARTTYPE,PARTUUID,MOUNTPOINTS",
    ])?;
    let mut disks = parse_lsblk(&json)?;
    for d in &mut disks {
        if d.table == "gpt" {
            d.free = output(&["sfdisk", "--json", &d.path]).ok().and_then(|j| largest_gap(&j));
        }
    }
    Ok(disks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LSBLK: &str = r#"{"blockdevices":[
      {"path":"/dev/nvme0n1","type":"disk","size":1024209543168,"model":"Samsung SSD 990 PRO 1TB","tran":"nvme","rm":false,"ro":false,"pttype":"gpt","fstype":null,"label":null,"parttype":null,"partuuid":null,"mountpoints":[null],"children":[
        {"path":"/dev/nvme0n1p1","type":"part","size":104857600,"fstype":"vfat","label":null,"parttype":"C12A7328-F81F-11D2-BA4B-00A0C93EC93B","partuuid":"AAAA1111-0000-0000-0000-000000000001","mountpoints":[null]},
        {"path":"/dev/nvme0n1p2","type":"part","size":16777216,"fstype":null,"label":null,"parttype":"e3c9e316-0b5c-4db8-817d-f92df00215ae","partuuid":"aaaa1111-0000-0000-0000-000000000002","mountpoints":[null]},
        {"path":"/dev/nvme0n1p3","type":"part","size":500000000000,"fstype":"BitLocker","label":null,"parttype":"ebd0a0a2-b9e5-4433-87c0-68b6b72699c7","partuuid":"aaaa1111-0000-0000-0000-000000000003","mountpoints":[null]}]},
      {"path":"/dev/sda","type":"disk","size":31914983424,"model":"SanDisk Ultra","tran":"usb","rm":true,"ro":false,"pttype":"dos","fstype":"iso9660","label":"NYXOS","mountpoints":["/iso"]},
      {"path":"/dev/zram0","type":"disk","size":8000000000,"mountpoints":["[SWAP]"]},
      {"path":"/dev/sr0","type":"rom","size":1000,"mountpoints":[null]}]}"#;

    #[test]
    fn disks_with_windows_and_the_live_usb() {
        let d = parse_lsblk(LSBLK).unwrap();
        assert_eq!(d.iter().map(|x| x.path.as_str()).collect::<Vec<_>>(), vec!["/dev/nvme0n1", "/dev/sda"]);
        assert!(d[0].windows() && !d[0].busy);
        assert_eq!(d[0].windows_esp().unwrap().partuuid, "aaaa1111-0000-0000-0000-000000000001");
        assert!(d[0].partitions[0].is_esp() && !d[0].partitions[0].is_windows());
        assert!(d[1].busy && d[1].removable);
        assert_eq!(d[0].name(), "Samsung SSD 990 PRO 1TB (1.0 TB)");
    }

    #[test]
    fn free_space_after_windows_aligned_and_at_least_a_gigabyte() {
        // 1 TB disk (2000409264 sectors); Windows uses up to about 500 GB.
        let j = r#"{"partitiontable":{"label":"gpt","firstlba":34,"lastlba":2000409230,"sectorsize":512,"partitions":[
          {"node":"/dev/nvme0n1p1","start":2048,"size":204800},
          {"node":"/dev/nvme0n1p2","start":206848,"size":32768},
          {"node":"/dev/nvme0n1p3","start":239616,"size":976562500}]}}"#;
        let g = largest_gap(j).unwrap();
        assert_eq!(g.start % 2048, 0);
        assert!(g.start >= 239616 + 976562500);
        assert!(g.bytes > 400_000_000_000 && g.bytes < 525_000_000_000);
        // A small gap doesn't count.
        let full = r#"{"partitiontable":{"firstlba":34,"lastlba":4194270,"sectorsize":512,"partitions":[{"start":2048,"size":4000000}]}}"#;
        assert_eq!(largest_gap(full), None);
        // An empty disk is all gap.
        let empty = r#"{"partitiontable":{"firstlba":34,"lastlba":4194270,"sectorsize":512}}"#;
        assert_eq!(largest_gap(empty).unwrap().start, 2048);
    }
}

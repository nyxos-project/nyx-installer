//! What to install where, and the steps that do it. Partitions are made with GUIDs chosen
//! here and used as /dev/disk/by-partuuid/…, so nothing depends on device names; beside
//! Windows nothing of Windows' is written to (NyxOS gets its own EFI partition, and Limine
//! starts Windows Boot Manager from Windows').

use crate::disks::{Disk, ESP_TYPE};
use anyhow::{Result, bail, ensure};
use nyx_system_proto::SystemSettings;
use nyx_system_proto::hardware::Hardware;
use serde::{Deserialize, Serialize};

pub const ROOT: &str = "/mnt";
/// Where the built system's store path is kept between building and installing it.
const TOPLEVEL: &str = "/tmp/nyx-install-system";
const LUKS_TYPE: &str = "ca7d7ccb-63ed-4c53-861c-1742536059cc";
const LINUX_TYPE: &str = "0fc63daf-8483-4772-8e79-3d69d8477de4";
const EFI_SIZE: u64 = 1 << 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Target {
    /// Everything on the disk goes.
    Erase { disk: String },
    /// Into free space (sectors), beside what's there.
    Alongside { disk: String, start: u64, sectors: u64 },
}

impl Target {
    pub fn disk(&self) -> &str {
        match self {
            Target::Erase { disk } | Target::Alongside { disk, .. } => disk,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub login: String,
    pub full_name: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub target: Target,
    /// The disk's passphrase; none leaves it unencrypted.
    pub passphrase: Option<String>,
    pub hostname: String,
    pub person: Person,
    pub time_zone: String,
    pub keyboard: String,
    pub locale: String,
    /// The swap file, as large as memory for hibernating.
    pub swap_gib: u32,
    /// `stable` or `testing`.
    pub channel: String,
    /// Windows Boot Manager's EFI partition (its PARTUUID), to offer it at boot.
    #[serde(default)]
    pub windows_esp: Option<String>,
    /// The NyxOS flake to install from instead of fetching the channel (the live USB's).
    #[serde(default)]
    pub nyxos_source: Option<String>,
    /// What the computer has, for drivers and power (the installer detects it).
    #[serde(default)]
    pub hardware: Hardware,
    /// More kernel parameters (a serial console, a hardware quirk).
    #[serde(default)]
    pub extra_kernel_params: Vec<String>,
    /// The GUIDs the new partitions get.
    pub efi_guid: String,
    pub root_guid: String,
}

pub fn valid_login(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase() || f == '_')
        && s.len() <= 32
        && s.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || "_-".contains(ch))
        && !["root", "nixos", "greeter", "nobody"].contains(&s)
}

pub fn valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_guid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// A new random GUID.
pub fn guid() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/uuid").unwrap_or_default().trim().to_lowercase()
}

/// A Nix string literal.
fn nix_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace("${", "\\${"))
}

/// One thing the installer does.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A command; `input` goes to its stdin (passphrases, passwords) and is never shown.
    Run {
        title: String,
        args: Vec<String>,
        input: Option<String>,
    },
    /// A command whose output becomes a file.
    RunTo {
        title: String,
        args: Vec<String>,
        path: String,
    },
    Write {
        path: String,
        contents: String,
    },
}

fn run(title: &str, args: &[&str]) -> Step {
    Step::Run { title: title.into(), args: args.iter().map(|s| s.to_string()).collect(), input: None }
}

fn run_with(title: &str, args: &[&str], input: String) -> Step {
    Step::Run { title: title.into(), args: args.iter().map(|s| s.to_string()).collect(), input: Some(input) }
}

impl Plan {
    pub fn validate(&self, disk: &Disk) -> Result<()> {
        ensure!(disk.path == self.target.disk(), "the plan is for {}, not {}", self.target.disk(), disk.path);
        ensure!(!disk.busy, "{} is in use (the live system is on it, or something is mounted)", disk.path);
        ensure!(valid_login(&self.person.login), "a login name is lowercase letters, digits, - and _, starting with a letter");
        ensure!(!self.person.full_name.contains([':', '\n']), "a name can't have : or line breaks");
        ensure!(!self.person.password.is_empty() && !self.person.password.contains(['\n', ':']), "choose a password");
        ensure!(valid_hostname(&self.hostname), "a computer's name is letters, digits and -");
        ensure!(["stable", "testing"].contains(&self.channel.as_str()), "the channel is stable or testing");
        ensure!((1..=256).contains(&self.swap_gib), "between 1 and 256 GiB of swap");
        ensure!(valid_guid(&self.efi_guid) && valid_guid(&self.root_guid), "the new partitions need GUIDs");
        if let Some(p) = &self.passphrase {
            ensure!(p.chars().count() >= 8, "the disk's passphrase needs at least 8 characters");
        }
        if let Some(w) = &self.windows_esp {
            ensure!(valid_guid(w), "Windows' EFI partition is named by its GUID");
        }
        for k in &self.extra_kernel_params {
            ensure!(
                !k.is_empty() && k.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' && c != '$'),
                "not a kernel parameter: {k}"
            );
        }
        for v in [&self.time_zone, &self.keyboard, &self.locale] {
            ensure!(
                !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || "/_-.+@".contains(c)),
                "not a valid setting: {v}"
            );
        }
        match &self.target {
            Target::Erase { .. } => ensure!(disk.size >= 32 << 30, "NyxOS needs at least 32 GB"),
            Target::Alongside { sectors, .. } => {
                ensure!(disk.table == "gpt", "installing beside another system needs a GPT disk");
                ensure!(sectors * 512 >= 32 << 30, "NyxOS needs at least 32 GB of free space")
            }
        }
        Ok(())
    }

    fn efi(&self) -> String {
        format!("/dev/disk/by-partuuid/{}", self.efi_guid)
    }

    fn root_part(&self) -> String {
        format!("/dev/disk/by-partuuid/{}", self.root_guid)
    }

    /// The filesystem's device: the opened LUKS volume, or the partition.
    fn fs(&self) -> String {
        if self.passphrase.is_some() { "/dev/mapper/cryptroot".into() } else { self.root_part() }
    }

    fn flake(&self) -> String {
        format!("{ROOT}/etc/nyxos/flake")
    }

    /// The partition numbers the new partitions get: the first two free.
    fn numbers(disk: &Disk) -> (u32, u32) {
        let used: Vec<u32> =
            disk.partitions.iter().filter_map(|p| p.path.rsplit(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).collect();
        let mut free = (1..=128).filter(|n| !used.contains(n));
        (free.next().unwrap_or(1), free.next().unwrap_or(2))
    }

    pub fn steps(&self, disk: &Disk) -> Result<Vec<Step>> {
        self.validate(disk)?;
        let d = disk.path.as_str();
        let (efi, root, fs) = (self.efi(), self.root_part(), self.fs());
        let root_type = if self.passphrase.is_some() { LUKS_TYPE } else { LINUX_TYPE };
        let mut s = vec![];
        let (ne, nr, efi_range, root_range) = match &self.target {
            Target::Erase { .. } => {
                s.push(run("Clearing the disk", &["wipefs", "--all", d]));
                s.push(run("Making a new partition table", &["sgdisk", "--zap-all", d]));
                (1, 2, "0:+1G".to_string(), "0:0".to_string())
            }
            Target::Alongside { start, sectors, .. } => {
                let efi_end = start + EFI_SIZE / 512 - 1;
                let (ne, nr) = Self::numbers(disk);
                (ne, nr, format!("{start}:{efi_end}"), format!("{}:{}", efi_end + 1, start + sectors - 1))
            }
        };
        let n = |k: u32, rest: &str| format!("{k}:{rest}");
        s.push(run(
            "Making NyxOS's partitions",
            &[
                "sgdisk",
                &format!("--new={}", n(ne, &efi_range)),
                &format!("--typecode={}", n(ne, ESP_TYPE)),
                &format!("--partition-guid={}", n(ne, &self.efi_guid)),
                &format!("--change-name={}", n(ne, "NyxOS EFI")),
                &format!("--new={}", n(nr, &root_range)),
                &format!("--typecode={}", n(nr, root_type)),
                &format!("--partition-guid={}", n(nr, &self.root_guid)),
                &format!("--change-name={}", n(nr, "NyxOS")),
                d,
            ],
        ));
        s.push(run("Waiting for the new partitions", &["udevadm", "settle", "--timeout=30"]));
        s.push(run("Waiting for the new partitions", &["partprobe", d]));
        s.push(run("Waiting for the new partitions", &["udevadm", "settle", "--timeout=30"]));
        s.push(run("Formatting the EFI partition", &["mkfs.fat", "-F", "32", "-n", "NYXOS", &efi]));
        if let Some(p) = &self.passphrase {
            s.push(run_with(
                "Encrypting the disk",
                &["cryptsetup", "luksFormat", "--type", "luks2", "--batch-mode", "--label", "NyxOS", "--key-file=-", &root],
                p.clone(),
            ));
            s.push(run_with(
                "Opening the encrypted disk",
                &["cryptsetup", "open", "--key-file=-", &root, "cryptroot"],
                p.clone(),
            ));
        }
        s.push(run("Making the Btrfs filesystem", &["mkfs.btrfs", "--force", "--label", "nyxos", &fs]));
        s.push(run("Making subvolumes", &["mount", &fs, ROOT]));
        for v in ["@", "@home", "@nix", "@log", "@swap"] {
            s.push(run("Making subvolumes", &["btrfs", "subvolume", "create", &format!("{ROOT}/{v}")]));
        }
        s.push(run("Making subvolumes", &["umount", ROOT]));
        let opts = |v: &str| format!("subvol={v},compress=zstd,noatime");
        s.push(run("Mounting", &["mount", "-o", &opts("@"), &fs, ROOT]));
        for (v, at) in [("@home", "home"), ("@nix", "nix"), ("@log", "var/log"), ("@swap", "swap")] {
            let dir = format!("{ROOT}/{at}");
            s.push(run("Mounting", &["mkdir", "-p", &dir]));
            let o = if v == "@swap" { "subvol=@swap,noatime".to_string() } else { opts(v) };
            s.push(run("Mounting", &["mount", "-o", &o, &fs, &dir]));
        }
        s.push(run("Mounting", &["mkdir", "-p", &format!("{ROOT}/boot")]));
        s.push(run("Mounting", &["mount", "-o", "umask=0077", &efi, &format!("{ROOT}/boot")]));
        s.push(run(
            "Making the swap file",
            &["btrfs", "filesystem", "mkswapfile", "--size", &format!("{}g", self.swap_gib), &format!("{ROOT}/swap/swapfile")],
        ));
        let flake = self.flake();
        s.push(run("Writing this computer's configuration", &["mkdir", "-p", &flake, &format!("{ROOT}/etc/nyxos/overrides")]));
        s.push(Step::RunTo {
            title: "Describing this computer's hardware".into(),
            args: ["nixos-generate-config", "--root", ROOT, "--show-hardware-config"].iter().map(|x| x.to_string()).collect(),
            path: format!("{flake}/hardware-configuration.nix"),
        });
        s.push(Step::RunTo {
            title: "Making the swap file".into(),
            args: ["btrfs", "inspect-internal", "map-swapfile", "-r", &format!("{ROOT}/swap/swapfile")]
                .iter()
                .map(|x| x.to_string())
                .collect(),
            path: format!("{flake}/resume-offset"),
        });
        s.push(Step::Write { path: format!("{flake}/flake.nix"), contents: self.flake_nix() });
        s.push(Step::Write { path: format!("{flake}/host.nix"), contents: self.host_nix() });
        s.push(Step::Write { path: format!("{ROOT}/etc/nyxos/overrides/system.json"), contents: self.settings_json()? });
        let mut lock = vec!["nix", "flake", "lock", &flake, "--override-input", "nyxos-overrides"];
        let overrides = format!("path:{ROOT}/etc/nyxos/overrides");
        lock.push(&overrides);
        let source = self.nyxos_source.as_ref().map(|p| format!("path:{p}"));
        if let Some(src) = &source {
            lock.extend(["--override-input", "nyxos", src]);
        }
        s.push(run("Pinning NyxOS's version", &lock));
        // Built on the live system, whose store has NyxOS already; nixos-install then only
        // copies it to the disk and sets up the boot menu.
        let attr = format!("{flake}#nixosConfigurations.{}.config.system.build.toplevel", self.hostname);
        let mut build =
            vec!["nix", "build", "--no-link", "--print-out-paths", &attr, "--override-input", "nyxos-overrides", &overrides];
        if let Some(src) = &source {
            build.extend(["--override-input", "nyxos", src]);
        }
        s.push(Step::RunTo {
            title: "Building NyxOS for this computer".into(),
            args: build.iter().map(|x| x.to_string()).collect(),
            path: TOPLEVEL.into(),
        });
        s.push(run(
            "Installing NyxOS (this takes a while)",
            &[
                "sh",
                "-c",
                &format!("nixos-install --root {ROOT} --system \"$(cat {TOPLEVEL})\" --no-root-passwd --no-channel-copy"),
            ],
        ));
        // Recovery and offline rebuilds need every source on the new system.
        let store = format!("local?root={ROOT}");
        let mut archive = vec![
            "nix",
            "flake",
            "archive",
            "--no-check-sigs",
            "--to",
            &store,
            &flake,
            "--override-input",
            "nyxos-overrides",
            &overrides,
        ];
        if let Some(src) = &source {
            archive.extend(["--override-input", "nyxos", src]);
        }
        s.push(run("Keeping NyxOS's sources for recovery", &archive));
        s.push(run_with(
            "Setting your password",
            &["nixos-enter", "--root", ROOT, "-c", "chpasswd"],
            format!("{}:{}\n", self.person.login, self.person.password),
        ));
        s.push(run(
            "Keeping the install's log",
            &["sh", "-c", &format!("mkdir -p {ROOT}/var/log && cp {} {ROOT}/var/log/nyxos-install.log", crate::run::LOG)],
        ));
        s.push(run("Finishing", &["umount", "--recursive", ROOT]));
        if self.passphrase.is_some() {
            s.push(run("Finishing", &["cryptsetup", "close", "cryptroot"]));
        }
        Ok(s)
    }

    pub fn flake_nix(&self) -> String {
        format!(
            r#"{{
  description = "This computer's NyxOS";

  inputs = {{
    # The release channel: stable, or testing for what's next.
    nyxos.url = "github:nyxos-project/nyxos/{channel}";

    # The system settings Settings, Welcome and the Store change (the Nyx System Manager
    # and NyxOS Recovery pass it to each rebuild).
    nyxos-overrides = {{
      url = "path:/etc/nyxos/overrides";
      flake = false;
    }};
    nyxos.inputs.nyxos-overrides.follows = "nyxos-overrides";
  }};

  outputs =
    {{
      self,
      nyxos,
      nyxos-overrides,
      ...
    }}:
    {{
      nixosConfigurations.{host} = nyxos.lib.mkSystem {{
        inherit self;
        overrides = nyxos-overrides;
        modules = [
          ./hardware-configuration.nix
          ./host.nix
        ];
      }};
    }};
}}
"#,
            channel = self.channel,
            host = self.hostname,
        )
    }

    pub fn host_nix(&self) -> String {
        let luks = if self.passphrase.is_some() {
            "\n  # The encrypted disk, opened before the system starts; TRIM reaches the SSD.\n  boot.initrd.systemd.enable = true;\n  boot.initrd.luks.devices.cryptroot.allowDiscards = true;\n"
        } else {
            ""
        };
        let windows = match &self.windows_esp {
            Some(g) => format!("\n    boot.windows = {};", nix_str(g)),
            None => String::new(),
        };
        format!(
            r#"# This computer's own NyxOS settings, written by the installer. Settings, Welcome and the
# Store keep their changes in /etc/nyxos/overrides instead.
{{ config, lib, ... }}:
{{
  system.stateVersion = "26.05";
  networking.hostName = {host};
  nyx.system.host = {host};
  boot.loader.efi.canTouchEfiVariables = true;
{luks}
  # Btrfs with compression; hibernating to the swap file.
  fileSystems = lib.genAttrs [ "/" "/home" "/nix" "/var/log" ] (_: {{
    options = [ "compress=zstd" "noatime" ];
  }});
  swapDevices = [ {{ device = "/swap/swapfile"; }} ];
  boot.resumeDevice = config.fileSystems."/".device;
  boot.kernelParams = [ "resume_offset=${{lib.trim (builtins.readFile ./resume-offset)}}"{extra} ];

  nyxos = {{
    user = {{
      name = {login};
      description = {name};
    }};
    snapshots.enable = true;
    # Standalone Nyx apps are each person's own (Welcome offers them).
    standaloneApps = "per-user";{windows}
  }};
}}
"#,
            host = nix_str(&self.hostname),
            extra = self.extra_kernel_params.iter().map(|k| format!(" {}", nix_str(k))).collect::<String>(),
            login = nix_str(&self.person.login),
            name = nix_str(&self.person.full_name),
        )
    }

    /// The settings layer the new system starts with: time zone, keyboard, language and
    /// the hardware.
    pub fn settings_json(&self) -> Result<String> {
        let s = SystemSettings {
            time_zone: Some(self.time_zone.clone()),
            keyboard_layout: Some(self.keyboard.clone()),
            locale: Some(self.locale.clone()),
            hardware: self.hardware.clone(),
            ..SystemSettings::default()
        };
        Ok(serde_json::to_string_pretty(&s)? + "\n")
    }

    /// The steps as the commands they run, passwords left out.
    pub fn describe(steps: &[Step]) -> String {
        steps
            .iter()
            .map(|s| match s {
                Step::Run { args, input, .. } => {
                    format!("{}{}", args.join(" "), if input.is_some() { "  < (hidden)" } else { "" })
                }
                Step::RunTo { args, path, .. } => format!("{} > {path}", args.join(" ")),
                Step::Write { path, .. } => format!("write {path}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The plan for `disk` refused with why, or fine.
pub fn check(plan: &Plan, disk: &Disk) -> Result<()> {
    if let Target::Alongside { start, sectors, .. } = &plan.target {
        let Some(gap) = &disk.free else { bail!("{} has no free space", disk.path) };
        ensure!(*start >= gap.start && start + sectors <= gap.start + gap.sectors, "that isn't the free space on {}", disk.path);
    }
    plan.validate(disk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disks::{Gap, Partition};

    fn disk(windows: bool) -> Disk {
        let part = |n: u32, t: &str, fs: &str| Partition {
            path: format!("/dev/nvme0n1p{n}"),
            size: 1 << 30,
            fstype: fs.into(),
            label: String::new(),
            parttype: t.into(),
            partuuid: format!("aaaaaaaa-0000-0000-0000-00000000000{n}"),
            mounted: false,
        };
        Disk {
            path: "/dev/nvme0n1".into(),
            model: "Test".into(),
            size: 1_000_000_000_000,
            removable: false,
            transport: "nvme".into(),
            table: "gpt".into(),
            partitions: if windows {
                vec![
                    part(1, ESP_TYPE, "vfat"),
                    part(2, "e3c9e316-0b5c-4db8-817d-f92df00215ae", ""),
                    part(3, "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "BitLocker"),
                ]
            } else {
                vec![]
            },
            free: Some(Gap { start: 1_000_000_000, sectors: 900_000_000, bytes: 900_000_000 * 512 }),
            busy: false,
        }
    }

    fn plan(target: Target) -> Plan {
        Plan {
            target,
            passphrase: Some("correct horse".into()),
            hostname: "nyx-laptop".into(),
            person: Person { login: "amy".into(), full_name: "Amy \"A\" Smith".into(), password: "hunter22".into() },
            time_zone: "America/Chicago".into(),
            keyboard: "us".into(),
            locale: "en_US.UTF-8".into(),
            swap_gib: 16,
            channel: "stable".into(),
            windows_esp: None,
            nyxos_source: None,
            hardware: Hardware::default(),
            extra_kernel_params: vec![],
            efi_guid: "11111111-2222-4333-8444-555555555555".into(),
            root_guid: "66666666-7777-4888-9999-aaaaaaaaaaaa".into(),
        }
    }

    #[test]
    fn erasing_a_disk_encrypted() {
        let p = plan(Target::Erase { disk: "/dev/nvme0n1".into() });
        let d = disk(false);
        let steps = p.steps(&d).unwrap();
        let text = Plan::describe(&steps);
        assert!(text.starts_with("wipefs --all /dev/nvme0n1\nsgdisk --zap-all /dev/nvme0n1\n"));
        assert!(text.contains("--new=1:0:+1G --typecode=1:c12a7328-f81f-11d2-ba4b-00a0c93ec93b --partition-guid=1:11111111-2222-4333-8444-555555555555"));
        assert!(text.contains("--new=2:0:0 --typecode=2:ca7d7ccb-63ed-4c53-861c-1742536059cc"));
        assert!(text.contains("cryptsetup luksFormat --type luks2 --batch-mode --label NyxOS --key-file=- /dev/disk/by-partuuid/66666666-7777-4888-9999-aaaaaaaaaaaa  < (hidden)"));
        assert!(text.contains("mkfs.btrfs --force --label nyxos /dev/mapper/cryptroot"));
        assert!(text.contains("mount -o subvol=@home,compress=zstd,noatime /dev/mapper/cryptroot /mnt/home"));
        assert!(text.contains("btrfs filesystem mkswapfile --size 16g /mnt/swap/swapfile"));
        assert!(text.contains("nix build --no-link --print-out-paths /mnt/etc/nyxos/flake#nixosConfigurations.nyx-laptop.config.system.build.toplevel"));
        assert!(text.contains(
            "nixos-install --root /mnt --system \"$(cat /tmp/nyx-install-system)\" --no-root-passwd --no-channel-copy"
        ));
        assert!(text.ends_with("umount --recursive /mnt\ncryptsetup close cryptroot"));
        assert!(!text.contains("correct horse") && !text.contains("hunter22"), "secrets never show");
        let Step::Run { input, .. } =
            steps.iter().find(|s| matches!(s, Step::Run { title, .. } if title == "Setting your password")).unwrap()
        else {
            panic!()
        };
        assert_eq!(input.as_deref(), Some("amy:hunter22\n"));
    }

    #[test]
    fn beside_windows_nothing_of_windows_is_touched() {
        let mut p = plan(Target::Alongside { disk: "/dev/nvme0n1".into(), start: 1_000_000_000, sectors: 900_000_000 });
        p.passphrase = None;
        p.windows_esp = Some("aaaaaaaa-0000-0000-0000-000000000001".into());
        let d = disk(true);
        let text = Plan::describe(&p.steps(&d).unwrap());
        assert!(!text.contains("wipefs") && !text.contains("zap"));
        assert!(text.contains("--new=4:1000000000:1002097151"));
        assert!(text.contains("--new=5:1002097152:1899999999 --typecode=5:0fc63daf-8483-4772-8e79-3d69d8477de4"));
        for w in ["nvme0n1p1", "nvme0n1p2", "nvme0n1p3"] {
            assert!(!text.contains(w), "{w} is Windows'");
        }
        assert!(text.contains("mkfs.btrfs --force --label nyxos /dev/disk/by-partuuid/66666666"));
        assert!(!text.contains("cryptsetup"));
        assert!(p.host_nix().contains("boot.windows = \"aaaaaaaa-0000-0000-0000-000000000001\";"));
        check(&p, &d).unwrap();
        let mut outside = p.clone();
        outside.target = Target::Alongside { disk: "/dev/nvme0n1".into(), start: 10, sectors: 900_000_000 };
        assert!(check(&outside, &d).is_err());
    }

    #[test]
    fn the_files_it_writes() {
        let p = plan(Target::Erase { disk: "/dev/nvme0n1".into() });
        let f = p.flake_nix();
        assert!(f.contains("nyxos.url = \"github:nyxos-project/nyxos/stable\";"));
        assert!(f.contains("nixosConfigurations.nyx-laptop = nyxos.lib.mkSystem {"));
        let h = p.host_nix();
        assert!(h.contains("description = \"Amy \\\"A\\\" Smith\";"));
        assert!(h.contains("boot.resumeDevice = config.fileSystems.\"/\".device;"));
        assert!(h.contains("boot.initrd.systemd.enable = true;"));
        assert!(h.contains("resume_offset=${lib.trim (builtins.readFile ./resume-offset)}"));
        let s: serde_json::Value = serde_json::from_str(&p.settings_json().unwrap()).unwrap();
        assert_eq!(s["timeZone"], "America/Chicago");
        assert_eq!(s["keyboardLayout"], "us");
    }

    #[test]
    fn what_is_refused() {
        let d = disk(false);
        let ok = plan(Target::Erase { disk: "/dev/nvme0n1".into() });
        ok.validate(&d).unwrap();
        let bad = |f: fn(&mut Plan)| {
            let mut p = ok.clone();
            f(&mut p);
            p.validate(&d).is_err()
        };
        assert!(bad(|p| p.person.login = "Root".into()));
        assert!(bad(|p| p.person.login = "root".into()));
        assert!(bad(|p| p.hostname = "my laptop".into()));
        assert!(bad(|p| p.passphrase = Some("short".into())));
        assert!(bad(|p| p.channel = "nightly".into()));
        assert!(bad(|p| p.time_zone = "America/Chicago; rm -rf /".into()));
        assert!(bad(|p| p.target = Target::Erase { disk: "/dev/sda".into() }));
        let mut busy = d.clone();
        busy.busy = true;
        assert!(ok.validate(&busy).is_err());
    }
}

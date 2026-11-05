//! What the person chose, the lists they choose from, and the plan that comes of it.

use nyx_install::disks::Disk;
use nyx_install::plan::{self, Person, Plan, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Erase,
    Alongside,
}

#[derive(Debug, Clone)]
pub struct Choices {
    pub locale: String,
    pub keyboard: String,
    pub time_zone: String,
    pub disk: Option<String>,
    pub mode: Mode,
    pub encrypt: bool,
    pub passphrase: String,
    pub passphrase_again: String,
    pub full_name: String,
    pub login: String,
    pub password: String,
    pub password_again: String,
    pub hostname: String,
    /// Typed by the person, so not derived any more.
    pub login_typed: bool,
    pub hostname_typed: bool,
    /// Erasing a disk was confirmed on Review.
    pub erase_confirmed: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            locale: "en_US.UTF-8".into(),
            keyboard: "us".into(),
            time_zone: current_time_zone().unwrap_or_else(|| "America/Chicago".into()),
            disk: None,
            mode: Mode::Erase,
            encrypt: true,
            passphrase: String::new(),
            passphrase_again: String::new(),
            full_name: String::new(),
            login: String::new(),
            password: String::new(),
            password_again: String::new(),
            hostname: String::new(),
            login_typed: false,
            hostname_typed: false,
            erase_confirmed: false,
        }
    }
}

pub const LOCALES: &[(&str, &str)] = &[
    ("en_US.UTF-8", "English (United States)"),
    ("en_GB.UTF-8", "English (United Kingdom)"),
    ("de_DE.UTF-8", "Deutsch"),
    ("es_ES.UTF-8", "Español"),
    ("fr_FR.UTF-8", "Français"),
    ("it_IT.UTF-8", "Italiano"),
    ("nl_NL.UTF-8", "Nederlands"),
    ("pt_BR.UTF-8", "Português (Brasil)"),
    ("sv_SE.UTF-8", "Svenska"),
    ("pl_PL.UTF-8", "Polski"),
    ("ja_JP.UTF-8", "日本語"),
    ("zh_CN.UTF-8", "中文 (简体)"),
];

pub const KEYBOARDS: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("br", "Portuguese (Brazil)"),
    ("se", "Swedish"),
    ("no", "Norwegian"),
    ("dk", "Danish"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("ch", "Swiss"),
    ("nl", "Dutch"),
    ("jp", "Japanese"),
];

fn current_time_zone() -> Option<String> {
    let link = std::fs::read_link("/etc/localtime").ok()?;
    let s = link.to_string_lossy();
    s.split("zoneinfo/").nth(1).map(String::from).filter(|z| z != "UTC" && !z.is_empty())
}

/// The time zones systemd knows (`timedatectl list-timezones`), else a few.
pub fn time_zones() -> Vec<String> {
    let out = std::process::Command::new("timedatectl").arg("list-timezones").output();
    let list: Vec<String> =
        out.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(String::from).collect()).unwrap_or_default();
    if list.is_empty() {
        [
            "UTC",
            "America/New_York",
            "America/Chicago",
            "America/Denver",
            "America/Los_Angeles",
            "Europe/London",
            "Europe/Berlin",
            "Asia/Tokyo",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        list
    }
}

/// A login from a full name: `Amy Smith` → `amy`.
pub fn login_from(full_name: &str) -> String {
    let first = full_name.split_whitespace().next().unwrap_or("");
    let l: String = first.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).take(32).collect();
    if l.chars().next().is_some_and(|c| c.is_ascii_lowercase()) { l } else { String::new() }
}

/// What the disk page needs before going on, or why it can't.
pub fn disk_problem(c: &Choices, disks: &[Disk]) -> Option<String> {
    let Some(d) = c.disk.as_ref().and_then(|p| disks.iter().find(|d| &d.path == p)) else {
        return Some("Choose where to install NyxOS.".into());
    };
    if d.busy {
        return Some(format!("{} is in use: it's the live USB, or something on it is mounted.", d.name()));
    }
    match c.mode {
        Mode::Erase if d.size < 32 << 30 => return Some("NyxOS needs a disk of at least 32 GB.".into()),
        Mode::Alongside if d.free.as_ref().is_none_or(|g| g.bytes < 32 << 30) => {
            return Some("There isn't 32 GB of free space beside what's on this disk.".into());
        }
        _ => {}
    }
    if c.encrypt {
        if c.passphrase.chars().count() < 8 {
            return Some("The disk's passphrase needs at least 8 characters.".into());
        }
        if c.passphrase != c.passphrase_again {
            return Some("The two passphrases aren't the same.".into());
        }
    }
    None
}

pub fn person_problem(c: &Choices) -> Option<String> {
    if c.full_name.trim().is_empty() {
        return Some("Type your name.".into());
    }
    if c.full_name.contains([':', '\n']) {
        return Some("A name can't have : in it.".into());
    }
    if !plan::valid_login(&c.login) {
        return Some("A login is lowercase letters, digits, - and _, starting with a letter.".into());
    }
    if c.password.is_empty() {
        return Some("Choose a password.".into());
    }
    if c.password.contains(':') {
        return Some("A password can't have : in it.".into());
    }
    if c.password != c.password_again {
        return Some("The two passwords aren't the same.".into());
    }
    if !plan::valid_hostname(&c.hostname) {
        return Some("The computer's name is letters, digits and -.".into());
    }
    None
}

/// Memory in GiB, for a swap file big enough to hibernate.
fn memory_gib() -> u32 {
    let kb: u64 = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("MemTotal:").and_then(|v| v.split_whitespace().next()?.parse().ok())))
        .unwrap_or(8 * 1024 * 1024);
    (kb.div_ceil(1024 * 1024) as u32).clamp(2, 64)
}

pub fn plan(c: &Choices, disk: &Disk) -> Plan {
    let target = match (c.mode, &disk.free) {
        (Mode::Alongside, Some(g)) => Target::Alongside { disk: disk.path.clone(), start: g.start, sectors: g.sectors },
        _ => Target::Erase { disk: disk.path.clone() },
    };
    let windows_esp = (c.mode == Mode::Alongside).then(|| disk.windows_esp().map(|p| p.partuuid.clone())).flatten();
    Plan {
        target,
        passphrase: c.encrypt.then(|| c.passphrase.clone()),
        hostname: c.hostname.clone(),
        person: Person { login: c.login.clone(), full_name: c.full_name.trim().to_string(), password: c.password.clone() },
        time_zone: c.time_zone.clone(),
        keyboard: c.keyboard.clone(),
        locale: c.locale.clone(),
        swap_gib: memory_gib(),
        channel: "stable".into(),
        windows_esp,
        nyxos_source: std::fs::read_to_string("/etc/nyxos/source").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        hardware: nyx_system_proto::hardware::detect(std::path::Path::new("/")),
        extra_kernel_params: vec![],
        efi_guid: plan::guid(),
        root_guid: plan::guid(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins_from_names() {
        assert_eq!(login_from("Amy Smith"), "amy");
        assert_eq!(login_from("  José Ñúñez"), "jos");
        assert_eq!(login_from("42 Things"), "");
        assert_eq!(login_from(""), "");
    }

    #[test]
    fn people_need_a_name_login_password_and_computer_name() {
        let mut c = Choices {
            full_name: "Amy Smith".into(),
            login: "amy".into(),
            password: "pw".into(),
            password_again: "pw".into(),
            hostname: "amy-nyxos".into(),
            ..Default::default()
        };
        assert_eq!(person_problem(&c), None);
        c.password_again = "other".into();
        assert!(person_problem(&c).unwrap().contains("aren't the same"));
        c.password_again = "pw".into();
        c.login = "Amy".into();
        assert!(person_problem(&c).is_some());
    }
}

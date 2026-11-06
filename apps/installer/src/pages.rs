//! The pages: Welcome (language, keyboard, time zone), Disk, You, Review, and the install.

use crate::choices::{self, Mode};
use crate::window::Ui;
use gtk::{glib, prelude::*};
use nyx_install::disks::human;
use nyx_install::run::Event;
use nyx_ui::widgets as w;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Welcome,
    Disk,
    You,
    Review,
}

pub struct Step {
    pub title: &'static str,
    pub page: Page,
}

pub const PAGES: &[Step] = &[
    Step { title: "Welcome", page: Page::Welcome },
    Step { title: "Where to install", page: Page::Disk },
    Step { title: "You and this computer", page: Page::You },
    Step { title: "Review and install", page: Page::Review },
];

pub fn build(ui: &Rc<Ui>, page: Page) -> gtk::Widget {
    match page {
        Page::Welcome => welcome(ui),
        Page::Disk => disk(ui),
        Page::You => you(ui),
        Page::Review => review(ui),
    }
}

/// Why Next waits on `page`, if it does.
pub fn problem(ui: &Ui, page: Page) -> Option<String> {
    let c = ui.choices.borrow();
    match page {
        Page::Welcome => None,
        Page::Disk => choices::disk_problem(&c, &ui.disks.borrow()),
        Page::You => choices::person_problem(&c),
        Page::Review => {
            let d = choices::disk_problem(&c, &ui.disks.borrow()).or_else(|| choices::person_problem(&c));
            d.or_else(|| (c.mode == Mode::Erase && !c.erase_confirmed).then(|| "Confirm that the disk is erased.".into()))
        }
    }
}

fn heading(title: &str, text: &str) -> gtk::Box {
    let b = w::vbox(6);
    b.append(&w::label(title, "installer-title"));
    if !text.is_empty() {
        let l = w::label(text, "installer-lede");
        l.set_wrap(true);
        b.append(&l);
    }
    b
}

/// A heading in the middle, for the pages that take over the window.
fn centred(title: &str, text: &str) -> gtk::Box {
    let b = w::vbox(6);
    let t = w::label(title, "installer-title");
    t.set_xalign(0.5);
    b.append(&t);
    if !text.is_empty() {
        let l = w::label(text, "installer-lede");
        l.set_wrap(true);
        l.set_xalign(0.5);
        l.set_justify(gtk::Justification::Center);
        b.append(&l);
    }
    b
}

fn group(title: &str) -> gtk::Box {
    let b = w::vbox(6);
    b.add_css_class("installer-group");
    if !title.is_empty() {
        b.append(&w::label(title, "section-title"));
    }
    b
}

fn dropdown(items: &[(String, String)], current: &str, on: impl Fn(String) + 'static) -> gtk::DropDown {
    let names: Vec<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let d = gtk::DropDown::from_strings(&names);
    d.set_enable_search(items.len() > 20);
    if let Some(i) = items.iter().position(|(id, _)| id == current) {
        d.set_selected(i as u32);
    }
    let ids: Vec<String> = items.iter().map(|(id, _)| id.clone()).collect();
    d.connect_selected_notify(move |d| {
        if let Some(id) = ids.get(d.selected() as usize) {
            on(id.clone());
        }
    });
    d
}

fn welcome(ui: &Rc<Ui>) -> gtk::Widget {
    let page = w::vbox(18);
    page.append(&heading("Install NyxOS", "A few questions first. Nothing on this computer changes until you choose Install."));
    let c = ui.choices.borrow().clone();
    let g = group("");
    let pairs = |l: &[(&str, &str)]| l.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
    let u = ui.clone();
    g.append(&w::row(
        "Language",
        "",
        Some(dropdown(&pairs(choices::LOCALES), &c.locale, move |v| u.choices.borrow_mut().locale = v).upcast_ref()),
    ));
    let u = ui.clone();
    g.append(&w::row(
        "Keyboard",
        "Also for the disk's passphrase at startup.",
        Some(dropdown(&pairs(choices::KEYBOARDS), &c.keyboard, move |v| u.choices.borrow_mut().keyboard = v).upcast_ref()),
    ));
    let zones: Vec<(String, String)> = ui.time_zones.iter().map(|z| (z.clone(), z.replace('_', " "))).collect();
    let u = ui.clone();
    g.append(&w::row(
        "Time zone",
        "",
        Some(dropdown(&zones, &c.time_zone, move |v| u.choices.borrow_mut().time_zone = v).upcast_ref()),
    ));
    page.append(&g);
    page.upcast()
}

fn disk(ui: &Rc<Ui>) -> gtk::Widget {
    let page = w::vbox(18);
    page.append(&heading("Where to install", "A disk, and whether NyxOS takes all of it or only its free space."));
    if let Some(e) = ui.disks_error.borrow().as_ref() {
        page.append(&w::banner(&format!("The disks couldn't be read: {e}"), true));
    }
    let disks = ui.disks.borrow().clone();
    let chosen = ui.choices.borrow().disk.clone();
    let g = group("Disks");
    // A group of its own even with one disk, so each shows as a choice, not a checkbox.
    let first = Some(gtk::CheckButton::new());
    for d in &disks {
        let what = if d.busy {
            "In use (the live USB, or mounted)".to_string()
        } else if d.windows() {
            format!("Windows{}", d.free.as_ref().map(|f| format!(", {} free", human(f.bytes))).unwrap_or_default())
        } else if d.partitions.is_empty() {
            "Empty".into()
        } else {
            format!(
                "{} partitions{}",
                d.partitions.len(),
                d.free.as_ref().map(|f| format!(", {} free", human(f.bytes))).unwrap_or_default()
            )
        };
        let radio = gtk::CheckButton::new();
        radio.set_group(first.as_ref());
        radio.set_active(chosen.as_deref() == Some(&d.path));
        radio.set_sensitive(!d.busy);
        let (u, path, windows, has_gap) =
            (ui.clone(), d.path.clone(), d.windows(), d.free.as_ref().is_some_and(|g| g.bytes >= 32 << 30));
        radio.connect_toggled(move |r| {
            if r.is_active() {
                let mut c = u.choices.borrow_mut();
                c.disk = Some(path.clone());
                // Beside Windows when there's room for NyxOS; never erase it by default.
                c.mode = if windows || has_gap { Mode::Alongside } else { Mode::Erase };
                c.erase_confirmed = false;
                drop(c);
                u.rebuild();
            }
        });
        let row = w::row(&d.name(), &format!("{} · {what}", d.path), Some(radio.upcast_ref()));
        // The whole row chooses the disk.
        let click = gtk::GestureClick::new();
        let r = radio.clone();
        click.connect_released(move |_, _, _, _| {
            if r.is_sensitive() {
                r.set_active(true);
            }
        });
        row.add_controller(click);
        g.append(&row);
    }
    if disks.is_empty() {
        g.append(&w::label("No disk NyxOS can be installed on was found.", "installer-lede"));
    }
    page.append(&g);

    let c = ui.choices.borrow().clone();
    if let Some(d) = c.disk.as_ref().and_then(|p| disks.iter().find(|d| &d.path == p)) {
        let g = group("How");
        let gap = d.free.as_ref().filter(|g| g.bytes >= 32 << 30);
        let erase = gtk::CheckButton::new();
        erase.set_active(c.mode == Mode::Erase);
        let beside = gtk::CheckButton::new();
        beside.set_group(Some(&erase));
        beside.set_active(c.mode == Mode::Alongside);
        beside.set_sensitive(gap.is_some());
        let u = ui.clone();
        erase.connect_toggled(move |b| {
            if b.is_active() {
                u.choices.borrow_mut().mode = Mode::Erase;
                u.update();
            }
        });
        let u = ui.clone();
        beside.connect_toggled(move |b| {
            if b.is_active() {
                u.choices.borrow_mut().mode = Mode::Alongside;
                u.update();
            }
        });
        let beside_text = match (gap, d.windows()) {
            (Some(g), true) => format!("NyxOS uses the {} free; Windows stays as it is, and the boot menu offers both.", human(g.bytes)),
            (Some(g), false) => format!("NyxOS uses the {} free; what's on the disk stays.", human(g.bytes)),
            (None, true) => "Make room first: shrink Windows' partition in Windows' Disk Management (32 GB at least), then start the installer again.".into(),
            (None, false) => "There's no free space of 32 GB or more on this disk.".into(),
        };
        // An empty disk has nothing to keep.
        if !d.partitions.is_empty() {
            g.append(&w::row(
                if d.windows() { "Beside Windows" } else { "In the free space" },
                &beside_text,
                Some(beside.upcast_ref()),
            ));
        }
        g.append(&w::row(
            "Erase the disk",
            &format!("Everything on {} goes{}.", d.name(), if d.windows() { ", Windows too" } else { "" }),
            Some(erase.upcast_ref()),
        ));
        page.append(&g);

        let g = group("Encryption");
        let u = ui.clone();
        g.append(
            &w::switch_row(
                "Encrypt NyxOS",
                "A passphrase at each start keeps your files unreadable without it.",
                c.encrypt,
                move |on| {
                    u.choices.borrow_mut().encrypt = on;
                    u.rebuild();
                },
            )
            .0,
        );
        if c.encrypt {
            for (title, again) in [("Passphrase", false), ("The same again", true)] {
                let e = gtk::PasswordEntry::new();
                e.set_show_peek_icon(true);
                e.set_text(if again { &c.passphrase_again } else { &c.passphrase });
                let u = ui.clone();
                e.connect_changed(move |e| {
                    let mut c = u.choices.borrow_mut();
                    let t = e.text().to_string();
                    if again {
                        c.passphrase_again = t;
                    } else {
                        c.passphrase = t;
                    }
                    drop(c);
                    u.update();
                });
                g.append(&w::row(
                    title,
                    if again { "" } else { "At least 8 characters. Without it the disk can't be read." },
                    Some(e.upcast_ref()),
                ));
            }
        }
        page.append(&g);
    }
    page.upcast()
}

fn entry(text: &str, on: impl Fn(String) + 'static) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(text);
    e.set_width_chars(24);
    e.connect_changed(move |e| on(e.text().to_string()));
    e
}

fn you(ui: &Rc<Ui>) -> gtk::Widget {
    let page = w::vbox(18);
    page.append(&heading("You and this computer", "Your account, which can change settings for everyone, and the computer's name."));
    let c = ui.choices.borrow().clone();
    let g = group("You");
    let login = entry(&c.login, {
        let u = ui.clone();
        move |v| {
            let mut c = u.choices.borrow_mut();
            c.login_typed = true;
            c.login = v;
            drop(c);
            u.update();
        }
    });
    let host = entry(&c.hostname, {
        let u = ui.clone();
        move |v| {
            let mut c = u.choices.borrow_mut();
            c.hostname_typed = true;
            c.hostname = v;
            drop(c);
            u.update();
        }
    });
    let (u, l2, h2) = (ui.clone(), login.clone(), host.clone());
    let name = entry(&c.full_name, move |v| {
        let mut c = u.choices.borrow_mut();
        c.full_name = v.clone();
        let (lt, ht) = (c.login_typed, c.hostname_typed);
        drop(c);
        // The login and the computer's name follow the name until they're typed.
        if !lt {
            let l = choices::login_from(&v);
            l2.set_text(&l);
            u.choices.borrow_mut().login_typed = false;
            if !ht {
                h2.set_text(&if l.is_empty() { String::new() } else { format!("{l}-nyxos") });
                u.choices.borrow_mut().hostname_typed = false;
            }
        }
        u.update();
    });
    g.append(&w::row("Your name", "", Some(name.upcast_ref())));
    g.append(&w::row("Login", "Lowercase, for your home folder.", Some(login.upcast_ref())));
    for (title, again) in [("Password", false), ("The same again", true)] {
        let e = gtk::PasswordEntry::new();
        e.set_show_peek_icon(true);
        e.set_text(if again { &c.password_again } else { &c.password });
        let u = ui.clone();
        e.connect_changed(move |e| {
            let t = e.text().to_string();
            let mut c = u.choices.borrow_mut();
            if again {
                c.password_again = t;
            } else {
                c.password = t;
            }
            drop(c);
            u.update();
        });
        g.append(&w::row(
            title,
            "",
            Some(e.upcast_ref()),
        ));
    }
    page.append(&g);
    g.append(&w::row("Computer's name", "How it shows on your network.", Some(host.upcast_ref())));
    page.append(&g);
    page.upcast()
}

fn review(ui: &Rc<Ui>) -> gtk::Widget {
    let page = w::vbox(18);
    page.append(&heading("Review and install", "Installing takes a few minutes."));
    let c = ui.choices.borrow().clone();
    let disks = ui.disks.borrow().clone();
    let Some(d) = c.disk.as_ref().and_then(|p| disks.iter().find(|d| &d.path == p)).cloned() else {
        page.append(&w::banner("Choose a disk first.", true));
        return page.upcast();
    };
    let root = unsafe { libc_geteuid() } == 0;
    if !root {
        page.append(&w::banner(
            "The installer has to run as root to change the disks. Start it from the live USB's desktop.",
            true,
        ));
    }
    let g = group("");
    let place = match c.mode {
        Mode::Erase => format!("{} is erased, and NyxOS takes all of it.", d.name()),
        Mode::Alongside => format!(
            "NyxOS goes into the {} free on {}; {} stays as it is.",
            d.free.as_ref().map(|g| human(g.bytes)).unwrap_or_default(),
            d.name(),
            if d.windows() { "Windows" } else { "everything already there" }
        ),
    };
    let lang = choices::LOCALES.iter().find(|(id, _)| *id == c.locale).map_or(c.locale.as_str(), |(_, n)| n);
    let kb = choices::KEYBOARDS.iter().find(|(id, _)| *id == c.keyboard).map_or(c.keyboard.as_str(), |(_, n)| n);
    for (t, v) in [
        ("Where", place),
        ("Encryption", if c.encrypt { "On, with a passphrase at each start".to_string() } else { "Off".to_string() }),
        ("You", format!("{} ({})", c.full_name.trim(), c.login)),
        ("Computer", c.hostname.clone()),
        ("Language, keyboard, time zone", format!("{lang}, {kb}, {}", c.time_zone.replace('_', " "))),
    ] {
        let r = w::row(t, &v, None);
        g.append(&r);
    }
    page.append(&g);
    if c.mode == Mode::Erase {
        let g = group("");
        let what: Vec<String> = d
            .partitions
            .iter()
            .map(|p| format!("{} {}", if p.label.is_empty() { &p.fstype } else { &p.label }, human(p.size)))
            .collect();
        page.append(&w::banner(
            &if what.is_empty() {
                format!("Everything on {} is erased.", d.name())
            } else {
                format!("Everything on {} is erased: {}.", d.name(), what.join(", "))
            },
            true,
        ));
        let u = ui.clone();
        g.append(
            &w::switch_row("I understand", &format!("Erase {} and install NyxOS on it.", d.path), c.erase_confirmed, move |on| {
                u.choices.borrow_mut().erase_confirmed = on;
                u.rebuild();
            })
            .0,
        );
        page.append(&g);
    }
    let install = gtk::Button::with_label("Install NyxOS");
    install.add_css_class(if c.mode == Mode::Erase { "destructive-action" } else { "suggested-action" });
    install.set_halign(gtk::Align::End);
    install.set_sensitive(root && problem(ui, Page::Review).is_none());
    let u = ui.clone();
    install.connect_clicked(move |_| start(&u));
    page.append(&install);
    page.upcast()
}

unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

/// Install: the plan's steps on a thread, their progress here.
fn start(ui: &Rc<Ui>) {
    let c = ui.choices.borrow().clone();
    let disks = ui.disks.borrow().clone();
    let Some(disk) = c.disk.as_ref().and_then(|p| disks.iter().find(|d| &d.path == p)).cloned() else { return };
    let plan = choices::plan(&c, &disk);
    let steps = match nyx_install::plan::check(&plan, &disk).and_then(|_| plan.steps(&disk)) {
        Ok(s) => s,
        Err(e) => {
            ui.take_over(&failed(ui, &format!("{e:#}"), ""));
            return;
        }
    };
    ui.busy.set(true);
    let page = w::vbox(16);
    page.append(&centred("Installing NyxOS", "A few minutes. Keep the computer plugged in."));
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(false);
    bar.add_css_class("installer-bar");
    page.append(&bar);
    let now = w::label("Starting…", "installer-now");
    now.set_xalign(0.5);
    page.append(&now);
    let log = gtk::TextView::new();
    log.set_editable(false);
    log.set_monospace(true);
    log.add_css_class("installer-log");
    let scroll = gtk::ScrolledWindow::builder().child(&log).min_content_height(260).vexpand(true).build();
    let details = gtk::Expander::new(Some("Details"));
    details.set_child(Some(&scroll));
    page.append(&details);
    ui.take_over(&page);

    let (tx, rx) = async_channel::unbounded::<Event>();
    std::thread::spawn(move || {
        let _ = nyx_install::run::run(&steps, false, &mut |e| {
            let _ = tx.send_blocking(e);
        });
    });
    let (u, buffer) = (ui.clone(), log.buffer());
    glib::spawn_future_local(async move {
        let mut text = String::new();
        while let Ok(e) = rx.recv().await {
            match e {
                Event::Step { index, total, title } => {
                    bar.set_fraction(index as f64 / total as f64);
                    now.set_text(&title);
                }
                Event::Line(l) => {
                    text.push_str(&l);
                    text.push('\n');
                    // The last part is enough to see what's happening.
                    if text.len() > 200_000 {
                        text.drain(..100_000);
                    }
                    buffer.set_text(&text);
                    let mut end = buffer.end_iter();
                    log.scroll_to_iter(&mut end, 0.0, false, 0.0, 1.0);
                }
                Event::Done => {
                    u.busy.set(false);
                    u.take_over(&done());
                    break;
                }
                Event::Failed { title, why } => {
                    u.busy.set(false);
                    u.take_over(&failed(&u, &format!("{title} didn't work: {why}"), &text));
                    break;
                }
            }
        }
    });
}

fn done() -> gtk::Widget {
    let page = w::vbox(20);
    page.append(&centred("NyxOS is installed", "Take out the USB stick and restart. Welcome sets up the rest when you sign in."));
    let restart = gtk::Button::with_label("Restart");
    restart.add_css_class("suggested-action");
    restart.add_css_class("installer-big");
    restart.set_halign(gtk::Align::Center);
    restart.connect_clicked(|_| {
        let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
    });
    page.append(&restart);
    page.upcast()
}

fn failed(ui: &Rc<Ui>, why: &str, log: &str) -> gtk::Widget {
    let page = w::vbox(16);
    page.append(&heading("The install stopped", why));
    if !log.is_empty() {
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_monospace(true);
        view.buffer().set_text(log);
        let scroll = gtk::ScrolledWindow::builder().child(&view).min_content_height(260).vexpand(true).build();
        page.append(&scroll);
    }
    let again = gtk::Button::with_label("Back to the review");
    again.set_halign(gtk::Align::Start);
    let u = ui.clone();
    again.connect_clicked(move |_| {
        // Whatever the install left mounted or open goes first.
        let _ = std::process::Command::new("umount").args(["--recursive", nyx_install::plan::ROOT]).status();
        let _ = std::process::Command::new("cryptsetup").args(["close", "cryptroot"]).status();
        u.load_disks();
        u.show(PAGES.len() - 1);
    });
    page.append(&again);
    page.upcast()
}

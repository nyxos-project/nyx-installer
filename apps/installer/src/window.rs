//! The window: a dot per step at the top, the page in a narrow column in the middle, Back
//! and Next under it. Next waits until the page has what it needs; while installing,
//! nothing can be changed.

use crate::choices::Choices;
use crate::pages::{self, PAGES, Page};
use gtk::{glib, prelude::*};
use nyx_install::disks::Disk;
use nyx_ui::{chrome, widgets as w};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub choices: RefCell<Choices>,
    pub disks: RefCell<Vec<Disk>>,
    pub disks_error: RefCell<Option<String>>,
    pub time_zones: Vec<String>,
    dots: gtk::Box,
    page: gtk::Box,
    scroller: gtk::ScrolledWindow,
    current: Cell<usize>,
    footer: gtk::Box,
    back: gtk::Button,
    next: gtk::Button,
    problem: gtk::Label,
    pub busy: Cell<bool>,
}

impl Ui {
    pub fn page(&self) -> Page {
        PAGES[self.current.get()].page
    }

    pub fn show(self: &Rc<Self>, i: usize) {
        if self.busy.get() {
            return;
        }
        let i = i.min(PAGES.len() - 1);
        self.current.set(i);
        let p = &PAGES[i];
        chrome::set_title(&self.window, "Install NyxOS", p.title);
        let mut dot = self.dots.first_child();
        let mut n = 0;
        while let Some(d) = dot {
            if n == i {
                d.add_css_class("current");
            } else {
                d.remove_css_class("current");
            }
            if n < i {
                d.add_css_class("done");
            } else {
                d.remove_css_class("done");
            }
            n += 1;
            dot = d.next_sibling();
        }
        self.dots.set_visible(true);
        self.back.set_visible(i > 0);
        self.next.set_visible(p.page != Page::Review);
        self.next.set_label(if p.page == Page::Welcome { "Get started" } else { "Next" });
        self.footer.set_visible(true);
        self.rebuild();
    }

    pub fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.page.first_child() {
            self.page.remove(&c);
        }
        self.page.append(&pages::build(self, self.page()));
        self.scroller.vadjustment().set_value(0.0);
        self.update();
        // The keyboard starts on the page's first control.
        let (page, window) = (self.page.clone(), self.window.clone());
        glib::idle_add_local_once(move || {
            // From nothing, so the first control, not the one after what had focus.
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            page.child_focus(gtk::DirectionType::TabForward);
        });
    }

    /// Next, when the page has what it needs.
    pub fn next(self: &Rc<Self>) {
        if !self.busy.get() && self.page() != Page::Review && pages::problem(self, self.page()).is_none() {
            self.show(self.current.get() + 1);
        }
    }

    /// Next and the reason it waits, after a change on the page.
    pub fn update(&self) {
        let problem = pages::problem(self, self.page());
        self.next.set_sensitive(problem.is_none());
        self.problem.set_text(problem.as_deref().unwrap_or(""));
    }

    /// The page takes over the window (installing, done) without the footer or the steps.
    pub fn take_over(&self, widget: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.page.first_child() {
            self.page.remove(&c);
        }
        self.page.append(widget);
        self.footer.set_visible(false);
        self.dots.set_visible(false);
    }

    pub fn load_disks(self: &Rc<Self>) {
        match nyx_install::disks::list() {
            Ok(d) => {
                *self.disks.borrow_mut() = d;
                *self.disks_error.borrow_mut() = None;
            }
            Err(e) => *self.disks_error.borrow_mut() = Some(format!("{e:#}")),
        }
    }
}

pub fn build(app: &gtk::Application) -> Rc<Ui> {
    let window =
        gtk::ApplicationWindow::builder().application(app).title("Install NyxOS").default_width(1000).default_height(700).build();
    window.add_css_class("installer-window");
    window.set_size_request(760, 560);
    let outer = w::vbox(0);
    outer.add_css_class("window-frame");
    window.set_child(Some(&outer));

    let dots = w::hbox(8);
    dots.add_css_class("installer-dots");
    dots.set_halign(gtk::Align::Center);
    for p in PAGES {
        let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        d.add_css_class("installer-dot");
        d.set_tooltip_text(Some(p.title));
        dots.append(&d);
    }
    outer.append(&dots);

    let page = w::vbox(0);
    page.add_css_class("installer-page");
    page.set_valign(gtk::Align::Center);
    let clamp = nyx_ui::clamp::Clamp::new(&page, 600);
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
    outer.append(&scroller);

    let footer = w::hbox(12);
    footer.add_css_class("installer-footer");
    let back = gtk::Button::with_label("Back");
    let problem = w::label("", "installer-problem");
    problem.set_hexpand(true);
    problem.set_xalign(1.0);
    problem.set_wrap(true);
    let next = gtk::Button::with_label("Next");
    next.add_css_class("suggested-action");
    footer.append(&back);
    footer.append(&problem);
    footer.append(&next);
    let footer_clamp = nyx_ui::clamp::Clamp::new(&footer, 600);
    outer.append(&footer_clamp);

    let ui = Rc::new(Ui {
        window: window.clone(),
        choices: RefCell::new(Choices::default()),
        disks: RefCell::default(),
        disks_error: RefCell::default(),
        time_zones: crate::choices::time_zones(),
        dots,
        page,
        scroller,
        current: Cell::new(0),
        footer,
        back: back.clone(),
        next: next.clone(),
        problem,
        busy: Cell::new(false),
    });
    ui.load_disks();

    let u = ui.clone();
    back.connect_clicked(move |_| u.show(u.current.get().saturating_sub(1)));
    let u = ui.clone();
    next.connect_clicked(move |_| u.next());
    nyx_ui::keys::install(&ui.window, nyx_ui::keys::Common { close: true, ..Default::default() });
    // Alt+Right or Ctrl+Enter: Next; Alt+Left: Back.
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, k, _, mods| {
        let alt = mods.contains(gtk::gdk::ModifierType::ALT_MASK);
        let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        match k {
            gtk::gdk::Key::Right if alt => u.next(),
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter if ctrl => u.next(),
            gtk::gdk::Key::Left if alt && !u.busy.get() => u.show(u.current.get().saturating_sub(1)),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    ui.window.add_controller(keys);
    let u = ui.clone();
    window.connect_close_request(move |_| if u.busy.get() { glib::Propagation::Stop } else { glib::Propagation::Proceed });
    ui.show(0);
    ui
}

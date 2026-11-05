//! The window: the steps down the side, the page in the middle, Back and Next under it.
//! Next waits until the page has what it needs; while installing, nothing can be changed.

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
    header: chrome::AppHeader,
    list: gtk::ListBox,
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
        if let Some(row) = self.list.row_at_index(i as i32) {
            self.list.select_row(Some(&row));
        }
        let p = &PAGES[i];
        self.header.set_subtitle(if p.page == Page::Welcome { "" } else { p.title });
        chrome::set_title(&self.window, "Install NyxOS", p.title);
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
        let page = self.page.clone();
        glib::idle_add_local_once(move || {
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
        // Reaching a step needs the ones before it.
        let reachable =
            (0..PAGES.len()).take_while(|&i| i == 0 || pages::problem(self, PAGES[i - 1].page).is_none()).last().unwrap_or(0);
        let mut i = 0;
        while let Some(row) = self.list.row_at_index(i) {
            row.set_sensitive(!self.busy.get() && i as usize <= reachable.max(self.current.get()));
            i += 1;
        }
    }

    /// The page takes over the window (installing, done) without the footer or the steps.
    pub fn take_over(&self, widget: &impl IsA<gtk::Widget>) {
        while let Some(c) = self.page.first_child() {
            self.page.remove(&c);
        }
        self.page.append(widget);
        self.footer.set_visible(false);
        self.list.set_sensitive(false);
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
    let header = chrome::app_header(&window, crate::APP_ID, "Install NyxOS");
    outer.append(&header.root);

    let body = w::hbox(0);
    body.set_vexpand(true);
    outer.append(&body);

    let list = gtk::ListBox::new();
    list.add_css_class("installer-steps");
    for (i, p) in PAGES.iter().enumerate() {
        let row = w::hbox(10);
        row.add_css_class("installer-step");
        let num = w::label(&format!("{}", i + 1), "installer-step-number");
        num.set_width_chars(2);
        row.append(&num);
        row.append(&gtk::Image::from_icon_name(p.icon));
        let title = w::label(p.title, "");
        title.set_hexpand(true);
        row.append(&title);
        list.append(&row);
    }
    let side = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&list).build();
    side.add_css_class("installer-side");
    side.set_size_request(230, -1);
    body.append(&side);

    let main = w::vbox(0);
    main.set_hexpand(true);
    let page = w::vbox(0);
    page.add_css_class("installer-page");
    let clamp = nyx_ui::clamp::Clamp::new(&page, 760);
    let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
    main.append(&scroller);

    let footer = w::hbox(8);
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
    main.append(&footer);
    body.append(&main);

    let ui = Rc::new(Ui {
        window: window.clone(),
        choices: RefCell::new(Choices::default()),
        disks: RefCell::default(),
        disks_error: RefCell::default(),
        time_zones: crate::choices::time_zones(),
        header,
        list: list.clone(),
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
    list.connect_row_activated(move |_, row| u.show(row.index() as usize));
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

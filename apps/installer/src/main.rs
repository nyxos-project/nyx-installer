//! Nyx Installer: installs NyxOS from the live USB. Language, keyboard and time zone; the
//! disk (erase it, or beside Windows in free space) and its encryption; the person and
//! the computer's name; a review; then the install, with its progress.

mod choices;
mod pages;
mod window;

use gtk::prelude::*;

pub const APP_ID: &str = "io.github.design_nexus.nyx.Installer";

fn prefs() -> nyx_theme::ThemePrefs {
    nyx_theme::ThemePrefs { mode: nyx_theme::ThemeMode::Theme, theme: "nyx-dark".into(), glow: false, reduce_motion: false }
}

fn main() -> gtk::glib::ExitCode {
    nyx_core::app::init(nyx_core::app::App { slug: "nyx-installer", name: "Install NyxOS" });
    if std::env::args().any(|a| a == "--help") {
        println!(
            "Usage: nyx-installer\n\nInstalls NyxOS on this computer (as root, from the live USB). nyx-install does the same without a window."
        );
        return gtk::glib::ExitCode::SUCCESS;
    }
    let id = std::env::var("NYX_INSTALLER_APP_ID").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| APP_ID.into());
    let app = gtk::Application::builder().application_id(id).build();
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        nyx_theme::use_design_2();
        nyx_theme::install(nyx_theme::Style {
            window_class: "installer-window",
            base_skip: &[],
            css: include_str!("style.css"),
            no_glow_css: "",
            extra_colors: None,
            prefs,
            set_theme: Some(|_, _| {}),
        });
        let ui = window::build(app);
        ui.window.present();
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
#[test]
fn style_uses_only_defined_colours() {
    for theme in nyx_theme::all() {
        let sheet = nyx_theme::sheet_for_check("installer-window", &theme, true) + include_str!("style.css");
        assert_eq!(nyx_theme::undefined_colors(&sheet), Vec::<String>::new(), "{}", theme.id);
    }
}

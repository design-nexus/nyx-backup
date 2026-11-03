mod activity;
mod backups;
mod destinations;
mod ecosystem_ui;
mod forms;
mod migration_ui;
mod recovery_ui;
mod restore_ui;
mod settings;
mod status;
mod ui;
#[cfg(feature = "ui-review")]
mod ui_review;
mod window;
use backup_core::*;
use gtk::prelude::*;

pub fn save_theme(p: &nyx_theme::ThemePrefs) {
    let data = serde_json::json!({"mode":p.mode,"theme":p.theme,"glow":false,"reduce_motion":p.reduce_motion});
    let _ = atomic(
        &Store::default().config.join("appearance.json"),
        data.to_string(),
    );
}

pub fn prefs() -> nyx_theme::ThemePrefs {
    let v: serde_json::Value = std::fs::read(Store::default().config.join("appearance.json"))
        .ok()
        .and_then(|s| serde_json::from_slice(&s).ok())
        .unwrap_or_default();
    nyx_theme::ThemePrefs {
        mode: if v["mode"] == "theme" {
            nyx_theme::ThemeMode::Theme
        } else {
            nyx_theme::ThemeMode::Omarchy
        },
        theme: v["theme"].as_str().unwrap_or("nyx-dark").into(),
        glow: false,
        reduce_motion: v["reduce_motion"].as_bool().unwrap_or(false),
    }
}

fn build(app: &gtk::Application) {
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    nyx_theme::use_design_2();
    nyx_theme::install(nyx_theme::Style {
        window_class: "backup-window",
        base_skip: &[],
        css: include_str!("style.css"),
        no_glow_css: "",
        extra_colors: None,
        prefs,
        set_theme: Some(|mode, theme| {
            let mut p = prefs();
            p.mode = mode;
            p.theme = theme;
            save_theme(&p);
        }),
    });
    let ui = window::build(app);
    #[cfg(feature = "ui-review")]
    ui_review::run(&ui);
    // Keep the window's state alive for as long as the window.
    let keep = ui.clone();
    ui.window.connect_close_request(move |_| {
        let _ = &keep;
        gtk::glib::Propagation::Proceed
    });
    ui.window.present();
}

fn main() {
    backup_core::init();
    // NYX_BACKUP_APP_ID runs a test copy under its own ID, beside the real one.
    let id = std::env::var("NYX_BACKUP_APP_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| APP_ID.to_string());
    let app = gtk::Application::builder().application_id(id).build();
    app.connect_activate(build);
    app.run();
}

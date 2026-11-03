//! The Settings card: Destinations, Backups and Integrations, then the
//! standard Appearance, Keyboard and About groups.
use crate::ui::*;
use backup_core::*;
use gtk::{glib, prelude::*};
use nyx_ui::{settings_card, widgets as w};
use std::rc::Rc;

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "1"], "Backups"),
    (&["Ctrl", "2"], "Restore"),
    (&["Ctrl", "3"], "Activity"),
    (&["Ctrl", "N"], "New backup"),
    (&["Ctrl", "Enter"], "Back up now"),
    (&["Ctrl", "R"], "Refresh"),
    (&["Ctrl", "K"], "Command palette"),
    (&["Ctrl", ","], "Settings"),
    (&["F1"], "Keyboard shortcuts"),
    (&["Esc"], "Close the open card"),
    (&["Ctrl", "Q"], "Close the window; running jobs continue"),
];

const GROUPS: &[(&str, &str, &str)] = &[
    (
        "Destinations",
        "drive-harddisk-symbolic",
        "Where backups are stored",
    ),
    ("Backups", "document-save-symbolic", "Notifications"),
    (
        "Integrations",
        "network-server-symbolic",
        "App access and GitHub sync",
    ),
];

pub struct Page {
    pub root: gtk::ScrolledWindow,
    pub body: gtk::Box,
    pub destinations: gtk::Box,
    pub integrations: gtk::Box,
    pub notifications: gtk::Switch,
}

fn group(body: &gtk::Box, name: &str, note: &str) -> gtk::Box {
    let wrapper = w::vbox(0);
    wrapper.add_css_class("settings-group");
    wrapper.set_widget_name(name);
    let heading = gtk::Label::new(Some(&name.to_uppercase()));
    heading.add_css_class("group-title");
    heading.set_xalign(0.0);
    wrapper.append(&heading);
    if !note.is_empty() {
        let n = label(note, "group-note");
        n.set_margin_bottom(10);
        wrapper.append(&n);
    }
    let list = w::vbox(0);
    list.add_css_class("group-list");
    wrapper.append(&list);
    body.append(&wrapper);
    list
}

impl Page {
    pub fn new() -> Page {
        let body = w::vbox(12);
        body.add_css_class("settings-page");
        let destinations = group(
            &body,
            "Destinations",
            "Each destination holds one encrypted Restic repository. Its password stays in your keyring.",
        );
        let backups = group(&body, "Backups", "");
        let (row, notifications) = w::switch_row(
            "Notifications",
            "Say when a backup finishes or needs attention. Notifications never name files or destinations.",
            Store::default().load().unwrap_or_default().notifications,
            |on| {
                let store = Store::default();
                if let Ok(mut c) = store.load() {
                    if c.notifications != on {
                        c.notifications = on;
                        let _ = store.save(&c);
                    }
                }
            },
        );
        backups.append(&row);
        let integrations = group(
            &body,
            "Integrations",
            "Grants let a running Nyx app see a backup's status and start it for an hour. They never allow restoring, reading credentials or deleting.",
        );
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&body)
            .vexpand(true)
            .build();
        Page {
            root,
            body,
            destinations,
            integrations,
            notifications,
        }
    }
}

pub fn standard() -> settings_card::Standard {
    settings_card::Standard {
        app: "backup",
        about: settings_card::About {
            name: "Backup",
            icon: APP_ID,
            version: env!("CARGO_PKG_VERSION"),
            description: "Encrypted backups of your files and settings",
            licence: "MIT",
            website: "https://github.com/design-nexus/nyx-backup",
            issues: "https://github.com/design-nexus/nyx-backup/issues",
            credits: &[
                ("Restic", "encrypted snapshots"),
                ("Rclone", "SMB and S3 transport"),
            ],
        },
        shortcuts: SHORTCUTS,
        reduce_motion: || nyx_theme::app_prefs().reduce_motion,
        set_reduce_motion: |on| {
            let mut p = crate::prefs();
            p.reduce_motion = on;
            crate::save_theme(&p);
            nyx_theme::apply();
        },
    }
}

pub fn render(ui: &Rc<Ui>) {
    crate::destinations::render(ui);
    crate::ecosystem_ui::render(ui);
    ui.settings
        .notifications
        .set_active(Store::default().load().unwrap_or_default().notifications);
}

/// Open the card, on `group` if given. The card sizes itself to the window
/// only while it's open.
pub fn open(ui: &Rc<Ui>, group: Option<&str>) {
    render(ui);
    let page = ui.settings.root.clone();
    let body = ui.settings.body.clone();
    settings_card::open(
        &ui.overlay,
        &ui.window,
        settings_card::Spec {
            groups: GROUPS,
            search_text: None,
            standard: Some(standard()),
        },
        move || (page.upcast(), body),
    );
    if let Some(g) = group {
        settings_card::select(g);
    }
    ui.window.add_tick_callback(|window, _| {
        if !settings_card::is_open() {
            return glib::ControlFlow::Break;
        }
        settings_card::fit(window);
        glib::ControlFlow::Continue
    });
}

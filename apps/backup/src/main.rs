use backup_core::*;
use gtk::{gio, glib, prelude::*};
use nyx_ui::{chrome, settings_card, widgets as w};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};
const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "Q"], "Close window; background jobs continue"),
    (&["Ctrl", ","], "Settings"),
    (&["Ctrl", "B"], "Collapse sidebar"),
    (&["F1"], "Keyboard shortcuts"),
    (&["Ctrl", "Shift", "P"], "Command palette"),
];
fn standard() -> settings_card::Standard {
    settings_card::Standard {
        app: "backup",
        about: settings_card::About {
            name: "Nyx Backup",
            icon: APP_ID,
            version: env!("CARGO_PKG_VERSION"),
            description: "Encrypted backups of selected user data",
            licence: "MIT",
            website: "https://github.com/design-nexus/nyx-backup",
            issues: "https://github.com/design-nexus/nyx-backup/issues",
            credits: &[
                ("Restic", "encrypted snapshots"),
                ("Rclone", "SMB transport"),
            ],
        },
        shortcuts: SHORTCUTS,
        reduce_motion: || nyx_theme::app_prefs().reduce_motion,
        set_reduce_motion: |on| {
            let mut p = nyx_theme::app_prefs();
            p.reduce_motion = on;
            save_theme(&p);
        },
    }
}
fn save_theme(p: &nyx_theme::ThemePrefs) {
    let data = serde_json::json!({"mode":p.mode,"theme":p.theme,"glow":false,"reduce_motion":p.reduce_motion});
    let _ = atomic(
        &Store::default().config.join("appearance.json"),
        data.to_string(),
    );
}
fn prefs() -> nyx_theme::ThemePrefs {
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
struct Ui {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    stack: gtk::Stack,
    profiles: gtk::Box,
    destinations: gtk::Box,
    health: gtk::Label,
    last: gtk::Label,
    next: gtk::Label,
    storage: gtk::Label,
    activity: gtk::Label,
    progress: gtk::ProgressBar,
    notice: gtk::Label,
    job: RefCell<Option<String>>,
    busy: Cell<bool>,
}
fn readable_time(value: u64) -> String {
    glib::DateTime::from_unix_local(value as i64)
        .ok()
        .and_then(|d| d.format("%b %e, %Y %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Unknown time".into())
}
fn text(s: &str, class: &str) -> gtk::Label {
    let l = w::label(s, class);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
}
fn page(title: &str, description: &str) -> (gtk::ScrolledWindow, gtk::Box) {
    let body = w::vbox(18);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.set_margin_start(28);
    body.set_margin_end(28);
    body.append(&text(title, "page-title"));
    body.append(&text(description, "dim-label"));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&body)
        .vexpand(true)
        .hexpand(true)
        .build();
    (scroll, body)
}
fn card(title: &str, description: &str) -> gtk::Box {
    let b = w::vbox(8);
    b.add_css_class("card");
    b.set_margin_bottom(2);
    b.append(&text(title, "heading"));
    b.append(&text(description, "dim-label"));
    b
}
fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c)
    }
}
fn action(icon: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let b = w::bar_button(icon, tip);
    b.connect_clicked(move |_| run());
    b
}
fn dialog(parent: &gtk::ApplicationWindow, title: &str) -> gtk::Dialog {
    let d = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(620)
        .default_height(440)
        .build();
    d.add_css_class("backup-window");
    let keys = gtk::EventControllerKey::new();
    let dd = d.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            dd.response(gtk::ResponseType::Cancel);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    d.add_controller(keys);
    d
}
fn message(ui: &Rc<Ui>, title: &str, body: &str) {
    let d = dialog(&ui.window, title);
    d.add_button("Close", gtk::ResponseType::Close);
    let l = text(body, "");
    l.set_selectable(true);
    l.set_margin_start(20);
    l.set_margin_end(20);
    l.set_margin_top(16);
    l.set_margin_bottom(16);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&l)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    d.content_area().append(&scroll);
    d.connect_response(|d, _| d.close());
    d.present();
}
fn work(
    ui: &Rc<Ui>,
    label: &str,
    job: impl FnOnce() -> anyhow::Result<String> + Send + 'static,
    done: impl FnOnce(&Rc<Ui>, String) + 'static,
) {
    if ui.busy.replace(true) {
        ui.notice
            .set_text("Wait for the current request to finish.");
        return;
    }
    ui.notice.set_text(label);
    let ui = ui.clone();
    nyx_core::cmd::background(job, move |result| {
        ui.busy.set(false);
        match result {
            Ok(s) => {
                ui.notice.set_text("Ready");
                done(&ui, s);
            }
            Err(e) => {
                ui.notice.set_text(&e.to_string());
                message(&ui, "Action needs attention", &e.to_string());
            }
        }
    });
}
fn outcome(j: &serde_json::Value) -> String {
    let operation = j["operation"].as_str().unwrap_or("Operation");
    let state = j["state"].as_str().unwrap_or("");
    let mut s = format!(
        "{operation}: {state}\n{}",
        j["message"].as_str().unwrap_or("")
    );
    let result = &j["result"];
    if operation == "history" {
        if let Some(items) = result.as_array() {
            for item in items {
                s.push_str(&format!(
                    "\n\n{}  {}\n{}",
                    item["time"].as_str().unwrap_or(""),
                    item["short_id"]
                        .as_str()
                        .or(item["id"].as_str())
                        .unwrap_or(""),
                    item["paths"]
                        .as_array()
                        .map(|a| a
                            .iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"))
                        .unwrap_or_default()
                ));
            }
            if items.is_empty() {
                s.push_str("\nNo snapshots for this profile.");
            }
        }
    } else if !result.is_null() {
        s.push_str(&format!(
            "\n\n{}",
            serde_json::to_string_pretty(result).unwrap_or_default()
        ));
    }
    s
}
fn run_profile(ui: &Rc<Ui>, id: String, op: &str) {
    let op = op.to_string();
    work(
        ui,
        "Starting background job…",
        move || Ok(backup_service::start(&id, &op)?),
        |ui, id| {
            *ui.job.borrow_mut() = Some(id);
            ui.stack.set_visible_child_name("overview");
            ui.notice
                .set_text("Background job started. Closing this window leaves it running.");
        },
    );
}
fn local_operation(ui: &Rc<Ui>, id: String, operation: &str) {
    let op = operation.to_string();
    work(
        ui,
        "Working with repository…",
        move || {
            let store = Store::default();
            let c = store.load()?;
            let p = c.profile(&id)?;
            let j = backup_engine::run(&store, &c, p, &op)?;
            Ok(serde_json::to_string(&j)?)
        },
        |ui, s| {
            if let Ok(v) = serde_json::from_str(&s) {
                message(ui, "Repository result", &outcome(&v));
            }
        },
    );
}
fn preview(ui: &Rc<Ui>, id: String) {
    work(
        ui,
        "Inspecting selected sources…",
        move || Ok(backup_service::client_call("Preview", (id.as_str(),))?),
        |ui, s| {
            if let Ok(p) = serde_json::from_str::<Preview>(&s) {
                let mut body = format!(
                    "{} selected files · {:.2} GiB logical size\n{} unreadable paths\n\nSources:\n{}\n\n{}",
                    p.files,
                    p.logical_bytes as f64 / 1073741824.0,
                    p.unreadable,
                    p.sources
                        .iter()
                        .map(|s| s.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    p.warnings.join("\n\n")
                );
                if !p.blocked.is_empty() {
                    body.push_str(&format!("\n\nBlocked:\n{}", p.blocked.join("\n")));
                }
                message(ui, "Backup preview", &body);
            }
        },
    );
}
fn refresh(ui: &Rc<Ui>) {
    let store = Store::default();
    let c = match store.load() {
        Ok(c) => c,
        Err(e) => {
            ui.notice.set_text(&e.to_string());
            return;
        }
    };
    clear(&ui.profiles);
    clear(&ui.destinations);
    if c.profiles.is_empty() {
        ui.profiles.append(&card("Create your first backup","Add a destination in Settings, then select the folders to protect. Nothing is backed up until you start an approved profile."));
    }
    for p in c.profiles {
        let row = card(
            &p.name,
            &format!(
                "{} folders · {} · Keep {} snapshots · {}",
                p.sources.len(),
                p.destination,
                p.keep_last,
                if p.approved {
                    "Approved"
                } else {
                    "Approval needed"
                }
            ),
        );
        let buttons = w::hbox(4);
        for (icon, tip, kind) in [
            ("document-edit-symbolic", "Edit profile", "edit"),
            (
                "document-properties-symbolic",
                "Preview selected files",
                "preview",
            ),
            ("media-playback-start-symbolic", "Back up now", "backup"),
            (
                "document-open-recent-symbolic",
                "Snapshot history",
                "history",
            ),
            ("emblem-ok-symbolic", "Verify repository metadata", "verify"),
            (
                "drive-harddisk-symbolic",
                "Repository storage statistics",
                "stats",
            ),
            (
                "appointment-soon-symbolic",
                "Enable or disable schedule",
                "schedule",
            ),
            ("edit-clear-all-symbolic", "Preview retention", "retention"),
        ] {
            let u = ui.clone();
            let p = p.clone();
            buttons.append(&action(icon, tip, move || match kind {
                "edit" => edit_profile(&u, Some(p.clone())),
                "preview" => preview(&u, p.id.clone()),
                "schedule" => schedule_dialog(&u, p.clone()),
                "retention" => retention_dialog(&u, p.clone()),
                _ => run_profile(&u, p.id.clone(), kind),
            }));
        }
        let u = ui.clone();
        let id = p.id.clone();
        buttons.append(&action(
            "security-high-symbolic",
            "Full-data verification (reads all repository data)",
            move || run_profile(&u, id.clone(), "verify-full"),
        ));
        row.append(&buttons);
        ui.profiles.append(&row);
    }
    for d in c.destinations {
        let status = match &d.kind {
            DestinationKind::Sftp { .. } | DestinationKind::Smb { .. } => {
                "Connection not tested. Test connectivity and validate the repository separately."
            }
            DestinationKind::Local { .. } => {
                "Local folder; removable-device identity is not bound."
            }
            _ => match backup_system::validate_destination(&d) {
                Ok(_) => "Bound mount available; repository validation is separate.",
                Err(_) => "Destination unavailable or bound mount absent.",
            },
        };
        let row = card(&d.name, status);
        let buttons = w::hbox(4);
        for (icon, tip, kind) in [
            ("document-edit-symbolic", "Edit destination", "edit"),
            (
                "network-transmit-receive-symbolic",
                "Test connectivity only",
                "test",
            ),
            (
                "dialog-password-symbolic",
                "Set repository password",
                "password",
            ),
            (
                "document-save-symbolic",
                "Export recovery password to a new file",
                "export",
            ),
            (
                "emblem-ok-symbolic",
                "Validate existing repository",
                "validate",
            ),
            (
                "list-add-symbolic",
                "Initialize a new repository explicitly",
                "init",
            ),
        ] {
            let u = ui.clone();
            let d = d.clone();
            buttons.append(&action(icon,tip,move||match kind{
    "edit"=>edit_destination(&u,Some(d.clone())),"test"=>{let d=d.clone();work(&u,"Testing connectivity…",move||Ok(serde_json::to_string(&backup_system::connection_test(&Store::default(),&d)?)?),|ui,_|message(ui,"Connected","Connection succeeded. This did not validate or initialize a repository."));},
    "password"=>credential_dialog(&u,d.clone(),false),"export"=>export_dialog(&u,d.id.clone()),
    _=>{let c=Store::default().load().unwrap_or_default();if let Some(p)=c.profiles.iter().find(|p|p.destination==d.id){if kind=="init"{confirm_init(&u,p.id.clone());}else{local_operation(&u,p.id.clone(),"validate");}}else{message(&u,"Profile needed","Create a profile using this destination first.");}},
   }));
        }
        if matches!(d.kind, DestinationKind::Smb { .. }) {
            let u = ui.clone();
            let d = d.clone();
            buttons.append(&action(
                "network-server-symbolic",
                "Set SMB credential and encrypted Rclone configuration",
                move || credential_dialog(&u, d.clone(), true),
            ));
        }
        if matches!(d.kind, DestinationKind::Nfs { .. }) {
            for connect in [true, false] {
                let u = ui.clone();
                let d = d.clone();
                buttons.append(&action(
                    if connect {
                        "network-connect-symbolic"
                    } else {
                        "network-disconnect-symbolic"
                    },
                    if connect {
                        "Connect preconfigured NFS share"
                    } else {
                        "Disconnect NFS share"
                    },
                    move || {
                        let d = d.clone();
                        work(
                            &u,
                            "Updating NFS connection…",
                            move || {
                                let store = Store::default();
                                let _guard = backup_engine::RepoLock::acquire(&store)?;
                                backup_system::nfs_connection(&d, connect)?;
                                Ok(String::new())
                            },
                            |ui, _| refresh(ui),
                        );
                    },
                ));
            }
        }
        row.append(&buttons);
        ui.destinations.append(&row);
    }
}
fn entry(body: &gtk::Box, title: &str, value: &str) -> gtk::Entry {
    body.append(&text(title, "dim-label"));
    let e = gtk::Entry::builder().text(value).hexpand(true).build();
    body.append(&e);
    e
}
fn multiline(body: &gtk::Box, title: &str, value: &str) -> gtk::TextView {
    body.append(&text(title, "dim-label"));
    let v = gtk::TextView::new();
    v.set_wrap_mode(gtk::WrapMode::WordChar);
    v.buffer().set_text(value);
    let sc = gtk::ScrolledWindow::builder()
        .child(&v)
        .min_content_height(90)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    body.append(&sc);
    v
}
fn lines(v: &gtk::TextView) -> Vec<String> {
    v.buffer()
        .text(&v.buffer().start_iter(), &v.buffer().end_iter(), false)
        .lines()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .collect()
}
fn form(d: &gtk::Dialog) -> gtk::Box {
    let b = w::vbox(8);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(20);
    b.set_margin_end(20);
    let sc = gtk::ScrolledWindow::builder()
        .child(&b)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    d.content_area().append(&sc);
    b
}
fn edit_profile(ui: &Rc<Ui>, profile: Option<Profile>) {
    let config = match Store::default().load() {
        Ok(c) => c,
        Err(_) => return,
    };
    if config.destinations.is_empty() {
        message(
            ui,
            "Destination needed",
            "Add a destination in Settings first.",
        );
        return;
    }
    let d = dialog(
        &ui.window,
        if profile.is_some() {
            "Edit backup profile"
        } else {
            "New backup profile"
        },
    );
    d.set_default_height(680);
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Save", gtk::ResponseType::Accept);
    let b = form(&d);
    let id = entry(
        &b,
        "Profile ID",
        profile
            .as_ref()
            .map(|p| p.id.as_str())
            .unwrap_or("personal"),
    );
    id.set_editable(profile.is_none());
    let name = entry(
        &b,
        "Name",
        profile
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("Personal data"),
    );
    b.append(&text("Destination", "dim-label"));
    let names: Vec<_> = config
        .destinations
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    let dest = gtk::DropDown::from_strings(&names);
    if let Some(p) = &profile {
        dest.set_selected(
            config
                .destinations
                .iter()
                .position(|d| d.id == p.destination)
                .unwrap_or(0) as u32,
        );
    }
    b.append(&dest);
    let source = multiline(
        &b,
        "Selected sources — one absolute path per line",
        &profile
            .as_ref()
            .map(|p| {
                p.sources
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
    );
    let row = w::hbox(8);
    let src = source.clone();
    let parent = d.clone();
    row.append(&action("folder-new-symbolic", "Add folder", move || {
        let src = src.clone();
        gtk::FileDialog::new().select_folder(Some(&parent), gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r {
                if let Some(p) = f.path() {
                    let mut text = lines(&src).join("\n");
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&p.display().to_string());
                    src.buffer().set_text(&text);
                }
            }
        });
    }));
    let src = source.clone();
    row.append(&action(
        "user-home-symbolic",
        "Add existing personal folders",
        move || {
            let inventory = backup_system::inventory();
            let mut paths = lines(&src);
            if let Some(v) = inventory["personal_data"].as_array() {
                paths.extend(v.iter().filter_map(|s| s.as_str().map(String::from)));
            }
            paths.sort();
            paths.dedup();
            src.buffer().set_text(&paths.join("\n"));
        },
    ));
    let src = source.clone();
    let parent = d.clone();
    row.append(&action(
        "preferences-system-symbolic",
        "Choose curated and other configuration folders explicitly",
        move || configuration_selection(&parent, &src),
    ));
    b.append(&row);
    let exclude = multiline(
        &b,
        "Extra exclusions — glob patterns, one per line",
        &profile
            .as_ref()
            .map(|p| p.exclusions.join("\n"))
            .unwrap_or_default(),
    );
    b.append(&text("Caches, trash, repository destinations and recognized database/VM stores are excluded. Unknown build folders are preserved. Stop active stores or select an export.","dim-label"));
    let secrets = gtk::CheckButton::with_label(
        "I authorize recognized keys and credential stores in this encrypted backup",
    );
    secrets.set_active(profile.as_ref().is_some_and(|p| p.include_secrets));
    b.append(&secrets);
    b.append(&text(
        "Arbitrary files may also contain secrets. Keep the recovery password separately.",
        "dim-label",
    ));
    let approved = gtk::CheckButton::with_label(
        "Approve this profile for manual, scheduled and integration requests",
    );
    approved.set_active(profile.as_ref().is_some_and(|p| p.approved));
    b.append(&approved);
    let keep = gtk::SpinButton::with_range(1.0, 10000.0, 1.0);
    keep.set_value(profile.as_ref().map(|p| p.keep_last).unwrap_or(7) as f64);
    b.append(&w::row(
        "Snapshots to keep",
        "Deletion requires a separate retention preview and confirmation.",
        Some(keep.upcast_ref()),
    ));
    let calendar = entry(
        &b,
        "Schedule (systemd calendar); enable separately",
        profile
            .as_ref()
            .and_then(|p| p.schedule.as_deref())
            .unwrap_or("daily"),
    );
    let err = text("", "error");
    b.append(&err);
    let ui = ui.clone();
    d.connect_response(move |d, response| {
        if response != gtk::ResponseType::Accept {
            d.close();
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let mut c = Store::default().load()?;
            let p = Profile {
                id: id.text().to_string(),
                name: name.text().to_string(),
                destination: config.destinations[dest.selected() as usize].id.clone(),
                sources: lines(&source).iter().map(PathBuf::from).collect(),
                exclusions: lines(&exclude),
                include_secrets: secrets.is_active(),
                approved: approved.is_active(),
                keep_last: keep.value() as u32,
                schedule: if calendar.text().is_empty() {
                    None
                } else {
                    Some(calendar.text().to_string())
                },
            };
            if profile.is_none() {
                anyhow::ensure!(
                    !c.profiles.iter().any(|x| x.id == p.id),
                    "Profile ID already exists"
                );
            }
            c.profiles.retain(|x| x.id != p.id);
            c.profiles.push(p);
            Store::default().save(&c)
        })();
        match result {
            Ok(_) => {
                d.close();
                refresh(&ui)
            }
            Err(e) => err.set_text(&e.to_string()),
        }
    });
    d.present();
}
fn configuration_selection(parent: &gtk::Dialog, source: &gtk::TextView) {
    let d = gtk::Dialog::builder()
        .transient_for(parent)
        .modal(true)
        .title("Choose configuration folders")
        .default_width(540)
        .default_height(500)
        .build();
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Add selected", gtk::ResponseType::Accept);
    let b = form(&d);
    let v = backup_system::inventory();
    let mut checks = Vec::new();
    for (key, title) in [
        ("curated_configuration", "Curated configuration"),
        (
            "other_configuration_for_explicit_selection",
            "Other configuration — review before including",
        ),
    ] {
        b.append(&text(title, "heading"));
        if let Some(paths) = v[key].as_array() {
            for p in paths {
                if let Some(path) = p.as_str() {
                    let check = gtk::CheckButton::with_label(path);
                    b.append(&check);
                    checks.push((check, path.to_string()));
                }
            }
        }
    }
    let keys = gtk::EventControllerKey::new();
    let dd = d.clone();
    keys.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            dd.response(gtk::ResponseType::Cancel);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    d.add_controller(keys);
    let source = source.clone();
    d.connect_response(move |d, r| {
        if r == gtk::ResponseType::Accept {
            let mut paths = lines(&source);
            for (c, p) in &checks {
                if c.is_active() {
                    paths.push(p.clone())
                }
            }
            paths.sort();
            paths.dedup();
            source.buffer().set_text(&paths.join("\n"));
        }
        d.close();
    });
    d.present();
}
fn edit_destination(ui: &Rc<Ui>, destination: Option<Destination>) {
    let d = dialog(
        &ui.window,
        if destination.is_some() {
            "Edit destination"
        } else {
            "New destination"
        },
    );
    d.set_default_height(650);
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Save", gtk::ResponseType::Accept);
    let b = form(&d);
    let id = entry(
        &b,
        "Destination ID",
        destination
            .as_ref()
            .map(|d| d.id.as_str())
            .unwrap_or("backup-drive"),
    );
    id.set_editable(destination.is_none());
    let name = entry(
        &b,
        "Name",
        destination
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("Backup drive"),
    );
    let kind = gtk::DropDown::from_strings(&[
        "Local folder",
        "Mounted external storage (bind identity)",
        "Preconfigured user-mountable NFS",
        "SFTP via SSH config/agent",
        "SMB via encrypted Rclone config",
    ]);
    b.append(&kind);
    let index = destination
        .as_ref()
        .map(|d| match d.kind {
            DestinationKind::Local { .. } => 0,
            DestinationKind::Mounted { .. } => 1,
            DestinationKind::Nfs { .. } => 2,
            DestinationKind::Sftp { .. } => 3,
            DestinationKind::Smb { .. } => 4,
        })
        .unwrap_or(0);
    kind.set_selected(index);
    let old = destination
        .as_ref()
        .map(|d| serde_json::to_value(d).unwrap())
        .unwrap_or_default();
    let val = |key: &str| old[key].as_str().unwrap_or("");
    let path = entry(
        &b,
        "Repository path (absolute for local/SFTP; within share for SMB)",
        val("path"),
    );
    let browse = action("folder-open-symbolic", "Choose repository folder", {
        let path = path.clone();
        let parent = d.clone();
        move || {
            let path = path.clone();
            gtk::FileDialog::new().select_folder(Some(&parent), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    if let Some(p) = f.path() {
                        path.set_text(&p.display().to_string());
                    }
                }
            });
        }
    });
    b.append(&browse);
    let host = entry(
        &b,
        "SSH host alias or SMB server",
        if index == 4 {
            val("server")
        } else {
            val("host")
        },
    );
    let share = entry(&b, "SMB share", val("share"));
    let user = entry(&b, "SMB username", val("username"));
    let domain = entry(&b, "SMB domain (optional)", val("domain"));
    let mount = entry(
        &b,
        "NFS mount point (administrator configured)",
        val("mount_point"),
    );
    let source = entry(&b, "NFS source (server:/export)", val("source"));
    let filesystem = entry(
        &b,
        "NFS filesystem type (nfs or nfs4)",
        if val("filesystem").is_empty() {
            "nfs4"
        } else {
            val("filesystem")
        },
    );
    b.append(&text("Mounted storage is bound to its exact mount point, filesystem and source when saved. NFS requires an existing user/users fstab entry. SFTP uses existing SSH settings and agent authentication. Set repository and SMB passwords separately after saving.","dim-label"));
    let error = text("", "error");
    b.append(&error);
    // Keep irrelevant fields visible but disabled so their purpose stays clear.
    let update = {
        let host = host.clone();
        let share = share.clone();
        let user = user.clone();
        let domain = domain.clone();
        let mount = mount.clone();
        let source = source.clone();
        let filesystem = filesystem.clone();
        move |i| {
            host.set_sensitive(i >= 3);
            for e in [&share, &user, &domain] {
                e.set_sensitive(i == 4);
            }
            for e in [&mount, &source, &filesystem] {
                e.set_sensitive(i == 2);
            }
        }
    };
    update(index);
    kind.connect_selected_notify(move |k| update(k.selected()));
    let ui = ui.clone();
    d.connect_response(move|d,r|{if r!=gtk::ResponseType::Accept{d.close();return}let result=(||->anyhow::Result<()>{let pathbuf=PathBuf::from(path.text().as_str());let k=match kind.selected(){
  0=>DestinationKind::Local{path:pathbuf},1=>{let mut ancestor=pathbuf.as_path();while !ancestor.exists(){ancestor=ancestor.parent().ok_or_else(||anyhow::anyhow!("Choose an existing mounted destination"))?;}let m=backup_system::mount_identity(ancestor)?;anyhow::ensure!(m["target"]!="/","This is on the root filesystem. Choose a mounted device or use Local folder.");DestinationKind::Mounted{path:pathbuf,mount_point:PathBuf::from(m["target"].as_str().unwrap_or("")),source:m["source"].as_str().unwrap_or("").into(),filesystem:m["fstype"].as_str().unwrap_or("").into(),uuid:m["uuid"].as_str().unwrap_or("").into()}},
  2=>DestinationKind::Nfs{path:pathbuf,mount_point:PathBuf::from(mount.text().as_str()),source:source.text().to_string(),filesystem:filesystem.text().to_string()},
  3=>DestinationKind::Sftp{host:host.text().to_string(),path:path.text().to_string()},_=>DestinationKind::Smb{server:host.text().to_string(),share:share.text().to_string(),path:path.text().to_string(),username:user.text().to_string(),domain:domain.text().to_string()},};let dest=Destination{id:id.text().to_string(),name:name.text().to_string(),kind:k};let mut c=Store::default().load()?;if destination.is_none(){anyhow::ensure!(!c.destinations.iter().any(|x|x.id==dest.id),"Destination ID already exists");}c.destinations.retain(|x|x.id!=dest.id);c.destinations.push(dest);Store::default().save(&c)})();match result{Ok(_)=>{d.close();refresh(&ui)},Err(e)=>error.set_text(&e.to_string())}});
    d.present();
}
fn credential_dialog(ui: &Rc<Ui>, dest: Destination, smb: bool) {
    let d = dialog(
        &ui.window,
        if smb {
            "SMB credentials"
        } else {
            "Repository password"
        },
    );
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Save in Secret Service", gtk::ResponseType::Accept);
    let b = form(&d);
    b.append(&text(if smb{"The NAS password is stored in Secret Service and an encrypted Rclone configuration."}else{"Use the existing password for an existing repository. For a new repository, choose a strong password or generate one. Export a recovery copy separately; losing this password makes the backups unrecoverable."},"dim-label"));
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    b.append(&password);
    if !smb {
        let p = password.clone();
        b.append(&action(
            "view-refresh-symbolic",
            "Generate a strong random repository password",
            move || {
                if let Ok(secret) = backup_system::generated_secret() {
                    p.set_text(secret.expose());
                }
            },
        ));
    }
    let ui = ui.clone();
    d.connect_response(move |d, r| {
        if r != gtk::ResponseType::Accept {
            d.close();
            return;
        }
        let secret = match nyx_sync::Secret::new(password.text().to_string()) {
            Ok(s) => s,
            Err(_) => return,
        };
        password.set_text("");
        let dest = dest.clone();
        work(
            &ui,
            "Saving credential in Secret Service…",
            move || {
                if smb {
                    backup_system::setup_smb(&Store::default(), &dest, &secret)?
                } else {
                    backup_system::save_password(&dest.id, &secret)?
                }
                Ok(String::new())
            },
            |ui, _| {
                ui.notice.set_text(
                    "Credential saved. Repository initialization remains a separate action.",
                )
            },
        );
        d.close();
    });
    d.present();
}
fn export_dialog(ui: &Rc<Ui>, id: String) {
    let ui = ui.clone();
    let parent = ui.window.clone();
    gtk::FileDialog::builder().title("Export recovery password — keep it separate from backups").initial_name("backup-recovery-password.txt").build().save(Some(&parent),gio::Cancellable::NONE,move|r|{if let Ok(file)=r{if let Some(path)=file.path(){work(&ui,"Exporting recovery password…",move||{backup_system::export_password(&id,&path)?;Ok(String::new())},|ui,_|message(ui,"Recovery password exported","The new file contains the repository password. Keep it in a secure place separate from this computer and backup repository."));}}});
}
fn confirm_init(ui: &Rc<Ui>, id: String) {
    let d = dialog(&ui.window, "Initialize a new repository?");
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Initialize", gtk::ResponseType::Accept);
    form(&d).append(&text("This creates a new encrypted repository at the selected destination. Use Validate for an existing repository. A failed connection is never repaired by initialization.",""));
    let ui = ui.clone();
    d.connect_response(move |d, r| {
        if r == gtk::ResponseType::Accept {
            local_operation(&ui, id.clone(), "init");
        }
        d.close();
    });
    d.present();
}
fn schedule_dialog(ui: &Rc<Ui>, p: Profile) {
    let d = dialog(&ui.window, "Backup schedule");
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Disable", gtk::ResponseType::Reject);
    d.add_button("Enable", gtk::ResponseType::Accept);
    form(&d).append(&text(&format!("Calendar: {}\n\nScheduling requires an approved profile and an unlocked keyring. User timers run while your user manager is active. Lingering is never enabled automatically.",p.schedule.as_deref().unwrap_or("No schedule configured")),""));
    let ui = ui.clone();
    d.connect_response(move |d, r| {
        if [gtk::ResponseType::Accept, gtk::ResponseType::Reject].contains(&r) {
            let p = p.clone();
            work(
                &ui,
                "Updating schedule…",
                move || {
                    backup_system::schedule(&Store::default(), &p, r == gtk::ResponseType::Accept)?;
                    Ok(String::new())
                },
                |ui, _| ui.notice.set_text("Schedule updated"),
            );
        }
        d.close();
    });
    d.present();
}
fn retention_dialog(ui: &Rc<Ui>, p: Profile) {
    let id = p.id.clone();
    work(
        ui,
        "Previewing profile retention…",
        move || {
            let store = Store::default();
            let c = store.load()?;
            Ok(serde_json::to_string(&backup_engine::run(
                &store,
                &c,
                c.profile(&id)?,
                "retention-preview",
            )?)?)
        },
        move |ui, s| {
            let j: serde_json::Value = serde_json::from_str(&s).unwrap_or_default();
            if j["state"] != "success" {
                message(ui, "Retention preview failed", &outcome(&j));
                return;
            }
            let d = dialog(&ui.window, "Review retention before deleting snapshots");
            d.add_button("Cancel", gtk::ResponseType::Cancel);
            d.add_button("Apply retention", gtk::ResponseType::Accept);
            let b = form(&d);
            b.append(&text(&format!("Profile: {}\nKeep {} snapshots. Only snapshots with this Backup profile's tags are eligible.\n\n{}",p.name,p.keep_last,outcome(&j)),""));
            let confirmation = entry(&b, "Type the profile ID to confirm snapshot deletion", "");
            let ui = ui.clone();
            d.connect_response(move |d, r| {
                if r == gtk::ResponseType::Accept {
                    if confirmation.text() != p.id {
                        return;
                    }
                    local_operation(&ui, p.id.clone(), "retention-apply");
                }
                d.close();
            });
            d.present();
        },
    );
}
fn settings(ui: &Rc<Ui>) {
    settings_card::open(
        &ui.overlay,
        &ui.window,
        settings_card::Spec {
            groups: &[],
            search_text: None,
            standard: Some(standard()),
        },
        || {
            let (scroll, b) = page("Preferences", "");
            (scroll.upcast(), b)
        },
    );
}
fn shortcuts(ui: &Rc<Ui>) {
    message(
        ui,
        "Keyboard shortcuts",
        &SHORTCUTS
            .iter()
            .map(|(k, v)| format!("{} — {v}", k.join("+")))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}
fn build(app: &gtk::Application) {
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
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Nyx Backup")
        .default_width(1040)
        .default_height(740)
        .build();
    window.add_css_class("backup-window");
    let overlay = gtk::Overlay::new();
    let outer = w::vbox(0);
    overlay.set_child(Some(&outer));
    window.set_child(Some(&overlay));
    let header = chrome::app_header(&window, APP_ID, "Nyx Backup");
    outer.append(&header.root);
    let main = w::hbox(0);
    main.set_vexpand(true);
    outer.append(&main);
    let sidebar = w::vbox(6);
    sidebar.add_css_class("sidebar");
    sidebar.set_width_request(212);
    sidebar.set_hexpand(false);
    main.append(&sidebar);
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    main.append(&stack);
    let (overview, body) = page(
        "Your data, protected",
        "Encrypted snapshots you can read with standard Restic tools.",
    );
    let coverage = card(
        "Selected user data",
        "Backs up the folders you select and readable system metadata. Complete system, cross-PC and boot recovery are planned and unqualified.",
    );
    body.append(&coverage);
    let health = text("Checking backup service…", "heading");
    let last = text("No successful backup recorded", "dim-label");
    let next = text(
        "Schedules are disabled until explicitly enabled",
        "dim-label",
    );
    let storage = text(
        "Storage statistics not collected — use the storage action on a profile.",
        "dim-label",
    );
    let activity = text("No active job", "dim-label");
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    let summary = card("Backup health", "");
    summary.append(&health);
    summary.append(&last);
    summary.append(&next);
    summary.append(&storage);
    summary.append(&activity);
    summary.append(&progress);
    let quick = w::hbox(6);
    summary.append(&quick);
    body.append(&summary);
    stack.add_named(&overview, Some("overview"));
    let (backups, b) = page(
        "Backups",
        "Select folders, preview coverage, then approve and run a profile.",
    );
    let add_profile = w::labeled_button("list-add-symbolic", "New profile");
    b.append(&add_profile);
    let profiles = w::vbox(12);
    b.append(&profiles);
    let jobs = w::labeled_button("document-open-recent-symbolic", "Job history");
    b.append(&jobs);
    stack.add_named(&backups, Some("backups"));
    let (restore, b) = page("Restore", "Phase 1 repositories remain readable by Restic.");
    b.append(&card("Recovery available through Restic","Use Restic with your repository location and recovery password to list snapshots and restore into a separate empty folder. Backup does not expose restore or destructive recovery through its integration API."));
    b.append(&card("Planned","Snapshot browsing, selective restore, Quick Dotfiles Restore, conflict handling and rollback arrive in Phase 2. System recovery and bootable recovery remain later phases until demonstrated in a VM."));
    let docs = gtk::LinkButton::with_label(
        "https://restic.readthedocs.io/en/stable/050_restore.html",
        "Open Restic recovery documentation",
    );
    b.append(&docs);
    stack.add_named(&restore, Some("restore"));
    let (settings_page, b) = page("Settings", "Manage destinations and security.");
    let add_dest = w::labeled_button("list-add-symbolic", "New destination");
    b.append(&add_dest);
    let destinations = w::vbox(12);
    b.append(&destinations);
    let notify = gtk::Switch::new();
    notify.set_active(Store::default().load().unwrap_or_default().notifications);
    b.append(&w::row(
        "Completion notifications",
        "Basic completion and failure notifications; no source or repository details.",
        Some(notify.upcast_ref()),
    ));
    notify.connect_active_notify(|s| {
        if let Ok(mut c) = Store::default().load() {
            c.notifications = s.is_active();
            let _ = Store::default().save(&c);
        }
    });
    let appearance = w::labeled_button(
        "preferences-system-symbolic",
        "Appearance, keyboard and About",
    );
    b.append(&appearance);
    stack.add_named(&settings_page, Some("settings"));
    let status = w::hbox(10);
    status.add_css_class("status-bar");
    let notice = text("Ready", "dim-label");
    notice.set_hexpand(true);
    notice.set_ellipsize(gtk::pango::EllipsizeMode::End);
    notice.set_wrap(false);
    status.append(&notice);
    outer.append(&status);
    let ui = Rc::new(Ui {
        window: window.clone(),
        overlay,
        stack: stack.clone(),
        profiles,
        destinations,
        health,
        last,
        next,
        storage,
        activity,
        progress,
        notice,
        job: RefCell::new(None),
        busy: Cell::new(false),
    });
    let compact = Rc::new(Cell::new(false));
    let nav_labels = Rc::new(RefCell::new(Vec::<gtk::Label>::new()));
    let nav_buttons = Rc::new(RefCell::new(Vec::<gtk::Button>::new()));
    for (id, title, icon) in [
        ("overview", "Overview", "view-grid-symbolic"),
        ("backups", "Backups", "document-save-symbolic"),
        ("restore", "Restore", "edit-undo-symbolic"),
        ("settings", "Settings", "preferences-system-symbolic"),
    ] {
        let button = gtk::Button::new();
        button.add_css_class("nav-item");
        button.set_tooltip_text(Some(title));
        let row = w::hbox(10);
        row.append(&gtk::Image::from_icon_name(icon));
        let label = text(title, "");
        row.append(&label);
        button.set_child(Some(&row));
        sidebar.append(&button);
        nav_labels.borrow_mut().push(label);
        nav_buttons.borrow_mut().push(button.clone());
        let stack = stack.clone();
        button.connect_clicked(move |_| stack.set_visible_child_name(id));
    }
    let buttons = nav_buttons.clone();
    stack.connect_visible_child_name_notify(move |s| {
        let name = s.visible_child_name().unwrap_or_default();
        for (button, id) in buttons
            .borrow()
            .iter()
            .zip(["overview", "backups", "restore", "settings"])
        {
            if name == id {
                button.add_css_class("active")
            } else {
                button.remove_css_class("active")
            }
        }
    });
    stack.set_visible_child_name("overview");
    nav_buttons.borrow()[0].add_css_class("active");
    let sidebar_width = sidebar.clone();
    let labels_width = nav_labels.clone();
    let compact_width = compact.clone();
    window.add_tick_callback(move |window, _| {
        let on = window.width() < 980 || compact_width.get();
        sidebar_width.set_width_request(if on { 56 } else { 212 });
        for l in labels_width.borrow().iter() {
            l.set_visible(!on);
        }
        glib::ControlFlow::Continue
    });
    let toggle = action("sidebar-show-symbolic", "Collapse sidebar (Ctrl+B)", {
        let sidebar = sidebar.clone();
        let labels = nav_labels.clone();
        let compact = compact.clone();
        move || {
            let on = !compact.get();
            compact.set(on);
            sidebar.set_width_request(if on { 56 } else { 212 });
            for l in labels.borrow().iter() {
                l.set_visible(!on)
            }
        }
    });
    header.start.append(&toggle);
    let u = ui.clone();
    header
        .end
        .append(&chrome::settings_button(move || settings(&u)));
    let u = ui.clone();
    status.append(&chrome::shortcuts_button(move || shortcuts(&u)));
    let u = ui.clone();
    add_profile.connect_clicked(move |_| edit_profile(&u, None));
    let u = ui.clone();
    add_dest.connect_clicked(move |_| edit_destination(&u, None));
    let u = ui.clone();
    appearance.connect_clicked(move |_| settings(&u));
    let u = ui.clone();
    quick.append(&action(
        "media-playback-start-symbolic",
        "Back up the first approved profile",
        move || {
            if let Ok(c) = Store::default().load() {
                if let Some(p) = c.profiles.iter().find(|p| p.approved) {
                    run_profile(&u, p.id.clone(), "backup")
                } else {
                    message(
                        &u,
                        "No approved profile",
                        "Create and approve a profile on the Backups page.",
                    )
                }
            }
        },
    ));
    let u = ui.clone();
    quick.append(&action(
        "media-playback-stop-symbolic",
        "Cancel active job",
        move || {
            if let Some(id) = u.job.borrow().clone() {
                work(
                    &u,
                    "Requesting cancellation…",
                    move || Ok(backup_service::client_call("Cancel", (id.as_str(),))?),
                    |ui, s| ui.notice.set_text(&s),
                );
            }
        },
    ));
    let u = ui.clone();
    jobs.connect_clicked(move |_| {
        work(
            &u,
            "Reading job history…",
            || Ok(backup_service::client_call("Jobs", ())?),
            |ui, s| {
                let jobs: Vec<Job> = serde_json::from_str(&s).unwrap_or_default();
                let body = jobs
                    .iter()
                    .map(|j| {
                        format!(
                            "{} · {} · {:?}\n{}\n",
                            j.profile, j.operation, j.state, j.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                message(
                    ui,
                    "Job history",
                    if body.is_empty() {
                        "No jobs recorded"
                    } else {
                        &body
                    },
                );
            },
        );
    });
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common {
            close: true,
            ..Default::default()
        },
    );
    let keys = gtk::EventControllerKey::new();
    let u = ui.clone();
    keys.connect_key_pressed(move |_, k, _, mods| {
        let ctrl = mods.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        if k == gtk::gdk::Key::F1 {
            shortcuts(&u);
        } else if k == gtk::gdk::Key::comma && ctrl {
            settings(&u);
        } else if (k == gtk::gdk::Key::b || k == gtk::gdk::Key::B) && ctrl {
            toggle.emit_clicked();
        } else if k == gtk::gdk::Key::Escape && settings_card::is_open() {
            settings_card::escape();
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    // Polling is lightweight; all bus calls and repository work run off the GTK thread.
    let poll = Rc::new(Cell::new(false));
    let weak = Rc::downgrade(&ui);
    let update = move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if poll.replace(true) {
            return glib::ControlFlow::Continue;
        }
        let poll = poll.clone();
        nyx_core::cmd::background(backup_service::status, move |result| {
            poll.set(false);
            match result {
                Ok(s) => {
                    if let Ok(h) = serde_json::from_str::<Health>(&s) {
                        if let Some(stats) = &h.storage {
                            ui.storage.set_text(&format!(
                                "Repository data: {:.2} GiB · {} blobs (last collected statistics)",
                                stats["total_size"].as_u64().unwrap_or(0) as f64 / 1073741824.0,
                                stats["total_blob_count"].as_u64().unwrap_or(0)
                            ));
                        }
                        ui.health.set_text(if h.warning.is_some() {
                            "Last backup needs attention"
                        } else if h.active.is_some() {
                            "Backup service is working"
                        } else if h.last_success.is_some() {
                            "Backup available"
                        } else {
                            "No successful backup yet"
                        });
                        ui.last.set_text(
                            &h.last_success
                                .as_ref()
                                .map(|j| {
                                    format!(
                                        "Last success: {} · {}",
                                        j.profile,
                                        readable_time(j.finished.unwrap_or(j.started))
                                    )
                                })
                                .unwrap_or_else(|| "No successful backup recorded".into()),
                        );
                        let scheduled: Vec<_> = h
                            .next_schedule
                            .iter()
                            .filter_map(|t| {
                                let next = t["next"].as_u64().unwrap_or(0);
                                if next == 0 || next == u64::MAX {
                                    None
                                } else {
                                    Some(format!(
                                        "{}: {}",
                                        t["unit"].as_str().unwrap_or("Schedule"),
                                        readable_time(next / 1_000_000)
                                    ))
                                }
                            })
                            .collect();
                        ui.next.set_text(&if scheduled.is_empty() {
                            "No active schedules".into()
                        } else {
                            format!("Next backup — {}", scheduled.join("; "))
                        });
                        if let Some(warning) = &h.warning {
                            ui.activity.set_text(warning);
                        }
                        if let Some(j) = h.active {
                            *ui.job.borrow_mut() = Some(j.id);
                            ui.activity
                                .set_text(&format!("{} · {}", j.profile, j.operation));
                            let fraction = j.progress["percent_done"].as_f64().unwrap_or(0.0);
                            ui.progress.set_fraction(fraction.clamp(0.0, 1.0));
                            ui.progress.set_text(Some(&format!(
                                "{:.0}% · {} bytes processed",
                                fraction * 100.0,
                                j.progress["bytes_done"].as_u64().unwrap_or(0)
                            )));
                        } else {
                            ui.progress.set_fraction(0.0);
                            ui.progress.set_text(Some("Idle"));
                            if let Some(id) = ui.job.borrow_mut().take() {
                                let u = ui.clone();
                                nyx_core::cmd::background(
                                    move || backup_service::client_call("Job", (id.as_str(),)),
                                    move |r| {
                                        if let Ok(s) = r {
                                            if let Ok(v) = serde_json::from_str(&s) {
                                                u.activity.set_text(&outcome(&v));
                                            }
                                        }
                                    },
                                );
                            }
                        }
                    }
                }
                Err(_) => ui
                    .health
                    .set_text("Backup service unavailable — check user service installation"),
            }
        });
        glib::ControlFlow::Continue
    };
    refresh(&ui);
    glib::timeout_add_local(std::time::Duration::from_secs(2), update);
    // Keep the UI model alive for the window's lifetime.
    window.connect_close_request(move |_| {
        let _ = &ui;
        glib::Propagation::Proceed
    });
    window.present();
}
fn main() {
    backup_core::init();
    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run();
}

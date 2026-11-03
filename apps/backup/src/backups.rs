//! The Backups view: one card per profile, a setup checklist until the first
//! backup succeeds, and the profile editor and actions.
use crate::forms::{self, Sheet};
use crate::ui::*;
use backup_core::*;
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

pub struct View {
    pub root: gtk::ScrolledWindow,
    banner: gtk::Box,
    summary: gtk::Box,
    summary_icon: gtk::Image,
    summary_title: gtk::Label,
    summary_note: gtk::Label,
    setup: gtk::Box,
    cards: gtk::Box,
    widgets: RefCell<Vec<Card>>,
}

struct Card {
    profile: String,
    root: gtk::Box,
    status_icon: gtk::Image,
    status: gtk::Label,
    schedule: gtk::Label,
    run: gtk::Button,
    progress_box: gtk::Box,
    progress: gtk::ProgressBar,
    progress_text: gtk::Label,
}

impl View {
    pub fn new() -> View {
        let body = w::vbox(12);
        body.add_css_class("view-body");
        let banner = w::vbox(0);
        body.append(&banner);
        let summary = w::hbox(10);
        summary.add_css_class("health-line");
        let summary_icon = gtk::Image::from_icon_name("object-select-symbolic");
        summary_icon.add_css_class("health-icon");
        summary.append(&summary_icon);
        let summary_title = line("", "health-title");
        summary.append(&summary_title);
        let summary_note = line("", "health-note");
        summary_note.set_hexpand(true);
        summary_note.set_xalign(1.0);
        summary.append(&summary_note);
        summary.set_visible(false);
        body.append(&summary);
        let setup = w::vbox(0);
        body.append(&setup);
        let cards = w::vbox(12);
        body.append(&cards);
        let clamp = nyx_ui::clamp::Clamp::new(&body, 760);
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();
        View {
            root,
            banner,
            summary,
            summary_icon,
            summary_title,
            summary_note,
            setup,
            cards,
            widgets: RefCell::new(vec![]),
        }
    }
}

/// Rebuild the cards from the saved configuration.
pub fn refresh(ui: &Rc<Ui>) {
    let v = &ui.backups;
    let c = match Store::default().load() {
        Ok(c) => c,
        Err(e) => {
            clear(&v.cards);
            v.cards.append(&w::banner(
                &glib_escape(&format!("The backup configuration can't be read: {e}")),
                true,
            ));
            return;
        }
    };
    clear(&v.cards);
    v.widgets.borrow_mut().clear();
    let single = c.profiles.len() == 1;
    for p in &c.profiles {
        let card = card(ui, p, &c, single);
        v.cards.append(&card.root);
        v.widgets.borrow_mut().push(card);
    }
    checklist(ui, &c);
    render_status(ui, ui.health.borrow().is_some() || c.profiles.is_empty());
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

fn card(ui: &Rc<Ui>, p: &Profile, c: &Config, single: bool) -> Card {
    let root = w::vbox(10);
    root.add_css_class("card");
    root.add_css_class("profile-card");
    let head = w::hbox(12);
    let icon = gtk::Image::from_icon_name("drive-harddisk-symbolic");
    icon.add_css_class("card-icon");
    icon.set_valign(gtk::Align::Start);
    head.append(&icon);
    let titles = w::vbox(2);
    titles.set_hexpand(true);
    let title_row = w::hbox(8);
    title_row.append(&line(&p.name, "card-title"));
    if !p.approved {
        let t = w::tag("Needs approval", "warning");
        t.set_tooltip_text(Some(
            "Backups start only after you allow this profile to run",
        ));
        title_row.append(&t);
    }
    titles.append(&title_row);
    let dest = c
        .destination(&p.destination)
        .map(|d| d.name.clone())
        .unwrap_or_else(|_| p.destination.clone());
    titles.append(&line(
        &format!(
            "{} → {dest} · keeps {}",
            count(p.sources.len() as u64, "folder", "folders"),
            count(p.keep_last as u64, "snapshot", "snapshots"),
        ),
        "card-subtitle",
    ));
    head.append(&titles);
    let run = gtk::Button::from_icon_name("media-playback-start-symbolic");
    run.set_tooltip_text(Some("Back up now (Ctrl+Enter)"));
    run.set_valign(gtk::Align::Center);
    run.add_css_class(if single {
        "suggested-action"
    } else {
        "icon-button"
    });
    run.add_css_class("run-button");
    {
        let ui = ui.clone();
        let p = p.clone();
        run.connect_clicked(move |_| back_up(&ui, &p));
    }
    head.append(&run);
    head.append(&profile_menu(ui, p));
    root.append(&head);

    let status_row = w::hbox(8);
    status_row.add_css_class("card-status");
    let status_icon = gtk::Image::from_icon_name("content-loading-symbolic");
    status_icon.add_css_class("status-icon");
    status_row.append(&status_icon);
    let status = line("Checking…", "status-text");
    status.set_hexpand(true);
    status_row.append(&status);
    let schedule = line("", "schedule-text");
    status_row.append(&schedule);
    root.append(&status_row);

    let progress_box = w::vbox(6);
    progress_box.set_visible(false);
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("job-progress");
    progress_box.append(&progress);
    let progress_row = w::hbox(8);
    let progress_text = line("", "progress-text");
    progress_text.set_hexpand(true);
    progress_row.append(&progress_text);
    {
        let ui = ui.clone();
        let profile = p.id.clone();
        progress_row.append(&icon_button(
            "media-playback-stop-symbolic",
            "Cancel this job",
            move || cancel_active(&ui, &profile),
        ));
    }
    progress_box.append(&progress_row);
    root.append(&progress_box);
    Card {
        profile: p.id.clone(),
        root,
        status_icon,
        status,
        schedule,
        run,
        progress_box,
        progress,
        progress_text,
    }
}

fn profile_menu(ui: &Rc<Ui>, p: &Profile) -> gtk::MenuButton {
    let scheduled = timer_next(ui, &p.id).is_some();
    let item = |f: fn(&Rc<Ui>, &Profile)| -> Box<dyn Fn()> {
        let ui = ui.clone();
        let p = p.clone();
        Box::new(move || f(&ui, &p))
    };
    let mut items: Vec<MenuItem> = vec![
        ("Preview what's included", false, item(preview)),
        (
            "Browse snapshots",
            false,
            item(|ui, p| {
                crate::restore_ui::open_profile(ui, &p.id);
            }),
        ),
        ("-", false, Box::new(|| {})),
        (
            "Check repository",
            false,
            item(|ui, p| run(ui, p, "verify")),
        ),
        (
            "Check all data",
            false,
            item(|ui, p| run(ui, p, "verify-full")),
        ),
        ("Storage use", false, item(|ui, p| run(ui, p, "stats"))),
        ("Clean up old snapshots…", false, item(retention)),
        ("-", false, Box::new(|| {})),
    ];
    if scheduled {
        items.push((
            "Turn schedule off",
            false,
            item(|ui, p| set_schedule(ui, p, false)),
        ));
    } else if p.schedule.is_some() {
        items.push((
            "Turn schedule on",
            false,
            item(|ui, p| set_schedule(ui, p, true)),
        ));
    }
    items.push(("Edit…", false, item(|ui, p| edit(ui, Some(p.clone())))));
    items.push(("Delete…", true, item(delete)));
    menu("More actions for this backup", items)
}

/// The next run of a profile's timer, in seconds, if its timer is on.
fn timer_next(ui: &Rc<Ui>, id: &str) -> Option<u64> {
    let health = ui.health.borrow();
    let unit = format!("nyx-backup-{id}.timer");
    health.as_ref()?.next_schedule.iter().find_map(|t| {
        (t["unit"].as_str() == Some(unit.as_str())).then(|| {
            let next = t["next"].as_u64().unwrap_or(0);
            if next == 0 || next == u64::MAX {
                0
            } else {
                next / 1_000_000
            }
        })
    })
}

fn schedule_name(calendar: &str) -> String {
    match calendar {
        "hourly" => "Hourly".into(),
        "daily" => "Daily".into(),
        "weekly" => "Weekly".into(),
        "monthly" => "Monthly".into(),
        other => other.to_string(),
    }
}

/// Update the health line and every card from the latest service status.
pub fn render_status(ui: &Rc<Ui>, service: bool) {
    let v = &ui.backups;
    clear(&v.banner);
    if !service {
        let b = w::banner(
            "The background service isn't responding. Backups, checks and restores need it.",
            true,
        );
        let retry = gtk::Button::with_label("Try again");
        let u = ui.clone();
        retry.connect_clicked(move |_| crate::status::poll_soon(&u));
        retry.set_valign(gtk::Align::Center);
        b.append(&retry);
        v.banner.append(&b);
    }
    let health = ui.health.borrow().clone();
    let config = Store::default().load().unwrap_or_default();
    let active = health.as_ref().and_then(|h| h.active.clone());
    for card in v.widgets.borrow().iter() {
        let info = health.as_ref().and_then(|h| {
            h.profiles
                .iter()
                .find(|p| p.profile == card.profile)
                .cloned()
        });
        let (icon, text, class) = match info.as_ref().and_then(|i| i.last_backup.as_ref()) {
            None => ("radio-symbolic", "Not backed up yet".to_string(), "pending"),
            Some(j) => {
                let at = when(j.finished.unwrap_or(j.started));
                match j.state {
                    JobState::Success => {
                        let r = &j.result;
                        let mut s = format!("Backed up {}", at.to_lowercase_first());
                        if let Some(files) = r["total_files_processed"].as_u64() {
                            s.push_str(&format!(" · {}", count(files, "file", "files")));
                        }
                        if let Some(added) = r["data_added"].as_u64() {
                            s.push_str(&format!(" · {} added", size(added)));
                        }
                        ("object-select-symbolic", s, "ok")
                    }
                    JobState::Incomplete => (
                        "dialog-warning-symbolic",
                        format!(
                            "Incomplete {}: some files couldn't be read",
                            at.to_lowercase_first()
                        ),
                        "warning",
                    ),
                    JobState::Cancelled => (
                        "dialog-warning-symbolic",
                        format!("Last backup cancelled {}", at.to_lowercase_first()),
                        "warning",
                    ),
                    _ => (
                        "dialog-error-symbolic",
                        format!("Last backup failed {}", at.to_lowercase_first()),
                        "danger",
                    ),
                }
            }
        };
        card.status_icon.set_icon_name(Some(icon));
        for c in ["ok", "warning", "danger", "pending"] {
            card.status_icon.remove_css_class(c);
        }
        card.status_icon.add_css_class(class);
        card.status.set_text(&text);
        card.status.set_tooltip_text(
            info.as_ref()
                .and_then(|i| i.last_backup.as_ref())
                .filter(|j| j.state != JobState::Success)
                .map(|j| j.message.as_str()),
        );
        let p = config.profile(&card.profile).ok();
        let schedule = match (
            timer_next(ui, &card.profile),
            p.and_then(|p| p.schedule.clone()),
        ) {
            (Some(next), calendar) if next > 0 => format!(
                "{} · next {}",
                schedule_name(calendar.as_deref().unwrap_or("Scheduled")),
                when(next).to_lowercase_first()
            ),
            (Some(_), calendar) => schedule_name(calendar.as_deref().unwrap_or("Scheduled")),
            (None, Some(_)) => "Schedule off".into(),
            (None, None) => "Manual".into(),
        };
        card.schedule.set_text(&schedule);
        match active.as_ref().filter(|j| j.profile == card.profile) {
            Some(job) => {
                card.progress_box.set_visible(true);
                card.run.set_sensitive(false);
                let (fraction, detail) = progress(job);
                match fraction {
                    Some(f) => card.progress.set_fraction(f),
                    None => card.progress.pulse(),
                }
                card.progress_text.set_text(&detail);
            }
            None => {
                card.progress_box.set_visible(false);
                card.run.set_sensitive(true);
            }
        }
    }
    // The line above the cards: what's happening across every profile.
    let any_profiles = !config.profiles.is_empty();
    // The setup checklist already says nothing has been backed up yet.
    v.summary
        .set_visible(any_profiles && health.is_some() && v.setup.first_child().is_none());
    let Some(h) = health else { return };
    for c in ["ok", "warning", "danger", "accent"] {
        v.summary_icon.remove_css_class(c);
    }
    let attention = h
        .profiles
        .iter()
        .filter(|p| config.profile(&p.profile).is_ok())
        .find(|p| {
            p.last_backup
                .as_ref()
                .is_some_and(|j| j.state != JobState::Success)
        });
    let (icon, class, title) = if let Some(j) = &h.active {
        (
            "view-refresh-symbolic",
            "accent",
            format!(
                "{} running · {}",
                operation_name(&j.operation),
                profile_name(&j.profile)
            ),
        )
    } else if let Some(p) = attention {
        (
            "dialog-warning-symbolic",
            "warning",
            format!("{} needs attention", profile_name(&p.profile)),
        )
    } else if let Some(last) = h.profiles.iter().filter_map(|p| p.last_success).max() {
        (
            "object-select-symbolic",
            "ok",
            format!(
                "Protected · last backup {}",
                when(last).to_lowercase_first()
            ),
        )
    } else {
        ("radio-symbolic", "pending", "Not backed up yet".to_string())
    };
    v.summary_icon.set_icon_name(Some(icon));
    v.summary_icon.add_css_class(class);
    v.summary_title.set_text(&title);
    let next = h
        .next_schedule
        .iter()
        .filter_map(|t| t["next"].as_u64())
        .filter(|n| *n != 0 && *n != u64::MAX)
        .min();
    v.summary_note.set_text(&match next {
        Some(n) => format!("Next {}", when(n / 1_000_000).to_lowercase_first()),
        None => String::new(),
    });
}

trait LowerFirst {
    fn to_lowercase_first(&self) -> String;
}
impl LowerFirst for String {
    /// "Today 09:12" → "today 09:12", for use mid-sentence.
    fn to_lowercase_first(&self) -> String {
        let mut c = self.chars();
        match c.next() {
            Some(f)
                if self.starts_with("Today")
                    || self.starts_with("Yesterday")
                    || self.starts_with("Tomorrow") =>
            {
                f.to_lowercase().collect::<String>() + c.as_str()
            }
            _ => self.clone(),
        }
    }
}

/// A job's progress as a fraction (if known) and a line of text.
pub fn progress(job: &Job) -> (Option<f64>, String) {
    let p = &job.progress;
    let stage = p["stage"].as_str().unwrap_or("");
    if let Some(f) = p["percent_done"].as_f64() {
        let mut parts = vec![format!("{:.0}%", f * 100.0)];
        if let (Some(done), Some(total)) = (p["bytes_done"].as_u64(), p["total_bytes"].as_u64()) {
            parts.push(format!("{} of {}", size(done), size(total)));
        }
        if let (Some(done), Some(total)) = (p["files_done"].as_u64(), p["total_files"].as_u64()) {
            parts.push(format!(
                "{} of {}",
                number(done),
                count(total, "file", "files")
            ));
        }
        return (Some(f.clamp(0.0, 1.0)), parts.join(" · "));
    }
    let text = match stage {
        "preparing" => "Preparing…".to_string(),
        "staging" => "Decrypting the selected files…".to_string(),
        _ => format!("{}…", operation_name(&job.operation)),
    };
    (None, text)
}

fn checklist(ui: &Rc<Ui>, c: &Config) {
    let v = &ui.backups;
    clear(&v.setup);
    let jobs = Store::default().jobs().unwrap_or_default();
    let backed_up = jobs
        .iter()
        .any(|j| j.operation == "backup" && j.state == JobState::Success);
    if backed_up {
        return;
    }
    let card = w::vbox(4);
    card.add_css_class("card");
    card.add_css_class("setup-card");
    let head = w::vbox(2);
    head.append(&line("Set up your first backup", "card-title"));
    head.append(&label(
        "Encrypted snapshots that standard Restic tools can read. Nothing runs until you start it.",
        "card-subtitle",
    ));
    head.set_margin_bottom(6);
    card.append(&head);
    let destination = c.destinations.first().cloned();
    let profile = c.profiles.first().cloned();
    let repository_ready = |d: &Destination| {
        jobs.iter().any(|j| {
            ["validate", "init"].contains(&j.operation.as_str())
                && j.state == JobState::Success
                && c.profile(&j.profile).is_ok_and(|p| p.destination == d.id)
        })
    };
    let step =
        |done: bool, enabled: bool, title: &str, desc: &str, button: &str, act: Box<dyn Fn()>| {
            let row = w::hbox(12);
            row.add_css_class("setup-step");
            let mark = gtk::Image::from_icon_name(if done {
                "object-select-symbolic"
            } else {
                "radio-symbolic"
            });
            mark.add_css_class(if done { "ok" } else { "pending" });
            mark.set_valign(gtk::Align::Center);
            row.append(&mark);
            let text = w::vbox(1);
            text.set_hexpand(true);
            text.append(&line(
                title,
                if done {
                    "step-title done"
                } else {
                    "step-title"
                },
            ));
            text.append(&label(desc, "step-desc"));
            row.append(&text);
            if !done {
                let b = gtk::Button::with_label(button);
                b.set_valign(gtk::Align::Center);
                b.set_sensitive(enabled);
                b.connect_clicked(move |_| act());
                row.append(&b);
            }
            card.append(&row);
        };
    let u = ui.clone();
    step(
        destination.is_some(),
        true,
        "Choose where backups go",
        "A drive, a folder, a NAS, an SFTP server or S3 storage.",
        "Add destination",
        Box::new(move || crate::destinations::edit(&u, None)),
    );
    let password_row = w::vbox(0);
    card.append(&password_row);
    let u = ui.clone();
    step(
        profile.is_some(),
        destination.is_some(),
        "Pick what to back up",
        "Folders to protect, how many snapshots to keep and when to run.",
        "Choose folders",
        Box::new(move || edit(&u, None)),
    );
    let (d2, p2) = (destination.clone(), profile.clone());
    let u = ui.clone();
    let ready = destination.as_ref().is_some_and(repository_ready);
    step(
        ready,
        destination.is_some() && profile.is_some(),
        "Connect the repository",
        "Check an existing repository, or create a new one at the destination.",
        "Connect",
        Box::new(move || {
            if let (Some(d), Some(_)) = (&d2, &p2) {
                crate::destinations::connect(&u, d);
            }
        }),
    );
    let u = ui.clone();
    let p3 = profile.clone();
    step(
        false,
        ready,
        "Run the first backup",
        "Later backups only send what changed.",
        "Back up now",
        Box::new(move || {
            if let Some(p) = &p3 {
                back_up(&u, p);
            }
        }),
    );
    v.setup.append(&card);
    // Whether a password is saved needs Secret Service; ask off the GTK thread.
    if let Some(d) = destination {
        let u = ui.clone();
        let id = d.id.clone();
        attempt(
            move || backup_system::has_secret(&id, "repository"),
            move |saved| {
                let saved = saved.unwrap_or(false);
                let row = w::hbox(12);
                row.add_css_class("setup-step");
                let mark = gtk::Image::from_icon_name(if saved {
                    "object-select-symbolic"
                } else {
                    "radio-symbolic"
                });
                mark.add_css_class(if saved { "ok" } else { "pending" });
                row.append(&mark);
                let text = w::vbox(1);
                text.set_hexpand(true);
                text.append(&line(
                    "Save the repository password",
                    if saved {
                        "step-title done"
                    } else {
                        "step-title"
                    },
                ));
                text.append(&label(
                    "Kept in your keyring. Without it the backups can't be read.",
                    "step-desc",
                ));
                row.append(&text);
                if !saved {
                    let b = gtk::Button::with_label("Set password");
                    b.set_valign(gtk::Align::Center);
                    let ui = u.clone();
                    b.connect_clicked(move |_| crate::destinations::password(&ui, &d));
                    row.append(&b);
                }
                password_row.append(&row);
            },
        );
    }
}

/// Start a backup, asking first if the profile hasn't been allowed to run.
pub fn back_up(ui: &Rc<Ui>, p: &Profile) {
    if p.approved {
        run(ui, p, "backup");
        return;
    }
    let sheet = Sheet::new("Allow this backup to run?", 440);
    sheet.body.append(&label(
        &format!(
            "“{}” hasn't been allowed to run yet. Allowing it lets you, its schedule and apps you grant access start it.",
            p.name
        ),
        "",
    ));
    sheet.close_button("Not now");
    let b = sheet.action("Allow and back up", true);
    let s = sheet.clone();
    let u = ui.clone();
    let id = p.id.clone();
    b.connect_clicked(move |_| {
        let id = id.clone();
        s.submit(
            &u,
            move || {
                let store = Store::default();
                let mut c = store.load()?;
                let p = c
                    .profiles
                    .iter_mut()
                    .find(|p| p.id == id)
                    .ok_or_else(|| anyhow::anyhow!("This backup no longer exists"))?;
                p.approved = true;
                let p = p.clone();
                store.save(&c)?;
                Ok(p)
            },
            |ui, p| {
                refresh(ui);
                run(ui, &p, "backup");
            },
        );
    });
    sheet.present();
}

/// Start a background-service job for a profile and report how it went.
pub fn run(ui: &Rc<Ui>, p: &Profile, op: &'static str) {
    let id = p.id.clone();
    let name = p.name.clone();
    work(
        ui,
        move || Ok(backup_service::start(&id, op)?),
        move |ui, job| {
            toast(
                ui,
                &match op {
                    "backup" => format!("Backing up {name}. Closing the window won't stop it."),
                    "verify" => "Checking the repository…".into(),
                    "verify-full" => "Reading all repository data. This can take a while.".into(),
                    "stats" => "Measuring storage use…".into(),
                    _ => format!("{}…", operation_name(op)),
                },
            );
            if op == "backup" {
                crate::status::poll_soon(ui);
                return;
            }
            track(ui, job, move |ui, j| result(ui, &j));
        },
    );
}

/// Show what a finished check or measurement found.
pub fn result(ui: &Rc<Ui>, j: &serde_json::Value) {
    let op = j["operation"].as_str().unwrap_or("");
    if j["state"] != "success" {
        error(
            ui,
            &format!("{} needs attention", operation_name(op)),
            j["message"]
                .as_str()
                .unwrap_or("The operation didn't finish."),
        );
        return;
    }
    let r = &j["result"];
    match op {
        "verify" | "verify-full" => {
            let errors = r["num_errors"].as_u64().unwrap_or(0);
            if errors == 0 {
                toast(ui, "The repository check found no problems");
            } else {
                error(
                    ui,
                    "The repository check found problems",
                    &format!(
                        "{} reported. Don't delete anything; run Restic's check yourself to see the details.",
                        count(errors, "problem", "problems")
                    ),
                );
            }
        }
        "stats" => {
            let sheet = Sheet::new("Storage use", 400);
            let (flow, values) =
                w::kv_flow(&["Stored", "Before compression", "Snapshots", "Blobs"]);
            values[0].set_text(&r["total_size"].as_u64().map(size).unwrap_or("–".into()));
            values[1].set_text(
                &r["total_uncompressed_size"]
                    .as_u64()
                    .map(size)
                    .unwrap_or("–".into()),
            );
            values[2].set_text(
                &r["snapshots_count"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or("–".into()),
            );
            values[3].set_text(
                &r["total_blob_count"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .unwrap_or("–".into()),
            );
            sheet.body.append(&flow);
            sheet.body.append(&label(
                "The whole repository, including other profiles that share it.",
                "field-hint",
            ));
            sheet.close_button("Close");
            sheet.present();
        }
        _ => toast(ui, &format!("{} finished", operation_name(op))),
    }
}

fn cancel_active(ui: &Rc<Ui>, profile: &str) {
    let job = ui
        .health
        .borrow()
        .as_ref()
        .and_then(|h| h.active.clone())
        .filter(|j| j.profile == profile);
    let Some(job) = job else { return };
    work(
        ui,
        move || Ok(backup_service::cancel(&job.id)?),
        |ui, _| {
            toast(ui, "Stopping… Completed data is kept for the next run.");
            crate::status::poll_soon(ui);
        },
    );
}

fn preview(ui: &Rc<Ui>, p: &Profile) {
    let id = p.id.clone();
    toast(ui, "Looking through the selected folders…");
    let profile = p.clone();
    work(
        ui,
        move || {
            let s = backup_service::client_call("Preview", (id.as_str(),))?;
            Ok(serde_json::from_str::<Preview>(&s)?)
        },
        move |ui, pv| {
            let sheet = Sheet::new(&format!("What “{}” includes", profile.name), 520);
            let (flow, values) = w::kv_flow(&["Files", "Size", "Unreadable"]);
            values[0].set_text(&count(pv.files, "file", "files"));
            values[1].set_text(&size(pv.logical_bytes));
            values[2].set_text(&pv.unreadable.to_string());
            sheet.body.append(&flow);
            sheet.body.append(&line("Folders", "field-title"));
            for s in &pv.sources {
                sheet
                    .body
                    .append(&line(&nyx_core::paths::pretty(s), "path-line"));
            }
            if !pv.blocked.is_empty() {
                sheet.body.append(&line("Can't back up yet", "field-title"));
                for b in &pv.blocked {
                    sheet.body.append(&label(b, "danger-text"));
                }
            }
            sheet.body.append(&line("Notes", "field-title"));
            for warning in &pv.warnings {
                sheet.body.append(&label(warning, "field-hint"));
            }
            sheet.close_button("Close");
            if pv.blocked.is_empty() {
                let b = sheet.action("Back up now", true);
                let win = sheet.clone();
                let ui = ui.clone();
                b.connect_clicked(move |_| {
                    win.close();
                    back_up(&ui, &profile);
                });
            }
            sheet.present();
        },
    );
}

fn retention(ui: &Rc<Ui>, p: &Profile) {
    let id = p.id.clone();
    let profile = p.clone();
    toast(ui, "Working out which snapshots would be removed…");
    work(
        ui,
        move || backup_service::admin_start(&id, "retention-preview", None),
        move |ui, job| {
            track(ui, job, move |ui, j| {
                if j["state"] != "success" {
                    result(ui, &j);
                    return;
                }
                retention_review(ui, &profile, &j["result"]);
            })
        },
    );
}

fn retention_review(ui: &Rc<Ui>, p: &Profile, r: &serde_json::Value) {
    let groups = r.as_array().cloned().unwrap_or_default();
    let remove: Vec<serde_json::Value> = groups
        .iter()
        .flat_map(|g| g["remove"].as_array().cloned().unwrap_or_default())
        .collect();
    let keep: usize = groups
        .iter()
        .map(|g| g["keep"].as_array().map_or(0, |k| k.len()))
        .sum();
    let sheet = Sheet::new("Clean up old snapshots", 480);
    if remove.is_empty() {
        sheet.body.append(&label(
            &format!(
                "Nothing to remove. This computer has {} for “{}”, within the {} it keeps.",
                count(keep as u64, "snapshot", "snapshots"),
                p.name,
                p.keep_last
            ),
            "",
        ));
        sheet.close_button("Close");
        sheet.present();
        return;
    }
    sheet.body.append(&label(
        &format!(
            "Keeps the newest {} of “{}” from this computer and removes {}. Removed snapshots can't be restored. Repository space is freed later by Restic's prune.",
            p.keep_last,
            p.name,
            count(remove.len() as u64, "older snapshot", "older snapshots"),
        ),
        "",
    ));
    let list = w::vbox(2);
    list.add_css_class("compact-list");
    for s in remove.iter().take(50) {
        list.append(&line(
            &format!(
                "{} · {}",
                when_text(s["time"].as_str().unwrap_or("")),
                s["short_id"].as_str().unwrap_or("")
            ),
            "path-line",
        ));
    }
    sheet.body.append(&list);
    let confirm = forms::entry(&sheet.body, &format!("Type {} to confirm", p.id), "", "");
    sheet.close_button("Cancel");
    let b = sheet.action(
        &format!(
            "Remove {}",
            count(remove.len() as u64, "snapshot", "snapshots")
        ),
        false,
    );
    b.add_css_class("destructive-action");
    b.set_sensitive(false);
    {
        let b = b.clone();
        let id = p.id.clone();
        confirm.connect_changed(move |e| b.set_sensitive(e.text() == id));
    }
    let s = sheet.clone();
    let u = ui.clone();
    let id = p.id.clone();
    b.connect_clicked(move |_| {
        let id = id.clone();
        s.submit(
            &u,
            move || backup_service::admin_start(&id, "retention-apply", Some(&id)),
            |ui, job| {
                toast(ui, "Removing old snapshots…");
                track(ui, job, |ui, j| {
                    if j["state"] == "success" {
                        toast(ui, "Old snapshots removed");
                    } else {
                        result(ui, &j);
                    }
                })
            },
        );
    });
    sheet.present();
}

fn set_schedule(ui: &Rc<Ui>, p: &Profile, on: bool) {
    let p = p.clone();
    work(
        ui,
        move || {
            backup_system::schedule(&Store::default(), &p, on)?;
            Ok(())
        },
        move |ui, _| {
            toast(
                ui,
                if on {
                    "Schedule turned on"
                } else {
                    "Schedule turned off"
                },
            );
            crate::status::poll_soon(ui);
            let ui = ui.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                refresh(&ui)
            });
        },
    );
}

fn delete(ui: &Rc<Ui>, p: &Profile) {
    let p = p.clone();
    let u = ui.clone();
    forms::confirm(
        &format!("Delete “{}”?", p.name),
        "Its settings and schedule are removed. Snapshots already in the repository stay there and can still be read with Restic.",
        "Delete",
        move || {
            let p = p.clone();
            work(
                &u,
                move || {
                    let _ = backup_system::schedule(&Store::default(), &p, false);
                    let store = Store::default();
                    let mut c = store.load()?;
                    c.profiles.retain(|x| x.id != p.id);
                    store.save(&c)?;
                    Ok(p.name.clone())
                },
                |ui, name| {
                    refresh(ui);
                    crate::restore_ui::refresh(ui);
                    toast(ui, &format!("Deleted “{name}”"));
                },
            );
        },
    );
}

const SCHEDULES: &[(&str, &str)] = &[
    ("off", "Off"),
    ("hourly", "Hourly"),
    ("daily", "Daily"),
    ("weekly", "Weekly"),
    ("custom", "Custom"),
];

/// Create or edit a profile.
pub fn edit(ui: &Rc<Ui>, profile: Option<Profile>) {
    let config = Store::default().load().unwrap_or_default();
    if config.destinations.is_empty() {
        toast(ui, "Add a destination first: where the backups go");
        crate::destinations::edit(ui, None);
        return;
    }
    let sheet = Sheet::new(
        if profile.is_some() {
            "Edit backup"
        } else {
            "New backup"
        },
        540,
    );
    let b = &sheet.body;
    let name = forms::entry(
        b,
        "Name",
        "",
        profile
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("Personal data"),
    );
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
    forms::field(b, "Destination", "", &dest);

    // Folders: a list of rows, each with a remove button.
    let sources: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(
        profile
            .as_ref()
            .map(|p| p.sources.clone())
            .unwrap_or_default(),
    ));
    let list = w::vbox(2);
    list.add_css_class("source-list");
    let render: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let (sources, list, render2) = (sources.clone(), list.clone(), render.clone());
        *render.borrow_mut() = Some(Box::new(move || {
            clear(&list);
            let items = sources.borrow().clone();
            if items.is_empty() {
                list.append(&label(
                    "No folders yet. Add the ones to protect.",
                    "field-hint",
                ));
            }
            for path in items {
                let row = w::hbox(8);
                row.add_css_class("source-row");
                let icon = gtk::Image::from_icon_name(if path.is_dir() {
                    "folder-symbolic"
                } else {
                    "text-x-generic-symbolic"
                });
                row.append(&icon);
                let l = line(&nyx_core::paths::pretty(&path), "path-line");
                l.set_hexpand(true);
                l.set_tooltip_text(Some(&path.display().to_string()));
                row.append(&l);
                let (sources, render) = (sources.clone(), render2.clone());
                row.append(&icon_button(
                    "list-remove-symbolic",
                    "Remove this folder",
                    move || {
                        sources.borrow_mut().retain(|p| p != &path);
                        if let Some(r) = render.borrow().as_ref() {
                            r();
                        }
                    },
                ));
                list.append(&row);
            }
        }));
    }
    let rerender = {
        let render = render.clone();
        move || {
            if let Some(r) = render.borrow().as_ref() {
                r();
            }
        }
    };
    rerender();
    let add_row = w::hbox(6);
    {
        let (sources, parent, rerender) = (sources.clone(), sheet.parent.clone(), rerender.clone());
        let add = gtk::Button::with_label("Add folder…");
        add.connect_clicked(move |_| {
            let (sources, rerender) = (sources.clone(), rerender.clone());
            forms::choose_folder(&parent, "Choose a folder to back up", move |p| {
                let mut s = sources.borrow_mut();
                if !s.contains(&p) {
                    s.push(p);
                }
                drop(s);
                rerender();
            });
        });
        add_row.append(&add);
    }
    {
        let (sources, rerender) = (sources.clone(), rerender.clone());
        let personal = gtk::Button::with_label("Personal folders");
        personal.set_tooltip_text(Some("Documents, Pictures, Music, Videos and Desktop"));
        personal.connect_clicked(move |_| {
            let inventory = backup_system::inventory();
            let mut s = sources.borrow_mut();
            for p in inventory["personal_data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
            {
                let p = PathBuf::from(p);
                if !s.contains(&p) {
                    s.push(p);
                }
            }
            drop(s);
            rerender();
        });
        add_row.append(&personal);
    }
    {
        let (sources, rerender) = (sources.clone(), rerender.clone());
        let configuration = gtk::Button::with_label("Configuration…");
        configuration.set_tooltip_text(Some("Choose app and desktop configuration folders"));
        configuration.connect_clicked(move |_| {
            let (sources, rerender) = (sources.clone(), rerender.clone());
            configuration_selection(move |chosen| {
                let mut s = sources.borrow_mut();
                for p in chosen {
                    if !s.contains(&p) {
                        s.push(p);
                    }
                }
                drop(s);
                rerender();
            });
        });
        add_row.append(&configuration);
    }
    let folders = w::vbox(6);
    folders.append(&list);
    folders.append(&add_row);
    forms::field(
        b,
        "Folders",
        "Caches, trash, live databases and virtual machines are left out automatically. Build folders are kept.",
        &folders,
    );

    let keep = gtk::SpinButton::with_range(1.0, 10000.0, 1.0);
    keep.set_value(profile.as_ref().map(|p| p.keep_last).unwrap_or(7) as f64);
    keep.set_valign(gtk::Align::Center);
    let keep_row = w::row(
        "Snapshots to keep",
        "Older ones are removed only when you clean up.",
        Some(keep.upcast_ref()),
    );
    keep_row.add_css_class("form-option");
    b.append(&keep_row);

    let current = profile.as_ref().and_then(|p| p.schedule.clone());
    let current_id = match current.as_deref() {
        None => "off",
        Some(s @ ("hourly" | "daily" | "weekly")) => s,
        Some(_) => "custom",
    };
    let custom = gtk::Entry::builder()
        .placeholder_text("Mon..Fri 18:00")
        .text(if current_id == "custom" {
            current.clone().unwrap_or_default()
        } else {
            String::new()
        })
        .build();
    custom.set_visible(current_id == "custom");
    let choice = Rc::new(RefCell::new(current_id.to_string()));
    let seg = {
        let (choice, custom) = (choice.clone(), custom.clone());
        w::segmented(&w::opts(SCHEDULES), current_id, move |id| {
            custom.set_visible(id == "custom");
            *choice.borrow_mut() = id;
        })
    };
    let sched = w::vbox(6);
    sched.append(&seg);
    sched.append(&custom);
    forms::field(
        b,
        "Schedule",
        "Runs while you're logged in. Custom takes a systemd calendar expression.",
        &sched,
    );

    let approved = forms::switch(
        b,
        "Allow this backup to run",
        "Needed for Back up now, the schedule and apps you grant access.",
        profile.as_ref().is_none_or(|p| p.approved),
    );
    let secrets = forms::switch(
        b,
        "Include keys and credentials",
        "SSH and GPG keys, password stores and browser profiles in the selected folders.",
        profile.as_ref().is_some_and(|p| p.include_secrets),
    );
    let adv = forms::advanced(b);
    let exclude = forms::text_area(
        &adv,
        "Leave out",
        "Glob patterns, one per line, such as **/node_modules.",
        &profile
            .as_ref()
            .map(|p| p.exclusions.join("\n"))
            .unwrap_or_default(),
    );
    let taken: Vec<String> = config.profiles.iter().map(|p| p.id.clone()).collect();
    let id_entry = forms::entry(
        &adv,
        "ID",
        "Used by nyx-backupctl and in snapshot tags. Can't be changed later.",
        &profile
            .as_ref()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| slug("Personal data", &taken)),
    );
    id_entry.set_sensitive(profile.is_none());
    if profile.is_none() {
        let id_entry = id_entry.clone();
        let edited = Rc::new(std::cell::Cell::new(false));
        let e2 = edited.clone();
        id_entry.connect_changed(move |e| {
            if e.has_focus() {
                e2.set(true);
            }
        });
        name.connect_changed(move |n| {
            if !edited.get() {
                id_entry.set_text(&slug(&n.text(), &taken));
            }
        });
    }
    sheet.close_button("Cancel");
    let save = sheet.action(if profile.is_some() { "Save" } else { "Create" }, true);
    forms::require(&name, &save);
    let s = sheet.clone();
    let u = ui.clone();
    let was_scheduled = profile
        .as_ref()
        .is_some_and(|p| timer_next(ui, &p.id).is_some());
    let focus = name.clone();
    save.connect_clicked(move |_| {
        let schedule = match choice.borrow().as_str() {
            "off" => None,
            "custom" => Some(custom.text().trim().to_string()).filter(|s| !s.is_empty()),
            other => Some(other.to_string()),
        };
        let Some(destination) = config.destinations.get(dest.selected() as usize) else {
            s.fail("Choose a destination");
            return;
        };
        let p = Profile {
            id: id_entry.text().trim().to_string(),
            name: name.text().trim().to_string(),
            destination: destination.id.clone(),
            sources: sources.borrow().clone(),
            exclusions: forms::text_lines(&exclude),
            include_secrets: secrets.is_active(),
            approved: approved.is_active(),
            keep_last: keep.value() as u32,
            schedule,
        };
        let new = profile.is_none();
        s.submit(
            &u,
            move || {
                let store = Store::default();
                let mut c = store.load()?;
                if new {
                    anyhow::ensure!(
                        !c.profiles.iter().any(|x| x.id == p.id),
                        "Another backup already uses the ID “{}”",
                        p.id
                    );
                }
                match c.profiles.iter_mut().find(|x| x.id == p.id) {
                    Some(old) => *old = p.clone(),
                    None => c.profiles.push(p.clone()),
                }
                store.save(&c)?;
                // A timer that is on follows the edited calendar; off stays off.
                if was_scheduled {
                    backup_system::schedule(&store, &p, p.schedule.is_some() && p.approved)?;
                }
                Ok(p)
            },
            |ui, p| {
                refresh(ui);
                crate::restore_ui::refresh(ui);
                crate::status::poll_soon(ui);
                toast(ui, &format!("Saved “{}”", p.name));
            },
        );
    });
    sheet.present();
    forms::focus_later(&focus);
}

/// Choose configuration folders from the curated list and everything else in ~/.config.
fn configuration_selection(done: impl Fn(Vec<PathBuf>) + 'static) {
    let sheet = Sheet::new("Choose configuration folders", 480);
    let v = backup_system::inventory();
    let mut checks = Vec::new();
    for (key, title, hint) in [
        (
            "curated_configuration",
            "Desktop",
            "Hyprland, Omarchy, Nyx and terminal settings.",
        ),
        (
            "other_configuration_for_explicit_selection",
            "Other apps",
            "Review these: app folders can hold tokens and caches.",
        ),
    ] {
        let list = w::vbox(2);
        for p in v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str())
        {
            let name = std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(p.to_string());
            let check = gtk::CheckButton::with_label(&name);
            check.set_tooltip_text(Some(p));
            list.append(&check);
            checks.push((check, PathBuf::from(p)));
        }
        forms::field(&sheet.body, title, hint, &list);
    }
    sheet.close_button("Cancel");
    let add = sheet.action("Add selected", true);
    let win = sheet.clone();
    add.connect_clicked(move |_| {
        done(
            checks
                .iter()
                .filter(|(c, _)| c.is_active())
                .map(|(_, p)| p.clone())
                .collect(),
        );
        win.close();
    });
    sheet.present();
}

/// The profile whose card has keyboard focus, else the first allowed one.
pub fn focused_or_first(ui: &Rc<Ui>) -> Option<Profile> {
    let c = Store::default().load().ok()?;
    let focus = gtk::prelude::GtkWindowExt::focus(&ui.window);
    for card in ui.backups.widgets.borrow().iter() {
        if focus.as_ref().is_some_and(|f| f.is_ancestor(&card.root)) {
            return c.profile(&card.profile).ok().cloned();
        }
    }
    c.profiles
        .iter()
        .find(|p| p.approved)
        .or(c.profiles.first())
        .cloned()
}

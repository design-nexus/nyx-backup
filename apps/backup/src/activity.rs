//! The Activity view: every job the background service ran, newest first,
//! with the details of each.
use crate::forms::Sheet;
use crate::ui::*;
use backup_core::*;
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::rc::Rc;

pub struct View {
    pub root: gtk::ScrolledWindow,
    list: gtk::ListBox,
    empty: gtk::Box,
}

impl View {
    pub fn new() -> View {
        let body = w::vbox(12);
        body.add_css_class("view-body");
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("job-list");
        body.append(&list);
        let empty = w::empty_state(
            "document-open-recent-symbolic",
            "No activity yet",
            "Backups, checks and restores show up here.",
            None,
        );
        empty.set_visible(false);
        body.append(&empty);
        let clamp = nyx_ui::clamp::Clamp::new(&body, 760);
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();
        View { root, list, empty }
    }
}

pub fn refresh(ui: &Rc<Ui>) {
    let u = ui.clone();
    attempt(
        || {
            let s = backup_service::client_call("Jobs", ())?;
            Ok(serde_json::from_str::<Vec<Job>>(&s)?)
        },
        move |result| {
            let v = &u.activity;
            clear(&v.list);
            let jobs = result.unwrap_or_default();
            v.empty.set_visible(jobs.is_empty());
            v.list.set_visible(!jobs.is_empty());
            let config = Store::default().load().unwrap_or_default();
            for job in jobs.into_iter().take(200) {
                v.list.append(&row(&u, &config, job));
            }
        },
    );
}

fn state_icon(state: &JobState) -> (&'static str, &'static str) {
    match state {
        JobState::Running => ("view-refresh-symbolic", "accent"),
        JobState::Success => ("object-select-symbolic", "ok"),
        JobState::Incomplete | JobState::Cancelled | JobState::Interrupted => {
            ("dialog-warning-symbolic", "warning")
        }
        JobState::Failed => ("dialog-error-symbolic", "danger"),
    }
}

fn row(ui: &Rc<Ui>, config: &Config, job: Job) -> gtk::ListBoxRow {
    let bx = w::hbox(12);
    bx.add_css_class("job-row");
    let (icon, class) = state_icon(&job.state);
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("status-icon");
    image.add_css_class(class);
    bx.append(&image);
    let text = w::vbox(1);
    text.set_hexpand(true);
    let name = config
        .profile(&job.profile)
        .map(|p| p.name.clone())
        .unwrap_or(job.profile.clone());
    text.append(&line(
        &format!("{} · {name}", operation_name(&job.operation)),
        "job-title",
    ));
    let detail = if job.state == JobState::Running {
        crate::backups::progress(&job).1
    } else {
        summary(&job)
    };
    text.append(&line(&detail, "job-detail"));
    bx.append(&text);
    let meta = w::vbox(1);
    meta.set_valign(gtk::Align::Center);
    let t = line(&when(job.started), "job-time");
    t.set_xalign(1.0);
    meta.append(&t);
    if let Some(f) = job.finished {
        let d = line(&duration(f.saturating_sub(job.started)), "job-time");
        d.set_xalign(1.0);
        meta.append(&d);
    }
    bx.append(&meta);
    if job.state == JobState::Running {
        let id = job.id.clone();
        let u = ui.clone();
        bx.append(&icon_button(
            "media-playback-stop-symbolic",
            "Cancel this job",
            move || {
                let id = id.clone();
                work(
                    &u,
                    move || Ok(backup_service::cancel(&id)?),
                    |ui, _| {
                        toast(ui, "Stopping…");
                        crate::status::poll_soon(ui);
                    },
                );
            },
        ));
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&bx));
    row.set_activatable(true);
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| details(&job));
    row.add_controller(gesture);
    row
}

/// One line on how a finished job went.
fn summary(job: &Job) -> String {
    let r = &job.result;
    if job.state == JobState::Success {
        match job.operation.as_str() {
            "backup" => {
                let mut parts = vec![];
                if let Some(n) = r["total_files_processed"].as_u64() {
                    parts.push(count(n, "file", "files"));
                }
                if let Some(n) = r["data_added"].as_u64() {
                    parts.push(format!("{} added", size(n)));
                }
                if parts.is_empty() {
                    "Completed".into()
                } else {
                    parts.join(" · ")
                }
            }
            "verify" | "verify-full" => match r["num_errors"].as_u64() {
                Some(0) | None => "No problems found".into(),
                Some(n) => count(n, "problem", "problems"),
            },
            "stats" => r["total_size"]
                .as_u64()
                .map(|n| format!("{} stored", size(n)))
                .unwrap_or("Completed".into()),
            "history" => r
                .as_array()
                .map(|a| count(a.len() as u64, "snapshot", "snapshots"))
                .unwrap_or("Completed".into()),
            _ => "Completed".into(),
        }
    } else {
        job.message.clone()
    }
}

fn details(job: &Job) {
    let sheet = Sheet::new(
        &format!(
            "{} · {}",
            operation_name(&job.operation),
            profile_name(&job.profile)
        ),
        480,
    );
    let state = match job.state {
        JobState::Running => "Running",
        JobState::Success => "Completed",
        JobState::Incomplete => "Incomplete",
        JobState::Failed => "Failed",
        JobState::Cancelled => "Cancelled",
        JobState::Interrupted => "Interrupted",
    };
    let (flow, values) = w::kv_flow(&["Result", "Started", "Took"]);
    values[0].set_text(state);
    values[1].set_text(&when(job.started));
    values[2].set_text(
        &job.finished
            .map(|f| duration(f.saturating_sub(job.started)))
            .unwrap_or("–".into()),
    );
    sheet.body.append(&flow);
    if !job.message.is_empty() && job.message != "Completed" {
        sheet.body.append(&label(&job.message, ""));
    }
    let r = &job.result;
    let facts: Vec<(&str, String)> = match job.operation.as_str() {
        "backup" => [
            ("New files", r["files_new"].as_u64().map(|n| n.to_string())),
            (
                "Changed files",
                r["files_changed"].as_u64().map(|n| n.to_string()),
            ),
            (
                "Unchanged files",
                r["files_unmodified"].as_u64().map(|n| n.to_string()),
            ),
            ("Data added", r["data_added"].as_u64().map(size)),
            ("Total size", r["total_bytes_processed"].as_u64().map(size)),
            (
                "Snapshot",
                r["snapshot_id"]
                    .as_str()
                    .map(|s| s.chars().take(8).collect()),
            ),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect(),
        "stats" => [
            ("Stored", r["total_size"].as_u64().map(size)),
            (
                "Before compression",
                r["total_uncompressed_size"].as_u64().map(size),
            ),
            (
                "Snapshots",
                r["snapshots_count"].as_u64().map(|n| n.to_string()),
            ),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect(),
        "verify" | "verify-full" => r["num_errors"]
            .as_u64()
            .map(|n| vec![("Problems found", n.to_string())])
            .unwrap_or_default(),
        "restore-plan" | "restore-apply" | "restore-rollback" => [
            ("Files", r["files"].as_u64().map(|n| n.to_string())),
            ("Installed", r["installed"].as_u64().map(|n| n.to_string())),
            ("Skipped", r["skipped"].as_u64().map(|n| n.to_string())),
            ("Conflicts", r["conflicts"].as_u64().map(|n| n.to_string())),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect(),
        _ => vec![],
    };
    if !facts.is_empty() {
        let grid = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(4)
            .build();
        grid.add_css_class("facts");
        for (i, (k, v)) in facts.iter().enumerate() {
            grid.attach(&line(k, "field-hint"), 0, i as i32, 1, 1);
            grid.attach(&line(v, ""), 1, i as i32, 1, 1);
        }
        sheet.body.append(&grid);
    }
    if job.operation == "history" {
        for s in r.as_array().into_iter().flatten().take(100) {
            sheet.body.append(&line(
                &format!(
                    "{} · {}",
                    when_text(s["time"].as_str().unwrap_or("")),
                    s["short_id"].as_str().unwrap_or("")
                ),
                "path-line",
            ));
        }
    }
    sheet.close_button("Close");
    sheet.present();
}

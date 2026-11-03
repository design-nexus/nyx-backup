//! The Restore view: a profile's snapshots, a browser to pick files from one,
//! then a reviewed plan, its receipt and rollback. Everything runs on the
//! private local channel; nothing is written until a reviewed plan is applied.
use crate::forms::{self, Sheet};
use crate::ui::*;
use backup_core::*;
use backup_restore::{Mode, Node, Plan, Policy, Request};
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Default)]
struct State {
    profiles: Vec<Profile>,
    profile: Option<String>,
    snapshot: Option<serde_json::Value>,
    directory: String,
    selected: BTreeSet<String>,
    /// Opened for settings files: the plan defaults to their original places.
    dotfiles: bool,
}

pub struct View {
    pub root: gtk::Box,
    profile: gtk::DropDown,
    title: gtk::Label,
    stack: gtk::Stack,
    snapshots: gtk::ListBox,
    crumbs: gtk::Box,
    entries: gtk::ListBox,
    note: gtk::Label,
    count: gtk::Label,
    restore: gtk::Button,
    clear: gtk::Button,
    toolbar: gtk::Box,
    state: RefCell<State>,
}

impl View {
    pub fn new() -> View {
        let root = w::vbox(0);
        let toolbar = w::hbox(8);
        toolbar.add_css_class("view-toolbar");
        let title = line("", "toolbar-title");
        title.set_hexpand(true);
        toolbar.append(&title);
        let profile = gtk::DropDown::from_strings(&[]);
        profile.set_tooltip_text(Some("Backup to restore from"));
        toolbar.append(&profile);
        root.append(&toolbar);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(if nyx_theme::app_prefs().reduce_motion {
            0
        } else {
            120
        });
        stack.add_named(
            &w::empty_state(
                "edit-undo-symbolic",
                "Nothing to restore yet",
                "Snapshots appear here after the first backup.",
                None,
            ),
            Some("empty"),
        );
        let loading = w::vbox(8);
        loading.set_valign(gtk::Align::Center);
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_size_request(24, 24);
        loading.append(&spinner);
        let l = line("Reading the repository…", "dim");
        l.set_halign(gtk::Align::Center);
        loading.append(&l);
        stack.add_named(&loading, Some("loading"));

        let snapshots = gtk::ListBox::new();
        snapshots.set_selection_mode(gtk::SelectionMode::None);
        snapshots.add_css_class("snapshot-list");
        let snap_body = w::vbox(0);
        snap_body.add_css_class("view-body");
        snap_body.append(&snapshots);
        stack.add_named(&scroller(&snap_body), Some("snapshots"));

        let browse = w::vbox(0);
        let crumbs = w::hbox(2);
        crumbs.add_css_class("crumbs");
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .child(&crumbs)
            .build();
        browse.append(&crumb_scroll);
        let entries = gtk::ListBox::new();
        entries.set_selection_mode(gtk::SelectionMode::None);
        entries.add_css_class("file-list");
        let entries_body = w::vbox(6);
        entries_body.add_css_class("view-body");
        let note = label("", "field-hint");
        note.set_visible(false);
        entries_body.append(&note);
        entries_body.append(&entries);
        browse.append(&scroller(&entries_body));
        let bar = w::hbox(8);
        bar.add_css_class("selection-bar");
        let count = line("Select files or folders to restore", "dim");
        count.set_hexpand(true);
        bar.append(&count);
        let clear_button = gtk::Button::with_label("Clear");
        clear_button.set_sensitive(false);
        bar.append(&clear_button);
        let restore = gtk::Button::with_label("Restore…");
        restore.add_css_class("suggested-action");
        restore.set_sensitive(false);
        bar.append(&restore);
        browse.append(&bar);
        stack.add_named(&browse, Some("browse"));
        root.append(&stack);
        View {
            root,
            profile,
            title,
            stack,
            snapshots,
            crumbs,
            entries,
            note,
            count,
            restore,
            clear: clear_button,
            toolbar,
            state: RefCell::new(State::default()),
        }
    }
}

fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&nyx_ui::clamp::Clamp::new(child, 760))
        .vexpand(true)
        .build()
}

/// Connect the view's controls; called once the window's state exists.
pub fn setup(ui: &Rc<Ui>) {
    let v = &ui.restore;
    let u = ui.clone();
    v.profile.connect_selected_notify(move |d| {
        let id = u
            .restore
            .state
            .borrow()
            .profiles
            .get(d.selected() as usize)
            .map(|p| p.id.clone());
        if id.is_some() && id != u.restore.state.borrow().profile {
            u.restore.state.borrow_mut().profile = id;
            load_snapshots(&u);
        }
    });
    let u = ui.clone();
    v.clear.connect_clicked(move |_| {
        u.restore.state.borrow_mut().selected.clear();
        render_selection(&u);
        let dir = u.restore.state.borrow().directory.clone();
        browse(&u, dir);
    });
    let u = ui.clone();
    v.restore.connect_clicked(move |_| options(&u));
    let item = |f: fn(&Rc<Ui>)| -> Box<dyn Fn()> {
        let ui = ui.clone();
        Box::new(move || f(&ui))
    };
    let m = menu(
        "More restore actions",
        vec![
            ("Restore settings files…", false, item(dotfiles)),
            (
                "Restore receipts and rollback…",
                false,
                item(|ui| {
                    if let Some(p) = current(ui) {
                        receipts(ui, p);
                    }
                }),
            ),
            (
                "Migration bundles…",
                false,
                item(|ui| {
                    if let Some(p) = current(ui) {
                        crate::migration_ui::bundles(ui, p);
                    }
                }),
            ),
            ("-", false, Box::new(|| {})),
            (
                "This computer's inventory",
                false,
                item(crate::migration_ui::inventory),
            ),
            (
                "Offline disk recovery…",
                false,
                item(crate::recovery_ui::show),
            ),
        ],
    );
    v.toolbar.append(&m);
}

fn current(ui: &Rc<Ui>) -> Option<String> {
    ui.restore.state.borrow().profile.clone()
}

/// Refresh the profile list after the configuration changed.
pub fn refresh(ui: &Rc<Ui>) {
    let v = &ui.restore;
    let profiles = Store::default().load().unwrap_or_default().profiles;
    let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    let model = gtk::StringList::new(&names);
    let keep = v.state.borrow().profile.clone();
    let index = keep
        .as_ref()
        .and_then(|k| profiles.iter().position(|p| &p.id == k));
    {
        let mut s = v.state.borrow_mut();
        s.profiles = profiles.clone();
        s.profile = index
            .and_then(|i| profiles.get(i))
            .or(profiles.first())
            .map(|p| p.id.clone());
    }
    v.profile.set_model(Some(&model));
    v.profile.set_selected(index.unwrap_or(0) as u32);
    v.profile.set_visible(profiles.len() > 1);
    if profiles.is_empty() {
        v.title.set_text("");
        v.stack.set_visible_child_name("empty");
    }
}

/// The Restore view was opened: show the current profile's snapshots.
pub fn shown(ui: &Rc<Ui>) {
    let v = &ui.restore;
    if v.state.borrow().profile.is_none() {
        refresh(ui);
    }
    let name = v.stack.visible_child_name().unwrap_or_default();
    if name == "empty" || name == "snapshots" || v.snapshots.first_child().is_none() {
        load_snapshots(ui);
    }
}

pub fn open_profile(ui: &Rc<Ui>, id: &str) {
    refresh(ui);
    let index = ui
        .restore
        .state
        .borrow()
        .profiles
        .iter()
        .position(|p| p.id == id);
    if let Some(i) = index {
        ui.restore.state.borrow_mut().profile = Some(id.to_string());
        ui.restore.profile.set_selected(i as u32);
    }
    ui.show("restore");
    load_snapshots(ui);
}

fn load_snapshots(ui: &Rc<Ui>) {
    let v = &ui.restore;
    let Some(profile) = current(ui) else {
        v.stack.set_visible_child_name("empty");
        return;
    };
    v.title.set_text("Snapshots");
    v.stack.set_visible_child_name("loading");
    let u = ui.clone();
    let p = profile.clone();
    attempt(
        move || backup_service::restore_read(&p, Request::Snapshots),
        move |result| {
            let v = &u.restore;
            // The user may have opened a snapshot meanwhile, or picked another backup.
            if current(&u).as_deref() != Some(profile.as_str())
                || v.stack.visible_child_name().as_deref() != Some("loading")
            {
                return;
            }
            clear(&v.snapshots);
            let mut items = match result {
                Ok(value) => value.as_array().cloned().unwrap_or_default(),
                Err(e) => {
                    v.stack.set_visible_child_name("snapshots");
                    v.snapshots.append(&message_row(
                        "dialog-warning-symbolic",
                        "The repository can't be read",
                        &e.to_string(),
                    ));
                    return;
                }
            };
            items.sort_by(|a, b| b["time"].as_str().cmp(&a["time"].as_str()));
            if items.is_empty() {
                v.stack.set_visible_child_name("empty");
                return;
            }
            v.title
                .set_text(&count(items.len() as u64, "snapshot", "snapshots"));
            for s in items {
                v.snapshots.append(&snapshot_row(&u, s));
            }
            v.stack.set_visible_child_name("snapshots");
        },
    );
}

fn message_row(icon: &str, title: &str, desc: &str) -> gtk::ListBoxRow {
    let bx = w::hbox(12);
    bx.add_css_class("job-row");
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("warning");
    bx.append(&i);
    let t = w::vbox(2);
    t.append(&line(title, "job-title"));
    t.append(&label(desc, "job-detail"));
    bx.append(&t);
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&bx));
    row.set_activatable(false);
    row
}

fn snapshot_row(ui: &Rc<Ui>, s: serde_json::Value) -> gtk::ListBoxRow {
    let bx = w::hbox(12);
    bx.add_css_class("job-row");
    let icon = gtk::Image::from_icon_name("document-open-recent-symbolic");
    icon.add_css_class("status-icon");
    bx.append(&icon);
    let text = w::vbox(1);
    text.set_hexpand(true);
    text.append(&line(
        &when_text(s["time"].as_str().unwrap_or("")),
        "job-title",
    ));
    let mut detail = vec![s["hostname"].as_str().unwrap_or("").to_string()];
    let sum = &s["summary"];
    if let Some(n) = sum["total_files_processed"].as_u64() {
        detail.push(count(n, "file", "files"));
    }
    if let Some(n) = sum["total_bytes_processed"].as_u64() {
        detail.push(size(n));
    }
    detail.retain(|d| !d.is_empty());
    text.append(&line(&detail.join(" · "), "job-detail"));
    bx.append(&text);
    let id = line(s["short_id"].as_str().unwrap_or(""), "job-time");
    id.add_css_class("mono");
    bx.append(&id);
    bx.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&bx));
    let u = ui.clone();
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| open_snapshot(&u, s.clone(), None));
    row.add_controller(gesture);
    row
}

/// Where browsing a snapshot starts: the folder its sources share.
fn start_directory(s: &serde_json::Value) -> String {
    let paths: Vec<PathBuf> = s["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(PathBuf::from))
        // Every snapshot also holds Backup's own system metadata file.
        .filter(|p| !p.to_string_lossy().contains("/nyx-backup/metadata/"))
        .collect();
    let Some(first) = paths.first() else {
        return "/".into();
    };
    let mut common = if paths.len() == 1 {
        first.parent().unwrap_or(Path::new("/")).to_path_buf()
    } else {
        first.clone()
    };
    for p in &paths {
        while !p.starts_with(&common) {
            if !common.pop() {
                return "/".into();
            }
        }
    }
    common.display().to_string()
}

fn open_snapshot(ui: &Rc<Ui>, s: serde_json::Value, directory: Option<String>) {
    let dir = directory.unwrap_or_else(|| start_directory(&s));
    {
        let mut st = ui.restore.state.borrow_mut();
        st.snapshot = Some(s);
        st.selected.clear();
    }
    render_selection(ui);
    browse(ui, dir);
}

fn browse(ui: &Rc<Ui>, directory: String) {
    let v = &ui.restore;
    let (Some(profile), Some(snapshot)) = (
        current(ui),
        v.state
            .borrow()
            .snapshot
            .as_ref()
            .and_then(|s| s["id"].as_str().map(String::from)),
    ) else {
        return;
    };
    v.state.borrow_mut().directory = directory.clone();
    let time = v
        .state
        .borrow()
        .snapshot
        .as_ref()
        .map(|s| when_text(s["time"].as_str().unwrap_or("")))
        .unwrap_or_default();
    let time = match time.split_once(' ') {
        Some((day @ ("Today" | "Yesterday"), rest)) => format!("{} {rest}", day.to_lowercase()),
        _ => time,
    };
    v.title.set_text(&format!("Snapshot from {time}"));
    render_crumbs(ui, &directory);
    v.stack.set_visible_child_name("browse");
    clear(&v.entries);
    v.note.set_visible(false);
    let u = ui.clone();
    let dir = directory.clone();
    attempt(
        move || {
            backup_service::restore_read(
                &profile,
                Request::Browse {
                    snapshot,
                    directory: dir,
                },
            )
        },
        move |result| {
            let v = &u.restore;
            if v.state.borrow().directory != directory {
                return;
            }
            clear(&v.entries);
            let value = match result {
                Ok(value) => value,
                Err(e) => {
                    v.entries.append(&message_row(
                        "dialog-warning-symbolic",
                        "This folder can't be read",
                        &e.to_string(),
                    ));
                    return;
                }
            };
            if value["truncated"] == true {
                v.note
                    .set_text("Showing the first 2,000 entries. Open a subfolder to see the rest.");
                v.note.set_visible(true);
            }
            let mut nodes: Vec<Node> =
                serde_json::from_value(value["entries"].clone()).unwrap_or_default();
            nodes.sort_by(|a, b| {
                (b.kind == "dir")
                    .cmp(&(a.kind == "dir"))
                    .then(a.path.cmp(&b.path))
            });
            if nodes.is_empty() {
                v.entries.append(&message_row(
                    "folder-symbolic",
                    "This folder is empty",
                    "Nothing was backed up here.",
                ));
            }
            for node in nodes {
                v.entries.append(&entry_row(&u, node));
            }
        },
    );
}

fn render_crumbs(ui: &Rc<Ui>, directory: &str) {
    let v = &ui.restore;
    clear(&v.crumbs);
    let u = ui.clone();
    let back = icon_button("go-previous-symbolic", "Back to snapshots", move || {
        u.restore.state.borrow_mut().snapshot = None;
        load_snapshots(&u);
    });
    v.crumbs.append(&back);
    let mut path = PathBuf::from("/");
    let mut parts: Vec<(String, String)> = vec![("/".into(), "/".into())];
    for c in Path::new(directory).components().skip(1) {
        path.push(c);
        parts.push((
            c.as_os_str().to_string_lossy().to_string(),
            path.display().to_string(),
        ));
    }
    let last = parts.len() - 1;
    for (i, (name, target)) in parts.into_iter().enumerate() {
        if i > 1 {
            v.crumbs.append(&line("›", "crumb-sep"));
        }
        let b = gtk::Button::with_label(if i == 0 { "Computer" } else { &name });
        b.add_css_class("flat");
        b.add_css_class("crumb");
        if i == last {
            b.add_css_class("last");
        }
        let u = ui.clone();
        b.connect_clicked(move |_| browse(&u, target.clone()));
        v.crumbs.append(&b);
    }
}

fn entry_row(ui: &Rc<Ui>, node: Node) -> gtk::ListBoxRow {
    let bx = w::hbox(10);
    bx.add_css_class("file-row");
    let check = gtk::CheckButton::new();
    check.set_active(ui.restore.state.borrow().selected.contains(&node.path));
    check.set_tooltip_text(Some("Select for restoring"));
    bx.append(&check);
    let dir = node.kind == "dir";
    let icon = gtk::Image::from_icon_name(match node.kind.as_str() {
        "dir" => "folder-symbolic",
        "symlink" => "insert-link-symbolic",
        _ => "text-x-generic-symbolic",
    });
    bx.append(&icon);
    let name = Path::new(&node.path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or(node.path.clone());
    let l = line(&name, "file-name");
    l.set_hexpand(true);
    bx.append(&l);
    if !dir {
        bx.append(&line(&size(node.size), "job-time"));
    } else {
        bx.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&bx));
    let (u, path) = (ui.clone(), node.path.clone());
    check.connect_toggled(move |c| {
        {
            let mut s = u.restore.state.borrow_mut();
            if c.is_active() {
                s.selected.insert(path.clone());
            } else {
                s.selected.remove(&path);
            }
        }
        render_selection(&u);
    });
    let gesture = gtk::GestureClick::new();
    let (u, path, c) = (ui.clone(), node.path.clone(), check.clone());
    gesture.connect_released(move |g, _, x, _| {
        // Clicks on the check box itself only toggle it.
        if x < 34.0 {
            return;
        }
        let _ = g;
        if dir {
            browse(&u, path.clone());
        } else {
            c.set_active(!c.is_active());
        }
    });
    row.add_controller(gesture);
    row
}

fn render_selection(ui: &Rc<Ui>) {
    let v = &ui.restore;
    let n = v.state.borrow().selected.len();
    v.count.set_text(&if n == 0 {
        "Select files or folders to restore".to_string()
    } else {
        format!("{} selected", count(n as u64, "item", "items"))
    });
    v.restore.set_sensitive(n > 0);
    v.clear.set_sensitive(n > 0);
}

/// Browse the latest snapshot's configuration folder, restoring to the
/// original places by default.
fn dotfiles(ui: &Rc<Ui>) {
    let Some(profile) = current(ui) else { return };
    let u = ui.clone();
    toast(ui, "Opening the latest snapshot…");
    work(
        ui,
        move || backup_service::restore_read(&profile, Request::Snapshots),
        move |ui, value| {
            let latest = value
                .as_array()
                .into_iter()
                .flatten()
                .max_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()))
                .cloned();
            let Some(s) = latest else {
                error(
                    ui,
                    "No snapshots yet",
                    "Back up first, then restore settings files from a snapshot.",
                );
                return;
            };
            ui.show("restore");
            open_snapshot(
                &u,
                s,
                Some(nyx_core::paths::config_home().display().to_string()),
            );
            u.restore.state.borrow_mut().dotfiles = true;
        },
    );
}

const MODES: &[(&str, &str)] = &[
    ("separate", "New folder"),
    ("original", "Original place"),
    ("migrate", "This computer"),
];
const POLICIES: &[(&str, &str)] = &[
    ("skip", "Keep existing"),
    ("keep-both", "Keep both"),
    ("replace", "Replace"),
];

fn options(ui: &Rc<Ui>) {
    let (paths, snapshot, profile, dotfiles) = {
        let s = ui.restore.state.borrow();
        (
            s.selected.iter().cloned().collect::<Vec<_>>(),
            s.snapshot
                .as_ref()
                .and_then(|v| v["id"].as_str().map(String::from))
                .unwrap_or_default(),
            s.profile.clone().unwrap_or_default(),
            s.dotfiles,
        )
    };
    if paths.is_empty() || snapshot.is_empty() {
        return;
    }
    let sheet = Sheet::new(
        &format!("Restore {}", count(paths.len() as u64, "item", "items")),
        500,
    );
    let mode = Rc::new(RefCell::new(
        if dotfiles { "original" } else { "separate" }.to_string(),
    ));
    let target = gtk::Entry::builder()
        .hexpand(true)
        .text(
            nyx_core::paths::home()
                .join("Restored files")
                .join(&snapshot[..8.min(snapshot.len())])
                .display()
                .to_string(),
        )
        .build();
    let target_row = w::hbox(6);
    target_row.append(&target);
    {
        let (t, parent) = (target.clone(), sheet.parent.clone());
        target_row.append(&icon_button(
            "folder-open-symbolic",
            "Choose a folder",
            move || {
                let t = t.clone();
                forms::choose_folder(&parent, "Restore into", move |p| {
                    t.set_text(&p.display().to_string())
                });
            },
        ));
    }
    let hint = label("", "field-hint");
    let closed = gtk::Switch::new();
    let closed_row = w::row(
        "The apps using these files are closed",
        "Replacing settings under a running app can lose its changes.",
        Some(closed.upcast_ref()),
    );
    closed_row.add_css_class("form-option");
    let update = {
        let (target_row, hint, closed_row) = (target_row.clone(), hint.clone(), closed_row.clone());
        move |m: &str| {
            target_row.set_visible(m == "separate");
            closed_row.set_visible(m != "separate");
            hint.set_text(match m {
                "separate" => "Files go into a new folder, so nothing you have now is touched.",
                "original" => "Files go back where they were in your home folder. Only your own dotfiles and settings qualify.",
                _ => "Maps another computer's home and settings to this one, and prepares a package list for Setup.",
            });
        }
    };
    update(&mode.borrow());
    let seg = {
        let initial = mode.borrow().clone();
        let (mode, update) = (mode.clone(), update.clone());
        w::segmented(&w::opts(MODES), &initial, move |m| {
            update(&m);
            *mode.borrow_mut() = m;
        })
    };
    let where_box = w::vbox(8);
    where_box.append(&seg);
    where_box.append(&target_row);
    where_box.append(&hint);
    forms::field(&sheet.body, "Restore to", "", &where_box);
    let policy = Rc::new(RefCell::new("skip".to_string()));
    let pseg = {
        let policy = policy.clone();
        w::segmented(&w::opts(POLICIES), "skip", move |p| {
            *policy.borrow_mut() = p
        })
    };
    forms::field(
        &sheet.body,
        "When a file already exists",
        "Replaced files are kept beside the new ones, so a restore can be rolled back.",
        &pseg,
    );
    sheet.body.append(&closed_row);
    let secrets = forms::switch(
        &sheet.body,
        "Include keys and credentials",
        "SSH and GPG keys, password stores and browser profiles in the selection.",
        false,
    );
    sheet.body.append(&label(
        "Files are decrypted into private storage first, so you can review every change before anything is written.",
        "field-hint",
    ));
    sheet.close_button("Cancel");
    let go = sheet.action("Review changes…", true);
    let (s, u) = (sheet.clone(), ui.clone());
    go.connect_clicked(move |_| {
        let m = mode.borrow().clone();
        let policy = match policy.borrow().as_str() {
            "keep-both" => Policy::KeepBoth,
            "replace" => Policy::Replace,
            _ => Policy::Skip,
        };
        if m != "separate" && !closed.is_active() {
            s.fail("Close the apps that use these files, then confirm it above.");
            return;
        }
        if m == "migrate" {
            s.close();
            crate::migration_ui::prepare(
                &u,
                profile.clone(),
                snapshot.clone(),
                paths.clone(),
                None,
                policy,
                secrets.is_active(),
            );
            return;
        }
        let request = Request::Plan {
            snapshot: snapshot.clone(),
            paths: paths.clone(),
            target: (m == "separate").then(|| PathBuf::from(target.text().trim())),
            mode: if m == "separate" {
                Mode::Separate
            } else {
                Mode::OriginalDotfiles
            },
            policy,
            include_secrets: secrets.is_active(),
        };
        let profile = profile.clone();
        let p2 = profile.clone();
        s.submit(
            &u,
            move || backup_service::restore_start(&profile, request),
            move |ui, job| {
                toast(ui, "Decrypting the selected files for review…");
                track(ui, job, move |ui, value| {
                    if value["state"] != "success" {
                        error(
                            ui,
                            "The restore couldn't be prepared",
                            value["message"].as_str().unwrap_or(""),
                        );
                        return;
                    }
                    if let Some(id) = value["result"]["plan"].as_str() {
                        review(ui, p2, id.to_string());
                    }
                });
            },
        );
    });
    sheet.present();
}

fn action_tag(action: &str) -> gtk::Label {
    match action {
        "create" => w::tag("New", "accent"),
        "replace" => w::tag("Replace", "warning"),
        "skip" => w::tag("Kept", ""),
        "unchanged" => w::tag("Same", ""),
        other => w::tag(other, ""),
    }
}

/// Review a restore plan or receipt: what each file will do, diffs, then apply
/// (two clicks) or roll back.
pub fn review(ui: &Rc<Ui>, profile: String, id: String) {
    let p = profile.clone();
    work(
        ui,
        move || backup_service::restore_read(&p, Request::Inspect { plan: id }),
        move |ui, value| {
            let plan: Plan = match serde_json::from_value(value) {
                Ok(p) => p,
                Err(e) => {
                    error(ui, "This restore can't be shown", &e.to_string());
                    return;
                }
            };
            review_sheet(ui, profile, plan);
        },
    );
}

fn review_sheet(ui: &Rc<Ui>, profile: String, plan: Plan) {
    let ready = plan.state == "ready";
    let sheet = Sheet::new(
        if ready {
            "Review the restore"
        } else {
            "Restore receipt"
        },
        600,
    );
    let creates = plan.entries.iter().filter(|e| e.action == "create").count();
    let replaces = plan
        .entries
        .iter()
        .filter(|e| e.action == "replace")
        .count();
    let keeps = plan
        .entries
        .iter()
        .filter(|e| e.action == "skip" || e.action == "unchanged")
        .count();
    let (flow, values) = w::kv_flow(&["New", "Replaced", "Left as is"]);
    values[0].set_text(&creates.to_string());
    values[1].set_text(&replaces.to_string());
    values[2].set_text(&keeps.to_string());
    sheet.body.append(&flow);
    let target = match plan.mode {
        Mode::Separate => format!("Into {}", nyx_core::paths::pretty(&plan.root)),
        Mode::OriginalDotfiles => "Back to their original places".into(),
        Mode::MigratedUserData(_) => "Mapped to this computer's home".into(),
    };
    let state = match plan.state.as_str() {
        "ready" => String::new(),
        "applied" => " · applied".into(),
        "rolled_back" => " · rolled back".into(),
        "partial" => " · partly applied".into(),
        "rollback_conflict" => " · rollback kept changed files".into(),
        other => format!(" · {other}"),
    };
    sheet
        .body
        .append(&label(&format!("{target}{state}"), "field-hint"));
    for m in plan.messages.iter().skip(1) {
        sheet.body.append(&label(m, "field-hint"));
    }
    let list = w::vbox(2);
    list.add_css_class("compact-list");
    sheet.body.append(&list);
    let more = gtk::Button::with_label("Show more");
    more.set_halign(gtk::Align::Start);
    sheet.body.append(&more);
    let plan = Rc::new(plan);
    let shown = Rc::new(std::cell::Cell::new(0usize));
    let fill = {
        let (ui, list, more, plan, shown, profile) = (
            ui.clone(),
            list.clone(),
            more.clone(),
            plan.clone(),
            shown.clone(),
            profile.clone(),
        );
        move || {
            let end = (shown.get() + 100).min(plan.entries.len());
            for e in &plan.entries[shown.get()..end] {
                let row = w::hbox(8);
                row.add_css_class("plan-row");
                row.append(&action_tag(&e.action));
                let text = w::vbox(1);
                text.set_hexpand(true);
                let name = Path::new(&e.source)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(e.source.clone());
                text.append(&line(&name, "file-name"));
                let detail = line(&nyx_core::paths::pretty(&e.target), "job-detail");
                detail.set_tooltip_text(Some(&e.target.display().to_string()));
                text.append(&detail);
                if let Some(a) = &e.adaptation {
                    text.append(&label(a, "field-hint"));
                }
                if !e.message.is_empty() {
                    text.append(&label(&e.message, "danger-text"));
                }
                row.append(&text);
                if e.after.kind == "file" && e.after.size <= 1024 * 1024 && e.action != "skip" {
                    let (u, profile, id, path) = (
                        ui.clone(),
                        profile.clone(),
                        plan.id.clone(),
                        e.source.clone(),
                    );
                    row.append(&icon_button(
                        "document-properties-symbolic",
                        "Compare with the current file",
                        move || diff(&u, profile.clone(), id.clone(), path.clone()),
                    ));
                }
                list.append(&row);
            }
            shown.set(end);
            more.set_visible(end < plan.entries.len());
        }
    };
    fill();
    let f = Rc::new(fill);
    more.connect_clicked(move |_| f());
    sheet.close_button(if ready { "Cancel" } else { "Close" });
    if ready {
        let (s, u, profile, id) = (sheet.clone(), ui.clone(), profile.clone(), plan.id.clone());
        sheet.danger("Apply", "Click again to apply", move || {
            let (profile, id) = (profile.clone(), id.clone());
            let p2 = profile.clone();
            s.submit(
                &u,
                move || {
                    backup_service::restore_start(
                        &profile,
                        Request::Apply {
                            plan: id.clone(),
                            confirmation: id,
                        },
                    )
                },
                move |ui, job| {
                    toast(ui, "Restoring…");
                    track(ui, job, move |ui, value| {
                        if value["state"] == "success" {
                            toast(ui, "Restored. The receipt can roll it back.");
                        } else {
                            error(
                                ui,
                                "The restore didn't finish",
                                value["message"].as_str().unwrap_or(""),
                            );
                        }
                        let _ = p2;
                        crate::activity::refresh(ui);
                    });
                },
            );
        });
    } else if plan.state != "rolled_back" {
        let b = sheet.action("Roll back…", false);
        let (win, u, profile, id) = (sheet.clone(), ui.clone(), profile.clone(), plan.id.clone());
        b.connect_clicked(move |_| {
            win.close();
            rollback(&u, profile.clone(), id.clone());
        });
    }
    sheet.present();
}

fn diff(ui: &Rc<Ui>, profile: String, plan: String, path: String) {
    work(
        ui,
        move || backup_service::restore_read(&profile, Request::Diff { plan, path }),
        |_, value| {
            let sheet = Sheet::new("Changes", 640);
            let text = value["diff"].as_str().unwrap_or("No differences");
            let code = label(
                if text.is_empty() {
                    "No differences"
                } else {
                    text
                },
                "code-block",
            );
            code.set_selectable(true);
            code.set_wrap(false);
            sheet.body.append(&code);
            sheet.close_button("Close");
            sheet.present();
        },
    );
}

fn receipts(ui: &Rc<Ui>, profile: String) {
    let p = profile.clone();
    work(
        ui,
        move || backup_service::restore_read(&p, Request::Receipts),
        move |ui, value| {
            let sheet = Sheet::new("Restore receipts", 520);
            let receipts = value["receipts"].as_array().cloned().unwrap_or_default();
            if receipts.is_empty() {
                sheet.body.append(&label(
                    "No restores yet. Each restore keeps a receipt here, with the files it replaced, so it can be rolled back.",
                    "",
                ));
            }
            let list = w::vbox(2);
            list.add_css_class("compact-list");
            for r in receipts {
                let Some(id) = r["id"].as_str().map(String::from) else {
                    continue;
                };
                let row = w::hbox(10);
                row.add_css_class("plan-row");
                let state = r["state"].as_str().unwrap_or("");
                row.append(&w::tag(
                    match state {
                        "ready" => "Not applied",
                        "applied" => "Applied",
                        "rolled_back" => "Rolled back",
                        "partial" => "Partial",
                        "rollback_conflict" => "Conflicts",
                        other => other,
                    },
                    if state == "applied" { "accent" } else { "" },
                ));
                let text = w::vbox(1);
                text.set_hexpand(true);
                text.append(&line(
                    &when(r["created"].as_u64().unwrap_or(0)),
                    "file-name",
                ));
                text.append(&line(
                    &nyx_core::paths::pretty(Path::new(r["target"].as_str().unwrap_or(""))),
                    "job-detail",
                ));
                row.append(&text);
                let (u, profile, win) = (ui.clone(), profile.clone(), sheet.clone());
                row.append(&icon_button(
                    "go-next-symbolic",
                    "Open this receipt",
                    move || {
                        win.close();
                        review(&u, profile.clone(), id.clone());
                    },
                ));
                list.append(&row);
            }
            sheet.body.append(&list);
            sheet.close_button("Close");
            sheet.present();
        },
    );
}

fn rollback(ui: &Rc<Ui>, profile: String, id: String) {
    let (p, i) = (profile.clone(), id.clone());
    work(
        ui,
        move || backup_service::restore_read(&p, Request::RollbackPreview { plan: i }),
        move |ui, value| {
            let sheet = Sheet::new("Roll back the restore", 560);
            sheet.body.append(&label(
                "Files you changed since the restore are kept. Replaced originals come back only if their saved copies are unchanged.",
                "",
            ));
            let list = w::vbox(2);
            list.add_css_class("compact-list");
            let entries = value["entries"].as_array().cloned().unwrap_or_default();
            let mut undo = 0;
            for e in entries.iter().take(300) {
                let row = w::hbox(8);
                row.add_css_class("plan-row");
                let status = &e["status"];
                let tag = if let Some(c) = status["conflict"].as_str() {
                    row.set_tooltip_text(Some(c));
                    w::tag("Kept", "warning")
                } else if status["action"] == "undo" {
                    undo += 1;
                    w::tag("Undo", "accent")
                } else {
                    w::tag("Nothing", "")
                };
                row.append(&tag);
                let l = line(
                    &nyx_core::paths::pretty(Path::new(e["target"].as_str().unwrap_or(""))),
                    "path-line",
                );
                l.set_hexpand(true);
                row.append(&l);
                list.append(&row);
            }
            sheet.body.append(&list);
            sheet.close_button("Cancel");
            let (s, u) = (sheet.clone(), ui.clone());
            let b = sheet.danger("Roll back", "Click again to roll back", move || {
                let (profile, id) = (profile.clone(), id.clone());
                s.submit(
                    &u,
                    move || {
                        backup_service::restore_start(
                            &profile,
                            Request::Rollback {
                                plan: id.clone(),
                                confirmation: id,
                            },
                        )
                    },
                    |ui, job| {
                        toast(ui, "Rolling back…");
                        track(ui, job, |ui, value| {
                            if value["state"] == "success" {
                                toast(ui, "Rolled back");
                            } else {
                                error(
                                    ui,
                                    "Some files were kept",
                                    value["message"].as_str().unwrap_or(""),
                                );
                            }
                        });
                    },
                );
            });
            b.set_sensitive(undo > 0);
            sheet.present();
        },
    );
}

/// For the review build: open the newest snapshot and select two entries.
#[cfg(feature = "ui-review")]
pub fn review_open_first(ui: &Rc<Ui>) {
    let Some(profile) = current(ui) else { return };
    let u = ui.clone();
    work(
        ui,
        move || backup_service::restore_read(&profile, Request::Snapshots),
        move |ui, value| {
            let Some(s) = value.as_array().and_then(|a| a.last()).cloned() else {
                return;
            };
            ui.show("restore");
            let dir = start_directory(&s);
            open_snapshot(&u, s, Some(dir.clone()));
            u.restore
                .state
                .borrow_mut()
                .selected
                .insert(format!("{dir}/Documents"));
            render_selection(&u);
        },
    );
}

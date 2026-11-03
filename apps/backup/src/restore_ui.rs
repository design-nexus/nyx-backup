//! Phase 2: local snapshot browser, staged review, diffs, and rollback receipts.
use super::*;
use backup_restore::{Mode, Node, Plan, Policy, Request};
use std::collections::BTreeSet;
fn read(
    ui: &Rc<Ui>,
    profile: String,
    request: Request,
    done: impl FnOnce(&Rc<Ui>, serde_json::Value) + 'static,
) {
    work(
        ui,
        "Reading restore data…",
        move || Ok(backup_service::restore_read(&profile, request)?.to_string()),
        move |ui, s| {
            if let Ok(value) = serde_json::from_str(&s) {
                done(ui, value)
            }
        },
    );
}
fn job(
    ui: &Rc<Ui>,
    profile: String,
    request: Request,
    done: impl FnOnce(&Rc<Ui>, serde_json::Value) + 'static,
) {
    work(
        ui,
        "Starting restore job…",
        move || backup_service::restore_start(&profile, request),
        move |ui, id| {
            *ui.job.borrow_mut() = Some(id.clone());
            ui.notice.set_text(
                "Restore job is owned by the background service. Close the window safely.",
            );
            let ui = ui.clone();
            nyx_core::cmd::background(
                move || backup_service::wait_job(&id),
                move |result| match result {
                    Ok(value) => done(&ui, value),
                    Err(e) => message(&ui, "Restore needs attention", &e.to_string()),
                },
            );
        },
    );
}
pub fn refresh(ui: &Rc<Ui>) {
    clear(&ui.restore);
    let u = ui.clone();
    ui.restore.append(&action(
        "computer-symbolic",
        "Read current package and hardware inventory",
        move || migration_ui::inventory(&u),
    ));
    let u = ui.clone();
    ui.restore.append(&action(
        "drive-harddisk-symbolic",
        "Offline disk recovery and boot kit",
        move || recovery_ui::show(&u),
    ));
    let c = match Store::default().load() {
        Ok(c) => c,
        Err(_) => return,
    };
    ui.restore.append(&card("Restore selected files","Browse a snapshot, select files or folders, then review a staged plan. Separate-folder restores are the default. Existing files are skipped unless you explicitly choose another conflict policy."));
    if c.profiles.is_empty() {
        ui.restore.append(&text(
            "Create a backup profile first. Its existing snapshots will appear here.",
            "dim-label",
        ));
    }
    for p in c.profiles {
        let row = card(
            &p.name,
            "Snapshots are restricted to this Backup profile. Restore, diffs and rollback use a private local service channel.",
        );
        let actions = action_flow();
        let u = ui.clone();
        let profile = p.id.clone();
        actions.append(&action(
            "folder-open-symbolic",
            "Browse snapshots",
            move || snapshots(&u, profile.clone(), false),
        ));
        let u = ui.clone();
        let profile = p.id.clone();
        actions.append(&action(
            "preferences-system-symbolic",
            "Quick Dotfiles Restore — review and restore original configuration paths",
            move || snapshots(&u, profile.clone(), true),
        ));
        let u = ui.clone();
        let profile = p.id.clone();
        actions.append(&action(
            "edit-undo-symbolic",
            "Restore receipts and rollback",
            move || receipts(&u, profile.clone()),
        ));
        let u = ui.clone();
        let profile = p.id.clone();
        actions.append(&action(
            "document-send-symbolic",
            "Migration / Setup bundles",
            move || migration_ui::bundles(&u, profile.clone()),
        ));
        row.append(&actions);
        ui.restore.append(&row);
    }
    ui.restore.append(&card("Recovery scope","Restores file contents, symlinks, modes and modification times. Ownership, ACLs and extended attributes are not applied. Close apps before replacing their configuration. Offline whole-disk recovery uses a separate boot environment and explicitly captured disk images. Ordinary profiles remain selected-data backups. Physical and cross-PC system recovery remain unqualified."));
}
fn snapshots(ui: &Rc<Ui>, profile: String, quick: bool) {
    let next = profile.clone();
    read(ui, profile, Request::Snapshots, move |ui, value| {
        let d = dialog(
            &ui.window,
            if quick {
                "Choose a snapshot for Quick Dotfiles Restore"
            } else {
                "Choose a snapshot"
            },
        );
        d.add_button("Close", gtk::ResponseType::Close);
        let b = form(&d);
        let snapshots = value.as_array().cloned().unwrap_or_default();
        if snapshots.is_empty() {
            b.append(&text("No snapshots found for this profile.", "dim-label"));
        }
        for snapshot in snapshots {
            let Some(id) = snapshot["id"].as_str() else {
                continue;
            };
            let row = card(
                snapshot["time"].as_str().unwrap_or("Snapshot"),
                &id[..id.len().min(12)],
            );
            let u = ui.clone();
            let profile = next.clone();
            let id = id.to_string();
            let dd = d.clone();
            row.append(&action(
                "folder-open-symbolic",
                "Browse this snapshot",
                move || {
                    dd.close();
                    browser(&u, profile.clone(), id.clone(), quick);
                },
            ));
            b.append(&row);
        }
        d.connect_response(|d, _| d.close());
        d.present();
    });
}
struct Browser {
    ui: Rc<Ui>,
    profile: String,
    snapshot: String,
    path: gtk::Entry,
    rows: gtk::Box,
    selected: RefCell<BTreeSet<String>>,
    count: gtk::Label,
    target: gtk::Entry,
    mode: gtk::DropDown,
    policy: gtk::DropDown,
    secrets: gtk::CheckButton,
    closed_apps: gtk::CheckButton,
}
fn browser(ui: &Rc<Ui>, profile: String, snapshot: String, quick: bool) {
    let d = dialog(&ui.window, "Browse snapshot and select files");
    d.set_default_width(840);
    d.set_default_height(740);
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Prepare restore plan", gtk::ResponseType::Accept);
    let b = form(&d);
    let nav = w::hbox(6);
    let up = w::bar_button("go-up-symbolic", "Parent snapshot folder");
    nav.append(&up);
    let path = gtk::Entry::builder()
        .hexpand(true)
        .text(if quick {
            nyx_core::paths::config_home().display().to_string()
        } else {
            "/".into()
        })
        .build();
    nav.append(&path);
    let open = w::bar_button("go-next-symbolic", "Open snapshot folder");
    nav.append(&open);
    b.append(&nav);
    let rows = w::vbox(6);
    b.append(&rows);
    let count = text("No paths selected", "dim-label");
    b.append(&count);
    let mode = gtk::DropDown::from_strings(&[
        "Separate folder (default)",
        "Original dotfiles — Quick Restore",
        "Migration / Setup — map source home to this user",
    ]);
    mode.set_selected(if quick { 1 } else { 0 });
    b.append(&mode);
    let target = entry(
        &b,
        "Separate restore folder",
        &nyx_core::paths::home()
            .join("Restored files")
            .join(&snapshot[..8])
            .display()
            .to_string(),
    );
    let row = w::hbox(6);
    let target_clone = target.clone();
    let parent = d.clone();
    row.append(&action(
        "folder-open-symbolic",
        "Choose a restore folder",
        move || {
            let target = target_clone.clone();
            gtk::FileDialog::new().select_folder(
                Some(&parent),
                gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            target.set_text(&path.display().to_string());
                        }
                    }
                },
            );
        },
    ));
    b.append(&row);
    b.append(&text("If a destination file exists:", "dim-label"));
    let policy = gtk::DropDown::from_strings(&[
        "Skip existing files",
        "Keep both with a restored suffix",
        "Replace and retain originals for rollback",
    ]);
    b.append(&policy);
    let secrets = gtk::CheckButton::with_label(
        "Authorize restoration of recognized private keys and credential stores",
    );
    b.append(&secrets);
    let closed_apps = gtk::CheckButton::with_label(
        "I have closed apps using these files (required for original-path restore and migration)",
    );
    closed_apps.set_visible(quick);
    b.append(&closed_apps);
    b.append(&text("Any file may contain secrets. Selected files are decrypted into private local staging for review. Recovery copies are retained until you manage them explicitly.","dim-label"));
    let browser = Rc::new(Browser {
        ui: ui.clone(),
        profile,
        snapshot,
        path,
        rows,
        selected: RefCell::new(BTreeSet::new()),
        count,
        target,
        mode,
        policy,
        secrets,
        closed_apps,
    });
    let target = browser.target.clone();
    target.set_sensitive(!quick);
    let closed = browser.closed_apps.clone();
    browser.mode.connect_selected_notify(move |mode| {
        target.set_sensitive(mode.selected() == 0);
        closed.set_visible(mode.selected() != 0);
    });
    let bb = browser.clone();
    open.connect_clicked(move |_| load_folder(&bb));
    let bb = browser.clone();
    up.connect_clicked(move |_| {
        let current = PathBuf::from(bb.path.text().as_str());
        bb.path.set_text(
            &current
                .parent()
                .unwrap_or(Path::new("/"))
                .display()
                .to_string(),
        );
        load_folder(&bb);
    });
    let bb = browser.clone();
    browser.path.connect_activate(move |_| load_folder(&bb));
    let bb = browser.clone();
    d.connect_response(move|d,response|{
  if response!=gtk::ResponseType::Accept{d.close();return}let paths:Vec<_>=bb.selected.borrow().iter().cloned().collect();if paths.is_empty() && bb.mode.selected()!=2 {message(&bb.ui,"Select files","Select at least one file or folder from the snapshot.");return}
  if bb.mode.selected()==2 {if !bb.closed_apps.is_active(){message(&bb.ui,"Close affected apps","Close apps using the selected files before migration.");return}let policy=match bb.policy.selected(){1=>Policy::KeepBoth,2=>Policy::Replace,_=>Policy::Skip};d.close();migration_ui::prepare(&bb.ui,bb.profile.clone(),bb.snapshot.clone(),paths,None,policy,bb.secrets.is_active());return}
  let mode=if bb.mode.selected()==1{Mode::OriginalDotfiles}else{Mode::Separate};if mode==Mode::OriginalDotfiles && !bb.closed_apps.is_active(){message(&bb.ui,"Close affected apps","Close apps using the selected configuration files, then confirm that they are closed.");return}
  let request=Request::Plan{snapshot:bb.snapshot.clone(),paths,target:if mode==Mode::Separate{Some(PathBuf::from(bb.target.text().as_str()))}else{None},mode,policy:match bb.policy.selected(){1=>Policy::KeepBoth,2=>Policy::Replace,_=>Policy::Skip},include_secrets:bb.secrets.is_active()};let profile=bb.profile.clone();d.close();
  let pp=profile.clone();job(&bb.ui,profile,request,move|ui,value|{if value["state"]!="success"{message(ui,"Restore plan needs attention",&outcome(&value));return}if let Some(id)=value["result"]["plan"].as_str(){review(ui,pp,id.to_string());}});
 });
    load_folder(&browser);
    d.present();
}
fn load_folder(browser: &Rc<Browser>) {
    let directory = browser.path.text().to_string();
    let bb = browser.clone();
    read(
        &browser.ui,
        browser.profile.clone(),
        Request::Browse {
            snapshot: browser.snapshot.clone(),
            directory,
        },
        move |_, value| {
            clear(&bb.rows);
            if value["truncated"] == true {
                bb.rows.append(&text("This folder has more than 2,000 entries. Browse a narrower folder or use the CLI.","dim-label"));
            }
            let nodes: Vec<Node> =
                serde_json::from_value(value["entries"].clone()).unwrap_or_default();
            if nodes.is_empty() {
                bb.rows
                    .append(&text("No entries in this folder.", "dim-label"));
            }
            for node in nodes {
                let row = w::hbox(8);
                let label = Path::new(&node.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(node.path.clone());
                let check = gtk::CheckButton::with_label(&format!(
                    "{} · {} · {} bytes",
                    label, node.kind, node.size
                ));
                check.set_hexpand(true);
                check.set_tooltip_text(Some(&node.path));
                check.set_active(bb.selected.borrow().contains(&node.path));
                let path = node.path.clone();
                let selected = bb.clone();
                check.connect_toggled(move |check| {
                    if check.is_active() {
                        selected.selected.borrow_mut().insert(path.clone());
                    } else {
                        selected.selected.borrow_mut().remove(&path);
                    }
                    selected.count.set_text(&format!(
                        "{} path(s) selected",
                        selected.selected.borrow().len()
                    ));
                });
                row.append(&check);
                if node.kind == "dir" {
                    let next = bb.clone();
                    let path = node.path.clone();
                    row.append(&action("folder-open-symbolic", "Open folder", move || {
                        next.path.set_text(&path);
                        load_folder(&next);
                    }));
                }
                bb.rows.append(&row);
            }
        },
    );
}
pub fn review(ui: &Rc<Ui>, profile: String, id: String) {
    let next = profile.clone();
    read(
        ui,
        profile,
        Request::Inspect { plan: id },
        move |ui, value| {
            let plan: Plan = match serde_json::from_value(value) {
                Ok(p) => p,
                Err(_) => return,
            };
            let d = dialog(&ui.window, "Review restore plan and recovery copies");
            d.set_default_width(800);
            d.set_default_height(650);
            d.add_button("Close", gtk::ResponseType::Close);
            if plan.state == "ready" {
                d.add_button("Apply reviewed plan", gtk::ResponseType::Accept);
            } else if plan.state != "rolled_back" {
                d.add_button("Preview rollback", gtk::ResponseType::Reject);
            }
            let b = form(&d);
            let heading = text(
                &format!(
                    "Plan {}\nState: {} · {} files\nTarget: {}\nSnapshot: {}",
                    plan.id,
                    plan.state,
                    plan.entries.len(),
                    plan.root.display(),
                    &plan.snapshot[..8]
                ),
                "",
            );
            heading.set_selectable(true);
            b.append(&heading);
            for message in &plan.messages {
                b.append(&text(message, "dim-label"));
            }
            let rows = w::vbox(6);
            b.append(&rows);
            let more = gtk::Button::with_label("Show more files");
            b.append(&more);
            let offset = Rc::new(Cell::new(0usize));
            let plan = Rc::new(plan);
            let populate = {
                let ui = ui.clone();
                let rows = rows.clone();
                let more = more.clone();
                let offset = offset.clone();
                let plan = plan.clone();
                let profile = next.clone();
                move || {
                    let end = (offset.get() + 100).min(plan.entries.len());
                    for e in &plan.entries[offset.get()..end] {
                        let row = card(
                            &e.source,
                            &format!(
                                "{} → {}\n{}",
                                e.action,
                                e.target.display(),
                                if e.message.is_empty() {
                                    &e.status
                                } else {
                                    &e.message
                                }
                            ),
                        );
                        if let Some(adaptation) = &e.adaptation {
                            row.append(&text(adaptation, "dim-label"));
                        }
                        if e.before.is_some() && e.status != "pending" {
                            let saved = text(
                                &format!("Saved original: {}", e.previous.display()),
                                "dim-label",
                            );
                            saved.set_selectable(true);
                            row.append(&saved);
                        }
                        if e.after.kind == "file" && e.after.size <= 1024 * 1024 {
                            let u = ui.clone();
                            let profile = profile.clone();
                            let id = plan.id.clone();
                            let path = e.source.clone();
                            row.append(&action(
                                "document-properties-symbolic",
                                "View text diff against current file",
                                move || {
                                    read(
                                        &u,
                                        profile.clone(),
                                        Request::Diff {
                                            plan: id.clone(),
                                            path: path.clone(),
                                        },
                                        |ui, value| {
                                            message(
                                                ui,
                                                "Current file → snapshot diff",
                                                value["diff"]
                                                    .as_str()
                                                    .unwrap_or("No diff available"),
                                            )
                                        },
                                    )
                                },
                            ));
                        }
                        rows.append(&row);
                    }
                    offset.set(end);
                    more.set_visible(end < plan.entries.len());
                }
            };
            let populate = Rc::new(populate);
            populate();
            let more_populate = populate.clone();
            more.connect_clicked(move |_| more_populate());
            let confirm = entry(&b, "Type the plan ID above to confirm applying changes", "");
            confirm.set_visible(plan.state == "ready");
            let u = ui.clone();
            d.connect_response(move |d, response| {
                if response == gtk::ResponseType::Accept {
                    if confirm.text() != plan.id {
                        message(
                            &u,
                            "Confirm the reviewed plan",
                            "Type the exact plan ID shown above before applying changes.",
                        );
                        return;
                    }
                    let profile = next.clone();
                    job(
                        &u,
                        profile,
                        Request::Apply {
                            plan: plan.id.clone(),
                            confirmation: plan.id.clone(),
                        },
                        |ui, value| {
                            message(ui, "Restore result", &outcome(&value));
                            refresh(ui);
                        },
                    );
                } else if response == gtk::ResponseType::Reject {
                    rollback_dialog(&u, next.clone(), plan.id.clone());
                }
                d.close();
            });
            d.present();
        },
    );
}
fn receipts(ui: &Rc<Ui>, profile: String) {
    let next = profile.clone();
    read(ui, profile, Request::Receipts, move |ui, value| {
        let d = dialog(&ui.window, "Restore receipts and rollback");
        d.set_default_height(550);
        d.add_button("Close", gtk::ResponseType::Close);
        let b = form(&d);
        let receipts = value["receipts"].as_array().cloned().unwrap_or_default();
        if receipts.is_empty() {
            b.append(&text("No restore receipts yet.", "dim-label"));
        }
        for receipt in receipts {
            let Some(id) = receipt["id"].as_str() else {
                continue;
            };
            let row = card(
                id,
                &format!(
                    "{} · {}",
                    receipt["state"].as_str().unwrap_or(""),
                    receipt["target"].as_str().unwrap_or("")
                ),
            );
            let u = ui.clone();
            let profile = next.clone();
            let id = id.to_string();
            row.append(&action(
                "document-properties-symbolic",
                "Review restore receipt",
                move || review(&u, profile.clone(), id.clone()),
            ));
            b.append(&row);
        }
        d.connect_response(|d, _| d.close());
        d.present();
    });
}
fn rollback_dialog(ui: &Rc<Ui>, profile: String, id: String) {
    let next = profile.clone();
    let plan = id.clone();
    read(
        ui,
        profile,
        Request::RollbackPreview { plan: id },
        move |ui, value| {
            let d = dialog(&ui.window, "Review rollback");
            d.add_button("Cancel", gtk::ResponseType::Cancel);
            d.add_button(
                "Rollback unchanged restored files",
                gtk::ResponseType::Accept,
            );
            let b = form(&d);
            b.append(&text("Rollback preserves files modified since restoration. Replaced originals are restored only when their recovery copies still match. Restored copies are retained beside the target.","dim-label"));
            if let Some(entries) = value["entries"].as_array() {
                for entry in entries {
                    b.append(&text(
                        &format!(
                            "{}\n{}",
                            entry["target"].as_str().unwrap_or(""),
                            entry["status"]
                        ),
                        "",
                    ));
                }
            }
            let confirmation = entry(&b, &format!("Type plan ID {plan} to confirm rollback"), "");
            let u = ui.clone();
            d.connect_response(move |d, r| {
                if r == gtk::ResponseType::Accept {
                    if confirmation.text() != plan {
                        return;
                    }
                    job(
                        &u,
                        next.clone(),
                        Request::Rollback {
                            plan: plan.clone(),
                            confirmation: plan.clone(),
                        },
                        |ui, value| message(ui, "Rollback result", &outcome(&value)),
                    );
                }
                d.close();
            });
            d.present();
        },
    );
}

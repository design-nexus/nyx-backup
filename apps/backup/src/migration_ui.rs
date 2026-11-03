//! Reviewed user migration and Setup handoff; never a root installer UI.
use crate::forms::{self, Sheet};
use crate::ui::*;
use backup_core::migration::Inventory;
use backup_migrate::{Bundle, Request};
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::rc::Rc;

pub fn prepare(
    ui: &Rc<Ui>,
    profile: String,
    snapshot: String,
    paths: Vec<String>,
    source: Option<Inventory>,
    policy: backup_restore::Policy,
    secrets: bool,
) {
    let p = profile.clone();
    work(
        ui,
        move || {
            backup_service::migration_start(
                &p,
                Request::Prepare {
                    snapshot,
                    paths,
                    source,
                    policy,
                    include_secrets: secrets,
                    rewrite_references: true,
                },
            )
        },
        move |ui, job| {
            toast(ui, "Preparing the migration…");
            track(ui, job, move |ui, value| {
                if value["state"] != "success" {
                    error(
                        ui,
                        "The migration couldn't be prepared",
                        value["message"].as_str().unwrap_or(""),
                    );
                    return;
                }
                if let Some(bundle) = value["result"]["bundle"].as_str() {
                    review(ui, profile, bundle.into());
                }
            });
        },
    );
}

pub fn review(ui: &Rc<Ui>, profile: String, id: String) {
    let p = profile.clone();
    work(
        ui,
        move || backup_service::migration_read(&p, Request::Inspect { bundle: id }),
        move |ui, value| {
            let bundle: Bundle = match serde_json::from_value(value) {
                Ok(b) => b,
                Err(e) => {
                    error(ui, "This migration can't be shown", &e.to_string());
                    return;
                }
            };
            review_sheet(ui, profile, bundle);
        },
    );
}

fn review_sheet(ui: &Rc<Ui>, profile: String, bundle: Bundle) {
    let sheet = Sheet::new("Review the migration", 600);
    let installs = bundle
        .packages
        .iter()
        .filter(|p| p.action == "install")
        .count();
    let (flow, values) = w::kv_flow(&["Packages", "Files held", "From"]);
    values[0].set_text(&installs.to_string());
    values[1].set_text(&bundle.held_files.len().to_string());
    values[2].set_text(&bundle.source.hardware.architecture);
    sheet.body.append(&flow);
    sheet.body.append(&label(
        &format!(
            "{} → {}",
            nyx_core::paths::pretty(&bundle.source.home),
            nyx_core::paths::pretty(&bundle.target.home)
        ),
        "field-hint",
    ));
    for note in bundle.hardware_changes.iter().chain(&bundle.warnings) {
        sheet.body.append(&label(note, "field-hint"));
    }
    sheet.body.append(&label(
        "Package versions are a record, not instructions: Setup installs from this computer's repositories. Drivers, kernels, firmware, foreign builds and Flatpaks are held for you to review.",
        "field-hint",
    ));
    let packages = w::vbox(2);
    packages.add_css_class("compact-list");
    for p in bundle.packages.iter().take(150) {
        let row = w::hbox(8);
        row.add_css_class("plan-row");
        row.append(&w::tag(
            &p.action,
            if p.action == "install" { "accent" } else { "" },
        ));
        let text = w::vbox(1);
        text.set_hexpand(true);
        text.append(&line(&p.name, "file-name"));
        text.append(&line(&p.reason, "job-detail"));
        row.append(&text);
        packages.append(&row);
    }
    forms::field(&sheet.body, "Packages", "", &packages);
    if !bundle.held_files.is_empty() {
        let held = w::vbox(2);
        held.add_css_class("compact-list");
        for f in bundle.held_files.iter().take(150) {
            let t = w::vbox(1);
            t.append(&line(&f.path, "file-name"));
            t.append(&label(&f.reason, "job-detail"));
            held.append(&t);
        }
        forms::field(&sheet.body, "Held for review", "", &held);
    }
    if !bundle.source.flatpaks.is_empty() {
        let fl = w::vbox(2);
        fl.add_css_class("compact-list");
        for a in bundle.source.flatpaks.iter().take(150) {
            fl.append(&line(
                &format!("{} · {}", a.application, a.origin),
                "path-line",
            ));
        }
        forms::field(&sheet.body, "Flatpak apps", "Install these yourself.", &fl);
    }
    sheet.close_button("Close");
    let export = sheet.action("Export bundle…", false);
    {
        let (u, profile, id, parent) = (
            ui.clone(),
            profile.clone(),
            bundle.id.clone(),
            sheet.parent.clone(),
        );
        export.connect_clicked(move |_| {
            let (u, profile, id) = (u.clone(), profile.clone(), id.clone());
            forms::choose_save(
                &parent,
                "Export the Setup bundle",
                "nyx-backup-setup.json",
                move |path| {
                    let (profile, id) = (profile.clone(), id.clone());
                    work(
                        &u,
                        move || {
                            backup_service::migration_read(
                                &profile,
                                Request::Export { bundle: id, path },
                            )
                        },
                        |ui, _| {
                            toast(
                                ui,
                                "Bundle exported. Review it with nyx-backup-setup inspect.",
                            )
                        },
                    );
                },
            );
        });
    }
    if let Some(plan) = bundle.restore_plan.clone() {
        let files = sheet.action("Review files…", true);
        let (win, u, profile) = (sheet.clone(), ui.clone(), profile.clone());
        files.connect_clicked(move |_| {
            win.close();
            crate::restore_ui::review(&u, profile.clone(), plan.clone());
        });
    }
    sheet.present();
}

pub fn bundles(ui: &Rc<Ui>, profile: String) {
    let p = profile.clone();
    work(
        ui,
        move || backup_service::migration_read(&p, Request::Bundles),
        move |ui, value| {
            let sheet = Sheet::new("Migration bundles", 500);
            let items = value["bundles"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                sheet.body.append(&label(
                    "None yet. Select files in a snapshot and restore them to “This computer” to prepare one.",
                    "",
                ));
            }
            let list = w::vbox(2);
            list.add_css_class("compact-list");
            for b in items {
                let Some(id) = b["id"].as_str().map(String::from) else {
                    continue;
                };
                let row = w::hbox(10);
                row.add_css_class("plan-row");
                let text = w::vbox(1);
                text.set_hexpand(true);
                text.append(&line(
                    &when(b["created"].as_u64().unwrap_or(0)),
                    "file-name",
                ));
                text.append(&line(
                    &format!(
                        "Snapshot {}",
                        b["snapshot"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(8)
                            .collect::<String>()
                    ),
                    "job-detail",
                ));
                row.append(&text);
                let (u, profile, win) = (ui.clone(), profile.clone(), sheet.clone());
                row.append(&icon_button(
                    "go-next-symbolic",
                    "Open this bundle",
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

pub fn inventory(ui: &Rc<Ui>) {
    work(
        ui,
        || {
            let inventory = backup_system::migration_inventory::capture();
            Ok(inventory)
        },
        |_, inv: Inventory| {
            let sheet = Sheet::new("This computer's inventory", 480);
            let (flow, values) = w::kv_flow(&["Packages", "Flatpaks", "Displays"]);
            values[0].set_text(&inv.packages.len().to_string());
            values[1].set_text(&inv.flatpaks.len().to_string());
            values[2].set_text(&inv.hardware.displays.len().to_string());
            sheet.body.append(&flow);
            sheet.body.append(&label(
                &format!(
                    "{} {} · {}",
                    inv.hardware.system_vendor, inv.hardware.product, inv.hardware.architecture
                ),
                "",
            ));
            if !inv.missing.is_empty() {
                forms::field(
                    &sheet.body,
                    "Not covered",
                    "",
                    &label(&inv.missing.join("\n"), "field-hint"),
                );
            }
            sheet.body.append(&label(
                "Every new backup stores this inventory inside its encrypted snapshot, for migrating to another computer. Reading it changed nothing.",
                "field-hint",
            ));
            sheet.close_button("Close");
            sheet.present();
        },
    );
}

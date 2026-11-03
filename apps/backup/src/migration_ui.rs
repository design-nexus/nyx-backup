//! Phase 3: reviewed user migration and Setup handoff; never a root installer UI.
use super::*;
use backup_core::migration::Inventory;
use backup_migrate::{Bundle, Request};
pub fn prepare(
    ui: &Rc<Ui>,
    profile: String,
    snapshot: String,
    paths: Vec<String>,
    source: Option<Inventory>,
    policy: backup_restore::Policy,
    secrets: bool,
) {
    let next = profile.clone();
    work(
        ui,
        "Preparing migration and package plan…",
        move || {
            backup_service::migration_start(
                &profile,
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
        move |ui, id| {
            *ui.job.borrow_mut() = Some(id.clone());
            let ui = ui.clone();
            nyx_core::cmd::background(
                move || backup_service::wait_job(&id),
                move |result| match result {
                    Ok(value) => {
                        if value["state"] != "success" {
                            message(&ui, "Migration needs attention", &outcome(&value));
                            return;
                        }
                        if let Some(bundle) = value["result"]["bundle"].as_str() {
                            review(&ui, next, bundle.into());
                        }
                    }
                    Err(e) => message(&ui, "Migration needs attention", &e.to_string()),
                },
            );
        },
    );
}
fn read(
    ui: &Rc<Ui>,
    profile: String,
    request: Request,
    done: impl FnOnce(&Rc<Ui>, serde_json::Value) + 'static,
) {
    work(
        ui,
        "Reading migration data…",
        move || Ok(backup_service::migration_read(&profile, request)?.to_string()),
        move |ui, s| {
            if let Ok(value) = serde_json::from_str(&s) {
                done(ui, value)
            }
        },
    );
}
pub fn review(ui: &Rc<Ui>, profile: String, id: String) {
    let next = profile.clone();
    read(
        ui,
        profile,
        Request::Inspect { bundle: id },
        move |ui, value| {
            let bundle: Bundle = match serde_json::from_value(value) {
                Ok(bundle) => bundle,
                Err(_) => return,
            };
            let d = dialog(&ui.window, "Review migration and Setup bundle");
            d.set_default_width(800);
            d.set_default_height(650);
            d.add_button("Close", gtk::ResponseType::Close);
            let b = form(&d);
            let summary = text(
                &format!(
                    "Setup bundle {}\n{} → {}\nSource home: {}\nTarget home: {}\n{} packages proposed; {} files held for review",
                    bundle.id,
                    bundle.source.hardware.architecture,
                    bundle.target.hardware.architecture,
                    bundle.source.home.display(),
                    bundle.target.home.display(),
                    bundle
                        .packages
                        .iter()
                        .filter(|p| p.action == "install")
                        .count(),
                    bundle.held_files.len()
                ),
                "",
            );
            summary.set_selectable(true);
            b.append(&summary);
            for change in &bundle.hardware_changes {
                b.append(&text(change, "dim-label"));
            }
            for warning in &bundle.warnings {
                b.append(&text(warning, "dim-label"));
            }
            let actions = w::hbox(6);
            if let Some(plan) = &bundle.restore_plan {
                let u = ui.clone();
                let profile = next.clone();
                let plan = plan.clone();
                actions.append(&action(
                    "document-properties-symbolic",
                    "Review mapped files, adaptations and rollback plan",
                    move || restore_ui::review(&u, profile.clone(), plan.clone()),
                ));
            }
            let u = ui.clone();
            let profile = next.clone();
            let id = bundle.id.clone();
            let parent = d.clone();
            actions.append(&action("document-save-symbolic","Export a new Setup bundle file",move||{let u=u.clone();let profile=profile.clone();let id=id.clone();gtk::FileDialog::builder().title("Export Setup bundle").initial_name("nyx-backup-setup.json").build().save(Some(&parent),gio::Cancellable::NONE,move|result|{if let Ok(file)=result{if let Some(path)=file.path(){read(&u,profile,Request::Export{bundle:id,path},|ui,_|message(ui,"Setup bundle exported","Use nyx-backup-setup inspect FILE to review the handoff. Package installation and configuration restoration are separate, explicitly confirmed actions. Nothing was installed or restored by exporting."));}}});}));
            b.append(&actions);
            b.append(&text("Package versions are provenance, not downgrade instructions. The Setup consumer uses the target's configured package repositories. Drivers, kernels, firmware, foreign builds and Flatpaks are held for separate review.","dim-label"));
            let package_rows = w::vbox(6);
            b.append(&package_rows);
            let more = gtk::Button::with_label("Show more packages");
            b.append(&more);
            let offset = Rc::new(Cell::new(0));
            let packages = Rc::new(bundle.packages.clone());
            let populate = {
                let rows = package_rows.clone();
                let more = more.clone();
                let offset = offset.clone();
                move || {
                    let end = (offset.get() + 100).min(packages.len());
                    for package in &packages[offset.get()..end] {
                        rows.append(&text(
                            &format!("{} · {}\n{}", package.name, package.action, package.reason),
                            "",
                        ));
                    }
                    offset.set(end);
                    more.set_visible(end < packages.len());
                }
            };
            let populate = Rc::new(populate);
            populate();
            let pop = populate.clone();
            more.connect_clicked(move |_| pop());
            if !bundle.source.flatpaks.is_empty() {
                b.append(&text("Flatpak applications — manual review", "heading"));
                for app in bundle.source.flatpaks.iter().take(200) {
                    b.append(&text(
                        &format!(
                            "{} · {} · {} · {}",
                            app.application, app.branch, app.architecture, app.origin
                        ),
                        "dim-label",
                    ));
                }
            }
            if !bundle.held_files.is_empty() {
                b.append(&text("Files held for manual review", "heading"));
                for file in bundle.held_files.iter().take(200) {
                    b.append(&text(
                        &format!("{}\n{}", file.path, file.reason),
                        "dim-label",
                    ));
                }
                if bundle.held_files.len() > 200 {
                    b.append(&text(
                        "More held-file details are in the exported bundle.",
                        "dim-label",
                    ));
                }
            }
            d.connect_response(|d, _| d.close());
            d.present();
        },
    );
}
pub fn bundles(ui: &Rc<Ui>, profile: String) {
    let next = profile.clone();
    read(ui, profile, Request::Bundles, move |ui, value| {
        let d = dialog(&ui.window, "Migration / Setup bundles");
        d.add_button("Close", gtk::ResponseType::Close);
        let b = form(&d);
        let values = value["bundles"].as_array().cloned().unwrap_or_default();
        if values.is_empty() {
            b.append(&text(
                "Choose a snapshot and the Migration / Setup mode to prepare a bundle.",
                "dim-label",
            ));
        }
        for bundle in values {
            if let Some(id) = bundle["id"].as_str() {
                let row = card(id, bundle["snapshot"].as_str().unwrap_or(""));
                let u = ui.clone();
                let profile = next.clone();
                let id = id.to_string();
                row.append(&action(
                    "document-properties-symbolic",
                    "Review migration bundle",
                    move || review(&u, profile.clone(), id.clone()),
                ));
                b.append(&row);
            }
        }
        d.connect_response(|d, _| d.close());
        d.present();
    });
}
pub fn inventory(ui: &Rc<Ui>) {
    work(
        ui,
        "Reading package and hardware inventory…",
        || {
            Ok(serde_json::to_string(
                &backup_system::migration_inventory::capture(),
            )?)
        },
        |ui, s| {
            if let Ok(inventory) = serde_json::from_str::<Inventory>(&s) {
                message(
                    ui,
                    "Current migration inventory",
                    &format!(
                        "Architecture: {}\nMachine: {} {}\n{} packages; {} Flatpak apps; {} connected displays\nHome: {}\nConfig: {}\n\nMissing coverage:\n{}\n\nNew backups include a versioned inventory inside their encrypted snapshot metadata. No repository refresh, package installation or configuration change was performed.",
                        inventory.hardware.architecture,
                        inventory.hardware.system_vendor,
                        inventory.hardware.product,
                        inventory.packages.len(),
                        inventory.flatpaks.len(),
                        inventory.hardware.displays.len(),
                        inventory.home.display(),
                        inventory.config.display(),
                        inventory.missing.join("\n")
                    ),
                );
            }
        },
    );
}

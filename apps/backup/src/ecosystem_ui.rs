//! Explicit local setup surfaces; no account discovery or automatic network sync.
use crate::forms::{self, Sheet};
use crate::ui::*;
use backup_core::*;
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::rc::Rc;

pub fn cloud_credentials(ui: &Rc<Ui>, dest: &Destination) {
    let sheet = Sheet::new("Cloud credentials", 460);
    sheet.body.append(&label(
        "Kept in your keyring and an encrypted Rclone file tied to this endpoint, bucket, region and prefix. Saving doesn't connect to anything; test the connection afterwards.",
        "",
    ));
    let access = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let key = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let token = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    forms::field(&sheet.body, "Access key ID", "", &access);
    forms::field(&sheet.body, "Secret access key", "", &key);
    forms::field(
        &sheet.body,
        "Session token",
        "Only for temporary credentials.",
        &token,
    );
    sheet.close_button("Cancel");
    let save = sheet.action("Save", true);
    let (s, u, dest) = (sheet.clone(), ui.clone(), dest.clone());
    save.connect_clicked(move |_| {
        if access.text().is_empty() || key.text().is_empty() {
            s.fail("Enter the access key ID and secret access key");
            return;
        }
        let secrets = (|| -> anyhow::Result<_> {
            Ok((
                nyx_sync::Secret::new(access.text().to_string())?,
                nyx_sync::Secret::new(key.text().to_string())?,
                if token.text().is_empty() {
                    None
                } else {
                    Some(nyx_sync::Secret::new(token.text().to_string())?)
                },
            ))
        })();
        let (a, k, t) = match secrets {
            Ok(v) => v,
            Err(e) => {
                s.fail(&e.to_string());
                return;
            }
        };
        let dest = dest.clone();
        let fields = [access.clone(), key.clone(), token.clone()];
        s.submit(
            &u,
            move || backup_system::setup_s3(&Store::default(), &dest, &a, &k, t.as_ref()),
            move |ui, _| {
                for f in &fields {
                    f.set_text("");
                }
                toast(ui, "Cloud credentials saved. Test the connection next.");
            },
        );
    });
    sheet.present();
}

/// Fill the Settings card's Integrations group.
pub fn render(ui: &Rc<Ui>) {
    let list = &ui.settings.integrations;
    clear(list);
    let config = Store::default().load().unwrap_or_default();
    let grants = backup_service::integration::list(&Store::default()).unwrap_or_default();
    let live: Vec<_> = grants.iter().filter(|g| g.expires > now()).collect();
    if live.is_empty() {
        list.append(&w::row(
            "No apps have access",
            "Control Center and other Nyx apps can show a backup's status once you grant it.",
            None,
        ));
    }
    for g in live {
        let u = ui.clone();
        let id = g.id.clone();
        let revoke = gtk::Button::with_label("Revoke");
        revoke.connect_clicked(move |_| {
            let id = id.clone();
            work(
                &u,
                move || backup_service::integration::revoke(&Store::default(), &id),
                |ui, _| {
                    render(ui);
                    toast(ui, "Access revoked");
                },
            );
        });
        let desc = format!(
            "{} · {} · until {}",
            config
                .profile(&g.profile)
                .map(|p| p.name.clone())
                .unwrap_or(g.profile.clone()),
            g.actions.join(", "),
            when(g.expires)
        );
        list.append(&w::row(
            &g.consumer,
            &gtk::glib::markup_escape_text(&desc),
            Some(revoke.upcast_ref()),
        ));
    }
    let has_profiles = !config.profiles.is_empty();
    let u = ui.clone();
    let grant_button = gtk::Button::with_label("Grant access…");
    grant_button.set_sensitive(has_profiles);
    grant_button.connect_clicked(move |_| grant(&u));
    list.append(&w::row(
        "Give an app access",
        "For one hour, bound to the running app's connection.",
        Some(grant_button.upcast_ref()),
    ));
    let u = ui.clone();
    let sync = gtk::Button::with_label("Upload…");
    sync.set_sensitive(has_profiles);
    sync.connect_clicked(move |_| github_upload(&u));
    list.append(&w::row(
        "Copy settings to GitHub",
        "Uploads one backup's settings to a private repository after you review them. No files or passwords.",
        Some(sync.upcast_ref()),
    ));
    let u = ui.clone();
    let fetch = gtk::Button::with_label("Download…");
    fetch.connect_clicked(move |_| github_download(&u));
    list.append(&w::row(
        "Get settings from GitHub",
        "Saves them to a file for you to review. Nothing is imported or allowed to run.",
        Some(fetch.upcast_ref()),
    ));
}

fn profile_picker(body: &gtk::Box, config: &Config) -> gtk::DropDown {
    let names: Vec<_> = config.profiles.iter().map(|p| p.name.as_str()).collect();
    let d = gtk::DropDown::from_strings(&names);
    forms::field(body, "Backup", "", &d);
    d
}

fn grant(ui: &Rc<Ui>) {
    let config = Store::default().load().unwrap_or_default();
    if config.profiles.is_empty() {
        return;
    }
    let sheet = Sheet::new("Give an app access", 460);
    sheet.body.append(&label(
        "The grant binds to the app's current connection: a restarted app needs a new one. It doesn't protect against other programs running as you.",
        "field-hint",
    ));
    let profiles = profile_picker(&sheet.body, &config);
    let consumer = forms::entry(
        &sheet.body,
        "App",
        "The running app's bus name.",
        "io.github.design_nexus.nyx.ControlCenter",
    );
    let options = w::vbox(4);
    let backup = gtk::CheckButton::with_label("Start this backup");
    let verify = gtk::CheckButton::with_label("Check the repository");
    let cancel = gtk::CheckButton::with_label("Cancel its running job");
    for c in [&backup, &verify, &cancel] {
        options.append(c);
    }
    forms::field(
        &sheet.body,
        "Also allow",
        "Seeing the status is always included.",
        &options,
    );
    sheet.close_button("Cancel");
    let b = sheet.action("Grant for one hour", true);
    let (s, u) = (sheet.clone(), ui.clone());
    b.connect_clicked(move |_| {
        let Some(p) = config.profiles.get(profiles.selected() as usize) else {
            return;
        };
        if !p.approved {
            s.fail("Allow this backup to run before granting access to it");
            return;
        }
        let (profile, who) = (p.id.clone(), consumer.text().trim().to_string());
        let mut actions = vec!["status".to_string()];
        for (on, name) in [
            (backup.is_active(), "backup"),
            (verify.is_active(), "verify"),
            (cancel.is_active(), "cancel"),
        ] {
            if on {
                actions.push(name.into());
            }
        }
        s.submit(
            &u,
            move || {
                backup_service::integration::issue(
                    &Store::default(),
                    &who,
                    &profile,
                    &actions,
                    3600,
                )
            },
            |ui, _| {
                render(ui);
                toast(ui, "Access granted for one hour");
            },
        );
    });
    sheet.present();
}

fn github_upload(ui: &Rc<Ui>) {
    let config = Store::default().load().unwrap_or_default();
    let sheet = Sheet::new("Copy settings to GitHub", 480);
    sheet.body.append(&label(
        "Only this backup's settings and destination details go to the repository, after you review them. Files, passwords, grants and history stay here, and the copy can't run until it's allowed again. The GitHub CLI handles sign-in.",
        "field-hint",
    ));
    let profiles = profile_picker(&sheet.body, &config);
    let repo = forms::entry(&sheet.body, "Private repository", "owner/name", "");
    let branch = forms::entry(&sheet.body, "Branch", "An existing branch.", "main");
    sheet.close_button("Cancel");
    let b = sheet.action("Review upload…", true);
    forms::require(&repo, &b);
    let (s, u) = (sheet.clone(), ui.clone());
    b.connect_clicked(move |_| {
        let Some(p) = config.profiles.get(profiles.selected() as usize) else {
            return;
        };
        let (profile, repo, branch) = (
            p.id.clone(),
            repo.text().trim().to_string(),
            branch.text().trim().to_string(),
        );
        s.submit(
            &u,
            move || {
                backup_service::github_sync::prepare(&Store::default(), &repo, &branch, &[profile])
            },
            |ui, plan| github_review(ui, plan),
        );
    });
    sheet.present();
}

fn github_review(ui: &Rc<Ui>, plan: serde_json::Value) {
    let sheet = Sheet::new("Review the upload", 560);
    sheet.body.append(&label(
        "This is exactly what will be committed to the repository.",
        "field-hint",
    ));
    let code = label(
        &serde_json::to_string_pretty(&plan).unwrap_or_default(),
        "code-block",
    );
    code.set_selectable(true);
    sheet.body.append(&code);
    sheet.close_button("Cancel");
    let b = sheet.action("Upload", true);
    let (s, u) = (sheet.clone(), ui.clone());
    let id = plan["plan"].as_str().unwrap_or("").to_string();
    b.connect_clicked(move |_| {
        let id = id.clone();
        s.submit(
            &u,
            move || backup_service::github_sync::push(&Store::default(), &id, &id),
            |ui, _| toast(ui, "Settings uploaded to the private repository"),
        );
    });
    sheet.present();
}

fn github_download(ui: &Rc<Ui>) {
    let sheet = Sheet::new("Get settings from GitHub", 460);
    let repo = forms::entry(&sheet.body, "Private repository", "owner/name", "");
    let branch = forms::entry(&sheet.body, "Branch", "", "main");
    sheet.close_button("Cancel");
    let b = sheet.action("Save to file…", true);
    forms::require(&repo, &b);
    let (s, u) = (sheet.clone(), ui.clone());
    b.connect_clicked(move |_| {
        let (s, u) = (s.clone(), u.clone());
        let (repo, branch) = (
            repo.text().trim().to_string(),
            branch.text().trim().to_string(),
        );
        let parent = s.parent.clone();
        forms::choose_save(
            &parent,
            "Save the fetched settings",
            "nyx-backup-settings.toml",
            move |path| {
                let (repo, branch) = (repo.clone(), branch.clone());
                s.submit(
                    &u,
                    move || backup_service::github_sync::fetch(&repo, &branch, &path),
                    |ui, _| toast(ui, "Settings saved for review. Nothing was imported."),
                );
            },
        );
    });
    sheet.present();
}

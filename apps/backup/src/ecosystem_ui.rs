//! Explicit local setup surfaces; no account discovery or automatic network sync.
use super::*;
pub fn cloud_credentials(ui: &Rc<Ui>, dest: Destination) {
    let d = dialog(&ui.window, "Encrypted S3 credentials");
    d.add_button("Cancel", gtk::ResponseType::Cancel);
    d.add_button("Save in Secret Service", gtk::ResponseType::Accept);
    let b = form(&d);
    b.append(&text("Credentials are stored in Secret Service and an encrypted Rclone configuration bound to this endpoint, bucket, region and prefix. Repository passwords are separate. Nothing is connected or initialized by saving credentials.","dim-label"));
    let access = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let key = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let token = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    for (label, input) in [
        ("Access key ID", &access),
        ("Secret access key", &key),
        ("Session token (optional)", &token),
    ] {
        b.append(&text(label, ""));
        b.append(input);
    }
    let u = ui.clone();
    d.connect_response(move|d,r|{if r!=gtk::ResponseType::Accept{d.close();return}let result=(||->anyhow::Result<_>{Ok((nyx_sync::Secret::new(access.text().to_string())?,nyx_sync::Secret::new(key.text().to_string())?,if token.text().is_empty(){None}else{Some(nyx_sync::Secret::new(token.text().to_string())?)}))})();match result{Ok((a,k,t))=>{access.set_text("");key.set_text("");token.set_text("");let dest=dest.clone();work(&u,"Saving cloud credentials…",move||{backup_system::setup_s3(&Store::default(),&dest,&a,&k,t.as_ref())?;Ok(String::new())},|u,_|u.notice.set_text("Cloud credentials saved. Test connectivity, then validate or explicitly initialize the repository."));d.close();},Err(e)=>message(&u,"Credentials need attention",&e.to_string())}});
    d.present();
}
pub fn show(ui: &Rc<Ui>) {
    let d = dialog(&ui.window, "GitHub sync and integration grants");
    d.set_default_height(700);
    let b = form(&d);
    b.append(&text("Integration grants", "heading"));
    b.append(&text("Grants bind to a running same-user application's unique D-Bus connection and an approved profile, for one hour. Revocation or profile unapproval takes effect on the next call. Restarted consumers need a new grant. This is not protection from malicious code running as your user.","dim-label"));
    let config = Store::default().load().unwrap_or_default();
    if config.profiles.is_empty() {
        b.append(&text(
            "Create a profile before granting actions or syncing settings.",
            "dim-label",
        ));
    } else {
        let names: Vec<_> = config.profiles.iter().map(|p| p.name.as_str()).collect();
        let profiles = gtk::DropDown::from_strings(&names);
        b.append(&profiles);
        let consumer = entry(
            &b,
            "Running consumer bus name",
            "io.github.design_nexus.nyx.ControlCenter",
        );
        let backup = gtk::CheckButton::with_label("Allow starting this approved backup profile");
        let verify = gtk::CheckButton::with_label("Allow metadata verification");
        let cancel = gtk::CheckButton::with_label("Allow cancelling this profile's job");
        for w in [&backup, &verify, &cancel] {
            b.append(w);
        }
        let u = ui.clone();
        let list = config.profiles.clone();
        let selected = profiles.clone();
        b.append(&action(
            "security-high-symbolic",
            "Grant one-hour status and selected actions to this running connection",
            move || {
                let p = list[selected.selected() as usize].id.clone();
                let consumer = consumer.text().to_string();
                let mut actions = vec!["status".into()];
                for (on, name) in [
                    (backup.is_active(), "backup"),
                    (verify.is_active(), "verify"),
                    (cancel.is_active(), "cancel"),
                ] {
                    if on {
                        actions.push(name.into());
                    }
                }
                work(
                    &u,
                    "Authorizing running consumer…",
                    move || {
                        Ok(serde_json::to_string_pretty(
                            &backup_service::integration::issue(
                                &Store::default(),
                                &consumer,
                                &p,
                                &actions,
                                3600,
                            )?,
                        )?)
                    },
                    |u, s| message(u, "Integration grant created", &s),
                );
            },
        ));
        for grant in backup_service::integration::list(&Store::default()).unwrap_or_default() {
            let row = card(
                &grant.consumer,
                &format!(
                    "Profile {} · expires {} · connection {}",
                    grant.profile, grant.expires, grant.owner
                ),
            );
            let u = ui.clone();
            let id = grant.id.clone();
            row.append(&action(
                "edit-delete-symbolic",
                "Revoke this integration grant",
                move || match backup_service::integration::revoke(&Store::default(), &id) {
                    Ok(()) => u.notice.set_text("Grant revoked"),
                    Err(e) => message(&u, "Grant needs attention", &e.to_string()),
                },
            ));
            b.append(&row);
        }
        b.append(&text("GitHub Backup settings sync", "heading"));
        b.append(&text("Select one profile. Preview publishes only its Backup settings and destination metadata to an existing private GitHub repository. File contents, passwords, grants and job history are excluded; execution approval and recognized-secret inclusion are removed. GitHub CLI handles authentication through its existing credential store. No automatic sync runs.","dim-label"));
        let repo = entry(&b, "Private GitHub owner/repository", "");
        let branch = entry(&b, "Existing branch", "main");
        let u = ui.clone();
        let r = repo.clone();
        let br = branch.clone();
        let selected = profiles.clone();
        let list = config.profiles.clone();
        b.append(&action("document-send-symbolic","Preview the selected profile's GitHub settings upload",move||{let repo=r.text().to_string();let branch=br.text().to_string();let profile=list[selected.selected() as usize].id.clone();work(&u,"Preparing GitHub upload preview…",move||Ok(backup_service::github_sync::prepare(&Store::default(),&repo,&branch,&[profile])?.to_string()),|u,s|{let Ok(value)=serde_json::from_str::<serde_json::Value>(&s)else{return};let review=dialog(&u.window,"Review private GitHub upload");review.add_button("Cancel",gtk::ResponseType::Cancel);review.add_button("Upload reviewed settings",gtk::ResponseType::Accept);form(&review).append(&text(&serde_json::to_string_pretty(&value).unwrap_or_default(),""));let u=u.clone();review.connect_response(move|d,r|{if r==gtk::ResponseType::Accept{let id=value["plan"].as_str().unwrap_or("").to_string();work(&u,"Uploading reviewed Backup settings…",move||Ok(backup_service::github_sync::push(&Store::default(),&id,&id)?.to_string()),|u,_|u.notice.set_text("Reviewed settings uploaded to the selected private repository"));}d.close();});review.present();});}));
        let u = ui.clone();
        let parent = d.clone();
        b.append(&action("document-save-symbolic","Download settings into a new file for review; do not apply them",move||{let u=u.clone();let repo=repo.text().to_string();let branch=branch.text().to_string();gtk::FileDialog::builder().title("Save fetched Backup settings for review").initial_name("nyx-backup-settings.toml").build().save(Some(&parent),gio::Cancellable::NONE,move|result|{if let Ok(file)=result{if let Some(path)=file.path(){work(&u,"Fetching Backup settings…",move||Ok(backup_service::github_sync::fetch(&repo,&branch,&path)?.to_string()),|u,_|u.notice.set_text("Settings saved for review. Nothing was imported or approved."));}}});}));
    }
    d.connect_response(|d, _| d.close());
    d.present();
}

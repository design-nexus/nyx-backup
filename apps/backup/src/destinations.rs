//! Destinations: where repositories live, their credentials, and connecting
//! or creating the repository. Listed in the Settings card.
use crate::forms::{self, Sheet};
use crate::ui::*;
use backup_core::*;
use gtk::prelude::*;
use nyx_ui::widgets as w;
use std::{path::PathBuf, rc::Rc};

const KINDS: &[&str] = &[
    "Folder on this computer",
    "External drive",
    "NFS share",
    "SFTP server",
    "SMB share (NAS)",
    "S3-compatible storage",
];

/// Fill the Settings card's Destinations group.
pub fn render(ui: &Rc<Ui>) {
    let list = &ui.settings.destinations;
    clear(list);
    let c = Store::default().load().unwrap_or_default();
    if c.destinations.is_empty() {
        let row = w::row(
            "No destinations yet",
            "Add a drive, folder, NAS, SFTP server or S3 bucket to back up to.",
            None,
        );
        list.append(&row);
    }
    for d in &c.destinations {
        let used: Vec<_> = c
            .profiles
            .iter()
            .filter(|p| p.destination == d.id)
            .map(|p| p.name.clone())
            .collect();
        let desc = if used.is_empty() {
            destination_summary(d)
        } else {
            format!("{} · used by {}", destination_summary(d), used.join(", "))
        };
        let m = destination_menu(ui, d);
        let row = w::row(
            &d.name,
            &gtk::glib::markup_escape_text(&desc),
            Some(m.upcast_ref()),
        );
        list.append(&row);
    }
    let add = gtk::Button::with_label("Add destination…");
    let u = ui.clone();
    add.connect_clicked(move |_| edit(&u, None));
    let row = w::row(
        "New destination",
        "Each destination holds one encrypted Restic repository.",
        Some(add.upcast_ref()),
    );
    list.append(&row);
}

fn destination_menu(ui: &Rc<Ui>, d: &Destination) -> gtk::MenuButton {
    let item = |f: fn(&Rc<Ui>, &Destination)| -> Box<dyn Fn()> {
        let ui = ui.clone();
        let d = d.clone();
        Box::new(move || f(&ui, &d))
    };
    let mut items: Vec<MenuItem> = vec![
        ("Edit…", false, item(|ui, d| edit(ui, Some(d.clone())))),
        ("Test connection", false, item(test)),
        ("Connect repository…", false, item(connect)),
        ("-", false, Box::new(|| {})),
        ("Repository password…", false, item(password)),
        ("Export recovery password…", false, item(export)),
    ];
    match d.kind {
        DestinationKind::S3 { .. } => items.push((
            "Cloud credentials…",
            false,
            item(crate::ecosystem_ui::cloud_credentials),
        )),
        DestinationKind::Smb { .. } => items.push(("NAS credentials…", false, item(smb))),
        DestinationKind::Nfs { .. } => {
            items.push(("Mount share", false, item(|ui, d| nfs(ui, d, true))));
            items.push(("Unmount share", false, item(|ui, d| nfs(ui, d, false))));
        }
        _ => {}
    }
    items.push(("-", false, Box::new(|| {})));
    items.push(("Delete…", true, item(delete)));
    menu("More actions for this destination", items)
}

fn kind_index(d: &Destination) -> u32 {
    match d.kind {
        DestinationKind::Local { .. } => 0,
        DestinationKind::Mounted { .. } => 1,
        DestinationKind::Nfs { .. } => 2,
        DestinationKind::Sftp { .. } => 3,
        DestinationKind::Smb { .. } => 4,
        DestinationKind::S3 { .. } => 5,
    }
}

/// Create or edit a destination. A new one asks for its password next.
pub fn edit(ui: &Rc<Ui>, destination: Option<Destination>) {
    let sheet = Sheet::new(
        if destination.is_some() {
            "Edit destination"
        } else {
            "New destination"
        },
        500,
    );
    let b = &sheet.body;
    let name = forms::entry(
        b,
        "Name",
        "",
        destination
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("Backup drive"),
    );
    let kind = gtk::DropDown::from_strings(KINDS);
    let index = destination.as_ref().map(kind_index).unwrap_or(1);
    kind.set_selected(index);
    forms::field(b, "Type", "", &kind);
    let old = destination
        .as_ref()
        .and_then(|d| serde_json::to_value(d).ok())
        .unwrap_or_default();
    let val = |key: &str| old[key].as_str().unwrap_or("").to_string();

    let path = gtk::Entry::builder()
        .hexpand(true)
        .text(val("path"))
        .build();
    let browse = icon_button("folder-open-symbolic", "Choose the folder", {
        let (path, parent) = (path.clone(), sheet.parent.clone());
        move || {
            let path = path.clone();
            forms::choose_folder(&parent, "Choose the repository folder", move |p| {
                path.set_text(&p.display().to_string())
            });
        }
    });
    let path_row = w::hbox(6);
    path_row.append(&path);
    path_row.append(&browse);
    let path_field = forms::field(b, "Folder", "", &path_row);
    let host = forms::entry(
        b,
        "Server",
        "",
        &if index == 4 {
            val("server")
        } else {
            val("host")
        },
    );
    let share = forms::entry(b, "Share", "", &val("share"));
    let user = forms::entry(b, "Username", "", &val("username"));
    let domain = forms::entry(
        b,
        "Domain",
        "Leave empty unless your NAS uses one.",
        &val("domain"),
    );
    let endpoint = forms::entry(
        b,
        "Endpoint",
        "HTTPS address of the S3 service. Leave empty for Amazon S3.",
        &val("endpoint"),
    );
    let region = forms::entry(
        b,
        "Region",
        "",
        &Some(val("region"))
            .filter(|s| !s.is_empty())
            .unwrap_or("us-east-1".into()),
    );
    let bucket = forms::entry(b, "Bucket", "An existing bucket.", &val("bucket"));
    let mount = forms::entry(
        b,
        "Mount point",
        "Set up by an administrator in /etc/fstab with the user option.",
        &val("mount_point"),
    );
    let source = forms::entry(b, "Export", "server:/path", &val("source"));
    let filesystem = forms::entry(
        b,
        "Filesystem",
        "nfs or nfs4",
        &Some(val("filesystem"))
            .filter(|s| !s.is_empty())
            .unwrap_or("nfs4".into()),
    );
    let guidance = label("", "field-hint");
    b.append(&guidance);
    let taken: Vec<String> = Store::default()
        .load()
        .unwrap_or_default()
        .destinations
        .iter()
        .map(|d| d.id.clone())
        .collect();
    let adv = forms::advanced(b);
    let id = forms::entry(
        &adv,
        "ID",
        "Used by nyx-backupctl and in your keyring. Can't be changed later.",
        &destination
            .as_ref()
            .map(|d| d.id.clone())
            .unwrap_or_else(|| slug("Backup drive", &taken)),
    );
    id.set_sensitive(destination.is_none());
    if destination.is_none() {
        let id = id.clone();
        name.connect_changed(move |n| id.set_text(&slug(&n.text(), &taken)));
    }
    let update = {
        let show = |e: &gtk::Entry, on: bool| {
            if let Some(g) = e.parent() {
                g.set_visible(on);
            }
        };
        let (path_field, browse, guidance) = (path_field.clone(), browse.clone(), guidance.clone());
        let (host, share, user, domain) =
            (host.clone(), share.clone(), user.clone(), domain.clone());
        let (endpoint, region, bucket) = (endpoint.clone(), region.clone(), bucket.clone());
        let (mount, source, filesystem) = (mount.clone(), source.clone(), filesystem.clone());
        move |i: u32| {
            show(&host, i == 3 || i == 4);
            for e in [&share, &user, &domain] {
                show(e, i == 4);
            }
            for e in [&endpoint, &region, &bucket] {
                show(e, i == 5);
            }
            for e in [&mount, &source, &filesystem] {
                show(e, i == 2);
            }
            browse.set_visible(i <= 2);
            if let Some(l) = path_field.first_child().and_downcast::<gtk::Label>() {
                l.set_text(match i {
                    3 => "Folder on the server",
                    4 => "Folder in the share",
                    5 => "Prefix in the bucket",
                    _ => "Folder",
                });
            }
            if let Some(l) = host
                .parent()
                .and_then(|g| g.first_child())
                .and_downcast::<gtk::Label>()
            {
                l.set_text(if i == 3 { "SSH host" } else { "Server" });
            }
            guidance.set_text(match i {
                0 => "A folder on this computer's own disk. It doesn't protect against the disk failing; prefer an external drive.",
                1 => "Saving remembers this exact drive, so backups never go to an empty mount point when it's unplugged.",
                2 => "Backup mounts and unmounts a share your administrator already set up. It never edits system files.",
                3 => "Uses your SSH configuration and agent. Accept the server's host key with ssh first.",
                4 => "Set the NAS password after saving. It's kept in your keyring and an encrypted Rclone file.",
                _ => "Set the access keys after saving. They're kept in your keyring and an encrypted Rclone file.",
            });
        }
    };
    update(index);
    kind.connect_selected_notify(move |k| update(k.selected()));
    sheet.close_button("Cancel");
    let save = sheet.action(if destination.is_some() { "Save" } else { "Add" }, true);
    forms::require(&name, &save);
    let s = sheet.clone();
    let u = ui.clone();
    let editing = destination.is_some();
    let focus = name.clone();
    save.connect_clicked(move |_| {
        let text = |e: &gtk::Entry| e.text().trim().to_string();
        let (i, p) = (kind.selected(), text(&path));
        let fields = (
            text(&host),
            text(&share),
            text(&user),
            text(&domain),
            text(&endpoint),
            text(&region),
            text(&bucket),
            text(&mount),
            text(&source),
            text(&filesystem),
        );
        let (id, name) = (text(&id), text(&name));
        if p.is_empty() && i <= 3 {
            s.fail("Choose the folder");
            return;
        }
        s.submit(
            &u,
            move || {
                let (host, share, user, domain, endpoint, region, bucket, mount, source, fs) = fields;
                let pathbuf = PathBuf::from(&p);
                let kind = match i {
                    0 => DestinationKind::Local { path: pathbuf },
                    1 => {
                        anyhow::ensure!(pathbuf.is_absolute(), "Choose the folder on the drive");
                        let mut ancestor = pathbuf.as_path();
                        while !ancestor.exists() {
                            ancestor = ancestor
                                .parent()
                                .ok_or_else(|| anyhow::anyhow!("Connect the drive and choose a folder on it"))?;
                        }
                        let m = backup_system::mount_identity(ancestor)?;
                        anyhow::ensure!(
                            m["target"] != "/",
                            "This folder is on the system disk, not an external drive. Choose “Folder on this computer” instead."
                        );
                        DestinationKind::Mounted {
                            path: pathbuf,
                            mount_point: PathBuf::from(m["target"].as_str().unwrap_or("")),
                            source: m["source"].as_str().unwrap_or("").into(),
                            filesystem: m["fstype"].as_str().unwrap_or("").into(),
                            uuid: m["uuid"].as_str().unwrap_or("").into(),
                        }
                    }
                    2 => DestinationKind::Nfs {
                        path: pathbuf,
                        mount_point: PathBuf::from(mount),
                        source,
                        filesystem: fs,
                    },
                    3 => DestinationKind::Sftp { host, path: p },
                    4 => DestinationKind::Smb {
                        server: host,
                        share,
                        path: p,
                        username: user,
                        domain,
                    },
                    _ => DestinationKind::S3 {
                        endpoint,
                        region,
                        bucket,
                        path: p,
                    },
                };
                let dest = Destination { id, name, kind };
                let store = Store::default();
                let mut c = store.load()?;
                if !editing {
                    anyhow::ensure!(
                        !c.destinations.iter().any(|x| x.id == dest.id),
                        "Another destination already uses the ID “{}”",
                        dest.id
                    );
                }
                match c.destinations.iter_mut().find(|x| x.id == dest.id) {
                    Some(old) => *old = dest.clone(),
                    None => c.destinations.push(dest.clone()),
                }
                store.save(&c)?;
                Ok(dest)
            },
            move |ui, dest| {
                render(ui);
                crate::backups::refresh(ui);
                if !editing {
                    password(ui, &dest);
                } else {
                    toast(ui, &format!("Saved “{}”", dest.name));
                }
            },
        );
    });
    sheet.present();
    forms::focus_later(&focus);
}

/// Save the repository password, or replace it after checking the new one.
pub fn password(ui: &Rc<Ui>, d: &Destination) {
    let d = d.clone();
    let id = d.id.clone();
    let ui2 = ui.clone();
    attempt(
        move || backup_system::has_secret(&id, "repository"),
        move |saved| password_sheet(&ui2, d, saved.unwrap_or(false)),
    );
}

fn password_sheet(ui: &Rc<Ui>, d: Destination, saved: bool) {
    let sheet = Sheet::new(
        if saved {
            "Replace repository password"
        } else {
            "Repository password"
        },
        460,
    );
    sheet.body.append(&label(
        &if saved {
            format!(
                "“{}” already has a saved password. The new one is saved only if it opens the existing repository, so the snapshots stay readable.",
                d.name
            )
        } else {
            format!(
                "For an existing repository, enter its password. For a new one, choose a strong password or generate one. Keep a copy somewhere else: without it, the backups of “{}” can't be read.",
                d.name
            )
        },
        "",
    ));
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    let row = w::hbox(6);
    row.append(&entry);
    if !saved {
        let e = entry.clone();
        row.append(&icon_button(
            "view-refresh-symbolic",
            "Generate a strong password",
            move || {
                if let Ok(s) = backup_system::generated_secret() {
                    e.set_text(s.expose());
                    e.set_show_peek_icon(true);
                }
            },
        ));
    }
    forms::field(&sheet.body, "Password", "", &row);
    sheet.close_button("Cancel");
    let save = sheet.action(if saved { "Replace" } else { "Save" }, true);
    let s = sheet.clone();
    let u = ui.clone();
    save.connect_clicked(move |_| {
        let secret = match nyx_sync::Secret::new(entry.text().to_string()) {
            Ok(secret) if !entry.text().is_empty() => secret,
            _ => {
                s.fail("Enter a password");
                return;
            }
        };
        let d = d.clone();
        let entry = entry.clone();
        s.submit(
            &u,
            move || {
                if saved {
                    backup_engine::check_password(&Store::default(), &d, &secret)?;
                    backup_system::replace_password(&d.id, &secret)?;
                } else {
                    backup_system::save_password(&d.id, &secret)?;
                }
                Ok(d)
            },
            move |ui, d| {
                entry.set_text("");
                crate::backups::refresh(ui);
                toast(ui, "Password saved in your keyring");
                if !saved {
                    remind_export(ui, &d);
                }
            },
        );
    });
    sheet.present();
    forms::focus_later(&sheet.body);
}

fn remind_export(ui: &Rc<Ui>, d: &Destination) {
    let sheet = Sheet::new("Keep a copy of the password", 440);
    sheet.body.append(&label(
        "If this computer is lost, the password is the only way to read the backups. Export it to a file and keep that file somewhere safe, away from this computer and the backups.",
        "",
    ));
    sheet.close_button("Later");
    let b = sheet.action("Export…", true);
    let (win, u, d) = (sheet.clone(), ui.clone(), d.clone());
    b.connect_clicked(move |_| {
        win.close();
        export(&u, &d);
    });
    sheet.present();
}

fn export(ui: &Rc<Ui>, d: &Destination) {
    let (u, id) = (ui.clone(), d.id.clone());
    forms::choose_save(
        &ui.window,
        "Export the recovery password",
        "backup-recovery-password.txt",
        move |path| {
            let id = id.clone();
            work(
                &u,
                move || {
                    backup_system::export_password(&id, &path)?;
                    Ok(())
                },
                |ui, _| {
                    toast(
                        ui,
                        "Recovery password exported. Keep it away from this computer.",
                    )
                },
            );
        },
    );
}

fn test(ui: &Rc<Ui>, d: &Destination) {
    let d = d.clone();
    toast(ui, &format!("Testing “{}”…", d.name));
    work(
        ui,
        move || {
            backup_system::connection_test(&Store::default(), &d)?;
            Ok(d.name.clone())
        },
        |ui, name| toast(ui, &format!("“{name}” is reachable")),
    );
}

/// The profile repository jobs run under: every profile on a destination
/// shares its repository and password.
fn profile_for(d: &Destination) -> Option<Profile> {
    Store::default()
        .load()
        .ok()?
        .profiles
        .into_iter()
        .find(|p| p.destination == d.id)
}

/// Check an existing repository, or explicitly create a new one.
pub fn connect(ui: &Rc<Ui>, d: &Destination) {
    let Some(p) = profile_for(d) else {
        error(
            ui,
            "Choose what to back up first",
            &format!(
                "Repository checks run for a backup that uses “{}”. Create one, then connect the repository.",
                d.name
            ),
        );
        return;
    };
    let sheet = Sheet::new("Connect the repository", 460);
    sheet.body.append(&label(
        &format!(
            "If “{}” already holds a repository, check it with the saved password. Otherwise create a new, empty one there. A failed check never creates anything.",
            d.name
        ),
        "",
    ));
    sheet.close_button("Cancel");
    let create = sheet.action("Create new…", false);
    let check = sheet.action("Check existing", true);
    let (s, u, id) = (sheet.clone(), ui.clone(), p.id.clone());
    check.connect_clicked(move |_| {
        let id = id.clone();
        s.submit(
            &u,
            move || backup_service::admin_start(&id, "validate", None),
            |ui, job| {
                toast(ui, "Checking the repository…");
                track(ui, job, |ui, j| {
                    if j["state"] == "success" {
                        toast(ui, "Repository connected");
                        crate::backups::refresh(ui);
                    } else {
                        crate::backups::result(ui, &j);
                    }
                })
            },
        );
    });
    let (win, u, id, name) = (sheet.clone(), ui.clone(), p.id.clone(), d.name.clone());
    create.connect_clicked(move |_| {
        win.close();
        let (u, id) = (u.clone(), id.clone());
        forms::confirm(
            "Create a new repository?",
            &format!(
                "This creates a new encrypted repository on “{name}” with the saved password. Use it only for a destination that doesn't have one yet."
            ),
            "Create",
            move || {
                let id = id.clone();
                work(
                    &u,
                    move || backup_service::admin_start(&id, "init", None),
                    |ui, job| {
                        toast(ui, "Creating the repository…");
                        track(ui, job, |ui, j| {
                            if j["state"] == "success" {
                                toast(ui, "Repository created");
                                crate::backups::refresh(ui);
                            } else {
                                crate::backups::result(ui, &j);
                            }
                        })
                    },
                );
            },
        );
    });
    sheet.present();
}

fn smb(ui: &Rc<Ui>, d: &Destination) {
    let sheet = Sheet::new("NAS credentials", 440);
    sheet.body.append(&label(
        "The password is kept in your keyring and an encrypted Rclone file tied to this server and account. Set it again after changing the server or username.",
        "",
    ));
    let entry = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    forms::field(&sheet.body, "Password", "", &entry);
    sheet.close_button("Cancel");
    let save = sheet.action("Save", true);
    let (s, u, d) = (sheet.clone(), ui.clone(), d.clone());
    save.connect_clicked(move |_| {
        let Ok(secret) = nyx_sync::Secret::new(entry.text().to_string()) else {
            s.fail("Enter the NAS password");
            return;
        };
        if entry.text().is_empty() {
            s.fail("Enter the NAS password");
            return;
        }
        let d = d.clone();
        let entry = entry.clone();
        s.submit(
            &u,
            move || backup_system::setup_smb(&Store::default(), &d, &secret),
            move |ui, _| {
                entry.set_text("");
                toast(ui, "NAS credentials saved");
            },
        );
    });
    sheet.present();
}

fn nfs(ui: &Rc<Ui>, d: &Destination, connect: bool) {
    let d = d.clone();
    work(
        ui,
        move || {
            let store = Store::default();
            let _guard = backup_engine::RepoLock::acquire(&store)?;
            backup_system::nfs_connection(&d, connect)
        },
        move |ui, _| {
            toast(
                ui,
                if connect {
                    "Share mounted"
                } else {
                    "Share unmounted"
                },
            )
        },
    );
}

fn delete(ui: &Rc<Ui>, d: &Destination) {
    if let Some(p) = profile_for(d) {
        error(
            ui,
            "This destination is in use",
            &format!(
                "“{}” backs up to it. Move or delete that backup first.",
                p.name
            ),
        );
        return;
    }
    let (u, d) = (ui.clone(), d.clone());
    forms::confirm(
        &format!("Delete “{}”?", d.name),
        "Backup forgets this destination. The repository itself and its saved password are kept.",
        "Delete",
        move || {
            let id = d.id.clone();
            work(
                &u,
                move || {
                    let store = Store::default();
                    let mut c = store.load()?;
                    c.destinations.retain(|x| x.id != id);
                    store.save(&c)
                },
                |ui, _| {
                    render(ui);
                    crate::backups::refresh(ui);
                    toast(ui, "Destination deleted");
                },
            );
        },
    );
}

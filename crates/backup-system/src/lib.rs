//! Desktop credentials, mount binding, user timers, and readable metadata.
use anyhow::{Context, Result, ensure};
use backup_core::*;
use nyx_sync::{Secret, lookup_secret, store_secret};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub fn generated_secret() -> Result<Secret> {
    let mut bytes = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Secret::new(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}
pub fn save_password(id: &str, secret: &Secret) -> Result<()> {
    valid_id(id)?;
    store_secret("nyx-backup", id, "repository", secret)
}
pub fn password(id: &str) -> Result<Secret> {
    valid_id(id)?;
    lookup_secret("nyx-backup", id, "repository")
        .context("Unlock Secret Service and save the repository password")
}
pub fn helper(id: &str, kind: &str) -> String {
    format!("secret-tool lookup application nyx-backup account {id} kind {kind}")
}
pub fn export_password(id: &str, path: &Path) -> Result<()> {
    let secret = password(id)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("Choose a new recovery file; existing files are never overwritten")?;
    writeln!(f, "{}", secret.expose())?;
    f.sync_all()?;
    Ok(())
}
pub fn clean_command(program: &str) -> Command {
    let mut c = Command::new(program);
    // An inherited backend/password override must never redirect an approved profile.
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("RESTIC_") || k.starts_with("RCLONE_") || k == "_RCLONE_CONFIG_KEY_FILE" {
            c.env_remove(key);
        }
    }
    c.stdin(Stdio::null());
    c
}
fn checked(mut c: Command, what: &str) -> Result<Vec<u8>> {
    let o = c
        .stderr(Stdio::null())
        .output()
        .with_context(|| format!("Could not start {what}"))?;
    ensure!(
        o.status.success(),
        "{what} failed. Check connectivity, authentication and permissions."
    );
    Ok(o.stdout)
}
pub fn rclone_config(store: &Store, id: &str) -> PathBuf {
    store.config.join("rclone").join(format!("{id}.conf"))
}
pub fn setup_smb(store: &Store, d: &Destination, secret: &Secret) -> Result<()> {
    let DestinationKind::Smb {
        server,
        username,
        domain,
        ..
    } = &d.kind
    else {
        anyhow::bail!("Select an SMB destination")
    };
    d.validate()?;
    store_secret("nyx-backup", &d.id, "smb", secret)?;
    let unlock = generated_secret()?;
    store_secret("nyx-backup", &d.id, "rclone", &unlock)?;
    let mut c = clean_command("rclone");
    c.args(["obscure", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = c.spawn()?;
    writeln!(child.stdin.take().unwrap(), "{}", secret.expose())?;
    let o = child.wait_with_output()?;
    ensure!(
        o.status.success(),
        "Rclone could not encode the SMB credential"
    );
    let obscured = String::from_utf8(o.stdout)?;
    let plain = format!(
        "[nas]\ntype = smb\nhost = {server}\nuser = {username}\ndomain = {domain}\npass = {}\n",
        obscured.trim()
    );
    // Rclone ENCRYPT_V0 format: SHA256([password][rclone-config]), XSalsa20-Poly1305.
    // Our generated unlock key is ASCII, so Rclone's NFKC normalization is identical.
    use base64::Engine;
    use crypto_secretbox::{
        XSalsa20Poly1305,
        aead::{Aead, KeyInit},
    };
    use sha2::{Digest, Sha256};
    let key = Sha256::digest(format!("[{}][rclone-config]", unlock.expose()).as_bytes());
    let cipher = XSalsa20Poly1305::new_from_slice(&key)
        .map_err(|_| anyhow::anyhow!("Invalid encryption key"))?;
    let mut nonce = [0u8; 24];
    fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let encrypted = cipher
        .encrypt((&nonce).into(), plain.as_bytes())
        .map_err(|_| anyhow::anyhow!("Could not encrypt Rclone configuration"))?;
    let mut data = nonce.to_vec();
    data.extend(encrypted);
    atomic(
        &rclone_config(store, &d.id),
        format!(
            "RCLONE_ENCRYPT_V0:\n{}\n",
            base64::engine::general_purpose::STANDARD.encode(data)
        ),
    )?;
    Ok(())
}
pub fn rclone(store: &Store, d: &Destination) -> Result<Command> {
    valid_id(&d.id)?;
    ensure!(
        rclone_config(store, &d.id).exists(),
        "Set up SMB credentials first"
    );
    let mut c = clean_command("rclone");
    c.args(["--config"]).arg(rclone_config(store, &d.id)).args([
        "--password-command",
        &helper(&d.id, "rclone"),
        "--ask-password=false",
        "--contimeout",
        "15s",
        "--timeout",
        "30s",
    ]);
    Ok(c)
}
pub fn smb_path(d: &Destination) -> Result<String> {
    if let DestinationKind::Smb { share, path, .. } = &d.kind {
        Ok(format!("nas:{}/{}", share, path.trim_matches('/')))
    } else {
        anyhow::bail!("Not an SMB destination")
    }
}
pub fn mount_identity(path: &Path) -> Result<serde_json::Value> {
    let mut c = clean_command("findmnt");
    c.args([
        "--json",
        "--output",
        "TARGET,SOURCE,FSTYPE,UUID",
        "--target",
    ])
    .arg(path);
    let v: serde_json::Value = serde_json::from_slice(&checked(c, "Mount lookup")?)?;
    v["filesystems"]
        .get(0)
        .cloned()
        .context("No mounted filesystem at this path")
}
pub fn validate_destination(d: &Destination) -> Result<()> {
    d.validate()?;
    match &d.kind {
        DestinationKind::Mounted {
            path,
            mount_point,
            source,
            filesystem,
            ..
        }
        | DestinationKind::Nfs {
            path,
            mount_point,
            source,
            filesystem,
        } => {
            ensure!(
                mount_point.is_absolute() && path.starts_with(mount_point),
                "Repository must be inside the bound mount"
            );
            let m = mount_identity(mount_point)?;
            ensure!(
                m["target"].as_str() == mount_point.to_str()
                    && m["source"].as_str() == Some(source)
                    && m["fstype"].as_str() == Some(filesystem),
                "The bound device/share is absent or changed. Connect the original destination; refusing to write into an empty mount point."
            );
            if let DestinationKind::Mounted { uuid, .. } = &d.kind {
                ensure!(
                    !source.starts_with("/dev/") || !uuid.is_empty(),
                    "Bind this device again to record its filesystem UUID"
                );
                ensure!(
                    uuid.is_empty() || m["uuid"].as_str() == Some(uuid),
                    "The mounted filesystem UUID changed; refusing a different device"
                );
            }
            // Reject a repository symlink escaping the checked filesystem.
            let mut ancestor = path.as_path();
            while !ancestor.exists() {
                ancestor = ancestor.parent().context("Missing destination parent")?;
            }
            let canonical = fs::canonicalize(ancestor)?;
            ensure!(
                canonical.starts_with(fs::canonicalize(mount_point)?),
                "Repository path escapes the bound mount"
            );
        }
        DestinationKind::Local { path } => {
            let mut ancestor = path.as_path();
            while !ancestor.exists() {
                ancestor = ancestor.parent().context("Missing destination parent")?;
            }
            ensure!(ancestor.is_dir(), "Destination parent is not a directory");
        }
        _ => (),
    }
    Ok(())
}
pub fn connection_test(store: &Store, d: &Destination) -> Result<serde_json::Value> {
    validate_destination(d)?;
    match &d.kind {
        DestinationKind::Smb { .. } => {
            let mut c = rclone(store, d)?;
            c.arg("lsf").arg(smb_path(d)?).args(["--max-depth", "1"]);
            checked(c, "SMB connection")?;
        }
        DestinationKind::Sftp { host, .. } => {
            // Probe the same subsystem used by Restic. SFTP-only accounts may
            // intentionally reject every remote shell command.
            let mut c = clean_command("sftp");
            c.args([
                "-b",
                "/dev/null",
                "-oBatchMode=yes",
                "-oConnectTimeout=15",
                "--",
                host,
            ]);
            checked(c, "SFTP connection")?;
        }
        _ => {
            let p = d.local_path().unwrap();
            let mut a = p;
            while !a.exists() {
                a = a.parent().context("Missing parent")?;
            }
            fs::read_dir(a).context("Destination parent is not readable")?;
        }
    }
    Ok(
        serde_json::json!({"version":1,"connected":true,"repository_validated":false,"message":"Connection only; use Validate repository separately"}),
    )
}
pub fn nfs_connection(d: &Destination, connect: bool) -> Result<()> {
    let DestinationKind::Nfs {
        mount_point,
        source,
        filesystem,
        ..
    } = &d.kind
    else {
        anyhow::bail!("Only administrator-preconfigured NFS mounts can be managed")
    };
    ensure!(
        filesystem == "nfs" || filesystem == "nfs4",
        "Expected an NFS filesystem"
    );
    if connect && validate_destination(d).is_ok() {
        return Ok(());
    }
    let mut c = clean_command("findmnt");
    c.args([
        "--fstab",
        "--json",
        "--output",
        "TARGET,SOURCE,FSTYPE,OPTIONS",
        "--target",
    ])
    .arg(mount_point);
    let v: serde_json::Value = serde_json::from_slice(&checked(c, "NFS fstab lookup")?)?;
    let m = &v["filesystems"][0];
    ensure!(
        m["target"].as_str() == mount_point.to_str()
            && m["source"].as_str() == Some(source)
            && m["fstype"].as_str() == Some(filesystem),
        "Ask an administrator to configure this exact NFS share in fstab"
    );
    let options = m["options"].as_str().unwrap_or("");
    ensure!(
        options.split(',').any(|s| s == "user" || s == "users"),
        "NFS share must be user-mountable in fstab"
    );
    if !connect {
        validate_destination(d)?;
    }
    let mut c = clean_command(if connect { "mount" } else { "umount" });
    c.arg("--").arg(mount_point);
    checked(c, "NFS mount operation")?;
    if connect {
        validate_destination(d)?;
    }
    Ok(())
}
pub fn inventory() -> serde_json::Value {
    let home = nyx_core::paths::home();
    let personal: Vec<_> = ["Documents", "Pictures", "Music", "Videos", "Desktop"]
        .iter()
        .map(|p| home.join(p))
        .filter(|p| p.exists())
        .collect();
    let curated: Vec<_> = ["hypr", "omarchy", "nyx", "foot", "kitty", "alacritty"]
        .iter()
        .map(|p| nyx_core::paths::config_home().join(p))
        .filter(|p| p.exists())
        .collect();
    let unknown: Vec<_> = fs::read_dir(nyx_core::paths::config_home())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| !curated.contains(p))
        .collect();
    serde_json::json!({"version":1,"personal_data":personal,"curated_configuration":curated,"other_configuration_for_explicit_selection":unknown,"warning":"Any file can contain secrets. Recognized keys and credential stores are excluded unless explicitly authorized."})
}
pub fn metadata() -> serde_json::Value {
    let mut v = serde_json::json!({"version":1,"coverage":"Readable metadata only; no complete system recovery","collected":{},"missing":[]});
    for (name, args) in [
        ("os", vec!["cat", "/etc/os-release"]),
        ("kernel", vec!["uname", "-a"]),
        ("filesystems", vec!["lsblk", "--json", "--fs"]),
        ("mounts", vec!["findmnt", "--json"]),
        ("boot", vec!["cat", "/proc/cmdline"]),
        ("omarchy", vec!["omarchy", "version"]),
    ] {
        let mut c = clean_command(args[0]);
        c.args(&args[1..]);
        match checked(c, name) {
            Ok(s) => v["collected"][name] = String::from_utf8_lossy(&s).to_string().into(),
            Err(_) => v["missing"].as_array_mut().unwrap().push(name.into()),
        }
    }
    v
}
pub fn schedule(store: &Store, p: &Profile, enable: bool) -> Result<()> {
    valid_id(&p.id)?;
    let name = format!("nyx-backup-{}", p.id);
    if !enable {
        let mut c = clean_command("systemctl");
        c.args(["--user", "disable", "--now", &format!("{name}.timer")]);
        checked(c, "Disable schedule")?;
        return Ok(());
    }
    ensure!(p.approved, "Approve the profile before scheduling it");
    let calendar = p
        .schedule
        .as_ref()
        .context("Set an OnCalendar schedule first")?;
    ensure!(
        !calendar.contains(['\n', '\r', '%']) && calendar.len() < 120,
        "Invalid calendar expression"
    );
    let mut c = clean_command("systemd-analyze");
    c.args(["calendar", "--", calendar]);
    checked(c, "Calendar validation")?;
    let dir = nyx_core::paths::config_home().join("systemd/user");
    let ctl = std::env::current_exe()?.with_file_name("nyx-backupctl");
    ensure!(
        !ctl.to_string_lossy().contains(['\n', '\r', '%', '"', '\\']),
        "Unsupported installation path"
    );
    atomic(
        &dir.join(format!("{name}.service")),
        format!(
            "[Unit]\nDescription=Nyx Backup profile {}\n[Service]\nType=oneshot\nExecStart=\"{}\" backup {} --wait\n",
            p.id,
            ctl.display(),
            p.id
        ),
    )?;
    atomic(
        &dir.join(format!("{name}.timer")),
        format!(
            "[Unit]\nDescription=Nyx Backup schedule {}\n[Timer]\nOnCalendar={calendar}\nPersistent=true\nRandomizedDelaySec=120\n[Install]\nWantedBy=timers.target\n",
            p.id
        ),
    )?;
    for args in [
        vec!["--user", "daemon-reload"],
        vec!["--user", "enable", "--now", &format!("{name}.timer")],
    ] {
        let mut c = clean_command("systemctl");
        c.args(args);
        checked(c, "Enable schedule")?;
    }
    let _ = store;
    Ok(())
}
pub fn schedules() -> serde_json::Value {
    let mut c = clean_command("systemctl");
    c.args([
        "--user",
        "list-timers",
        "--all",
        "--output=json",
        "nyx-backup-*.timer",
    ]);
    checked(c, "Schedule status")
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::json!([]))
}
pub fn notify(success: bool) {
    // No source paths, repository locations, or child diagnostics enter notifications.
    let _ = clean_command("notify-send")
        .args([
            "--app-name=Nyx Backup",
            if success {
                "Backup completed"
            } else {
                "Backup needs attention"
            },
            "Open Nyx Backup to review the result.",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

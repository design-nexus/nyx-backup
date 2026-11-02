//! Explicit, staged file restoration. Never exposed on the integration D-Bus API.
mod fs_safe;
use anyhow::{Context, Result, ensure};
use backup_core::*;
use backup_engine::RepoLock;
use fs_safe::{Fingerprint, Parent};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const TEXT_LIMIT: u64 = 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    Skip,
    KeepBoth,
    Replace,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Separate,
    OriginalDotfiles,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Snapshots,
    Browse {
        snapshot: String,
        #[serde(default = "root_path")]
        directory: String,
    },
    Plan {
        snapshot: String,
        paths: Vec<String>,
        target: Option<PathBuf>,
        #[serde(default)]
        mode: Mode,
        #[serde(default)]
        policy: Policy,
        #[serde(default)]
        include_secrets: bool,
    },
    Inspect {
        plan: String,
    },
    Receipts,
    Diff {
        plan: String,
        path: String,
    },
    Apply {
        plan: String,
        confirmation: String,
    },
    RollbackPreview {
        plan: String,
    },
    Rollback {
        plan: String,
        confirmation: String,
    },
}
fn root_path() -> String {
    "/".into()
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Browse {
                snapshot,
                directory,
            } => {
                snapshot_id(snapshot)?;
                archive_path(directory)?;
            }
            Self::Plan {
                snapshot,
                paths,
                target,
                mode,
                ..
            } => {
                snapshot_id(snapshot)?;
                ensure!(
                    !paths.is_empty() && paths.len() <= 256,
                    "Select 1–256 files or folders"
                );
                for p in paths {
                    archive_path(p)?;
                }
                if mode == &Mode::Separate {
                    fs_safe::clean_absolute(
                        target
                            .as_ref()
                            .context("Choose a separate restore folder")?,
                    )?;
                }
            }
            Self::Inspect { plan } | Self::Diff { plan, .. } | Self::RollbackPreview { plan } => {
                valid_id(plan)?
            }
            Self::Apply { plan, confirmation } | Self::Rollback { plan, confirmation } => {
                valid_id(plan)?;
                ensure!(plan == confirmation, "Confirm the exact restore plan ID");
            }
            _ => (),
        }
        Ok(())
    }
    pub fn is_job(&self) -> bool {
        matches!(
            self,
            Self::Plan { .. } | Self::Apply { .. } | Self::Rollback { .. }
        )
    }
    pub fn operation(&self) -> &'static str {
        match self {
            Self::Plan { .. } => "restore-plan",
            Self::Apply { .. } => "restore-apply",
            Self::Rollback { .. } => "restore-rollback",
            _ => "restore-read",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub mode: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub source: String,
    pub target: PathBuf,
    pub before: Option<Fingerprint>,
    pub after: Fingerprint,
    pub action: String,
    pub status: String,
    pub previous: PathBuf,
    pub incoming: PathBuf,
    pub undone: PathBuf,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub id: String,
    pub profile: String,
    pub snapshot: String,
    pub created: u64,
    pub mode: Mode,
    pub policy: Policy,
    pub root: PathBuf,
    pub state: String,
    pub entries: Vec<Entry>,
    pub empty_directories: Vec<PathBuf>,
    pub created_directories: Vec<PathBuf>,
    pub messages: Vec<String>,
}
#[derive(Debug)]
struct Cancelled;
impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Restore job cancelled")
    }
}
impl std::error::Error for Cancelled {}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Cancelled.into());
    }
    Ok(())
}
fn snapshot_id(s: &str) -> Result<()> {
    ensure!(
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Choose an exact snapshot ID from this profile's history"
    );
    Ok(())
}
fn archive_path(s: &str) -> Result<PathBuf> {
    let path = PathBuf::from(s);
    fs_safe::clean_absolute(&path)?;
    Ok(path)
}
fn plan_dir(store: &Store, id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    Ok(store.state.join("restore").join(id))
}
fn save(store: &Store, plan: &Plan) -> Result<()> {
    atomic(
        &plan_dir(store, &plan.id)?.join("plan.json"),
        serde_json::to_vec_pretty(plan)?,
    )
}
fn load(store: &Store, profile: &str, id: &str) -> Result<Plan> {
    let plan: Plan = serde_json::from_slice(&fs::read(plan_dir(store, id)?.join("plan.json"))?)?;
    ensure!(
        plan.version == 1 && plan.id == id && plan.profile == profile,
        "Restore plan does not belong to this profile"
    );
    Ok(plan)
}
fn stage_path(store: &Store, id: &str, source: &str) -> Result<PathBuf> {
    Ok(plan_dir(store, id)?
        .join("stage")
        .join(archive_path(source)?.strip_prefix("/")?))
}
fn terminate(child: &mut std::process::Child) {
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
}
fn capture(mut command: Command, cancel: &AtomicBool) -> Result<Vec<u8>> {
    check(cancel)?;
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().context("Could not start Restic")?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut out = out;
        loop {
            let mut bytes = vec![0u8; 16384];
            match out.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    bytes.truncate(n);
                    if send.send(Ok(bytes)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = send.send(Err(e));
                    break;
                }
            }
        }
    });
    thread::spawn(move || {
        let _ = std::io::copy(&mut BufReader::new(err), &mut std::io::sink());
    });
    let mut output = Vec::new();
    loop {
        if cancel.load(Ordering::SeqCst) {
            terminate(&mut child);
            return Err(Cancelled.into());
        }
        match receive.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(bytes)) => {
                output.extend(bytes);
                if output.len() > OUTPUT_LIMIT {
                    terminate(&mut child);
                    anyhow::bail!("Snapshot result is too large; select a smaller folder")
                }
            }
            Ok(Err(e)) => {
                terminate(&mut child);
                return Err(e.into());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    check(cancel)?;
    ensure!(
        status.success(),
        "Restic operation failed (exit {:?}). Check repository access, credentials and locks; no replacement repository was initialized.",
        status.code()
    );
    Ok(output)
}
fn command(store: &Store, c: &Config, p: &Profile) -> Result<Command> {
    backup_engine::restic_command(store, c, p)
}
fn owned_snapshot(
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    snapshot_id(snapshot)?;
    let mut cmd = command(store, c, p)?;
    cmd.args(["cat", "snapshot", snapshot]);
    let value: serde_json::Value = serde_json::from_slice(&capture(cmd, cancel)?)?;
    let tags = value["tags"]
        .as_array()
        .context("Snapshot has no Backup ownership tags")?;
    ensure!(
        tags.iter().any(|t| t == "nyx-backup")
            && tags.iter().any(|t| t == &format!("profile:{}", p.id)),
        "Snapshot does not belong to this Backup profile"
    );
    Ok(())
}
fn nodes(
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    directories: &[String],
    recursive: bool,
    cancel: &AtomicBool,
) -> Result<Vec<Node>> {
    let mut cmd = command(store, c, p)?;
    cmd.args(["ls", snapshot]);
    if recursive {
        cmd.arg("--recursive");
    }
    for dir in directories {
        archive_path(dir)?;
        cmd.arg(dir);
    }
    let bytes = capture(cmd, cancel)?;
    let mut nodes = Vec::new();
    for line in BufReader::new(bytes.as_slice()).lines() {
        check(cancel)?;
        let value: serde_json::Value = serde_json::from_str(&line?)?;
        let Some(path) = value["path"].as_str() else {
            continue;
        };
        archive_path(path)?;
        let Some(kind) = value["type"].as_str() else {
            continue;
        };
        nodes.push(Node {
            path: path.into(),
            kind: kind.into(),
            size: value["size"].as_u64().unwrap_or(0),
            mode: value["mode"].as_u64().unwrap_or(0) as u32,
        });
        ensure!(nodes.len() <= 250_000, "Select a smaller snapshot folder");
    }
    Ok(nodes)
}
fn protected(store: &Store, c: &Config, path: &Path) -> Result<()> {
    fs_safe::clean_absolute(path)?;
    let mut paths = vec![
        store.config.clone(),
        store.state.clone(),
        store.cache.clone(),
    ];
    paths.extend(
        c.destinations
            .iter()
            .filter_map(|d| d.local_path().map(Path::to_path_buf)),
    );
    for forbidden in paths {
        let forbidden = fs_safe::resolved(&forbidden)?;
        ensure!(
            !path.starts_with(&forbidden),
            "Restore target is inside Backup's working files or a configured repository"
        );
    }
    Ok(())
}
fn quick_target(source: &Path, include_secrets: bool) -> Result<PathBuf> {
    ensure!(
        include_secrets || !backup_engine::contains_recognized_secrets(source),
        "Recognized keys and credential stores require explicit restore authorization"
    );
    ensure!(
        !backup_engine::contains_recognized_store(source),
        "Quick Dotfiles Restore excludes database/container/VM stores; restore a separate copy and import it using the application"
    );
    let home = nyx_core::paths::home();
    let config = nyx_core::paths::config_home();
    for prefix in [&config, &fs_safe::resolved(&config)?] {
        if let Ok(relative) = source.strip_prefix(prefix) {
            return Ok(fs_safe::resolved(&config)?.join(relative));
        }
    }
    for prefix in [&home, &fs_safe::resolved(&home)?] {
        if let Ok(relative) = source.strip_prefix(prefix) {
            ensure!(
                relative
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str().to_string_lossy().starts_with('.')),
                "Quick Restore only accepts this user's dotfiles and configuration"
            );
            return Ok(fs_safe::resolved(&home)?.join(relative));
        }
    }
    anyhow::bail!(
        "Quick Restore only accepts this computer's home/configuration paths; use a separate folder for other paths"
    )
}
fn mapped_target(
    store: &Store,
    c: &Config,
    mode: &Mode,
    root: &Path,
    source: &str,
    include_secrets: bool,
) -> Result<PathBuf> {
    let source = archive_path(source)?;
    ensure!(
        include_secrets || !backup_engine::contains_recognized_secrets(&source),
        "Recognized keys and credential stores require explicit restore authorization"
    );
    let path = match mode {
        Mode::Separate => root.join(source.strip_prefix("/")?),
        Mode::OriginalDotfiles => quick_target(&source, include_secrets)?,
    };
    protected(store, c, &path)?;
    Ok(path)
}
fn escaped(pattern: &str) -> String {
    let mut out = String::new();
    for c in pattern.chars() {
        if "\\[]*?".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn related(source: &Path, selected: &[String]) -> bool {
    selected.iter().any(|p| source.starts_with(p))
}
fn make_plan(
    store: &Store,
    c: &Config,
    p: &Profile,
    request: &Request,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<Plan> {
    let Request::Plan {
        snapshot,
        paths,
        target,
        mode,
        policy,
        include_secrets,
    } = request
    else {
        anyhow::bail!("Expected a restore plan")
    };
    owned_snapshot(store, c, p, snapshot, cancel)?;
    let root = match mode {
        Mode::Separate => fs_safe::resolved(target.as_ref().context("Choose a target folder")?)?,
        Mode::OriginalDotfiles => fs_safe::resolved(&nyx_core::paths::home())?,
    };
    ensure!(
        mode != &Mode::Separate || root != Path::new("/"),
        "Choose a separate folder, not the filesystem root; full-system restoration is outside this phase"
    );
    protected(store, c, &root)?;
    let selection = nodes(store, c, p, snapshot, paths, true, cancel)?;
    let mut selected: Vec<_> = selection
        .into_iter()
        .filter(|n| related(Path::new(&n.path), paths))
        .collect();
    // Restic ls directory filters do not return a selected regular-file node on every backend.
    if selected.is_empty() {
        let all = nodes(store, c, p, snapshot, &["/".into()], true, cancel)?;
        selected = all
            .into_iter()
            .filter(|n| related(Path::new(&n.path), paths))
            .collect();
    }
    for source in paths {
        ensure!(
            selected
                .iter()
                .any(|n| n.path == *source || Path::new(&n.path).starts_with(source)),
            "A selected path is absent from this snapshot: {source}"
        );
    }
    ensure!(!selected.is_empty(), "No files selected");
    for node in &selected {
        ensure!(
            ["file", "dir", "symlink"].contains(&node.kind.as_str()),
            "Special files are not restored by this phase"
        );
        let _ = mapped_target(store, c, mode, &root, &node.path, *include_secrets)?;
    }
    let id = id();
    let directory = plan_dir(store, &id)?;
    private_dir(&directory)?;
    let stage = directory.join("stage");
    private_dir(&stage)?;
    let mut cmd = command(store, c, p)?;
    cmd.args(["restore", snapshot, "--target"])
        .arg(&stage)
        .args(["--verify", "--overwrite", "never", "--exclude-xattr", "*"]);
    for path in paths {
        cmd.arg("--include").arg(escaped(path));
    }
    job.progress = serde_json::json!({"stage":"staging","selected_paths":paths.len()});
    store.write_job(job)?;
    changed(job);
    capture(cmd, cancel)?;
    let mut plan=Plan{version:1,id,profile:p.id.clone(),snapshot:snapshot.clone(),created:now(),mode:mode.clone(),policy:policy.clone(),root,state:"ready".into(),entries:vec![],empty_directories:vec![],created_directories:vec![],messages:vec!["Files are staged in private local storage. Applying restores file contents, symlinks, modes and modification times. Ownership, ACLs and extended attributes are not applied. Close apps using files before replacing their configuration.".into()]};
    let mut selected_paths = std::collections::HashSet::new();
    for node in selected {
        check(cancel)?;
        let source = stage_path(store, &plan.id, &node.path)?;
        if node.kind == "dir" {
            if fs::read_dir(&source).is_ok_and(|mut r| r.next().is_none()) {
                plan.empty_directories.push(mapped_target(
                    store,
                    c,
                    mode,
                    &plan.root,
                    &node.path,
                    *include_secrets,
                )?);
            }
            continue;
        }
        ensure!(
            selected_paths.insert(node.path.clone()),
            "Snapshot returned duplicate paths"
        );
        let after = fs_safe::fingerprint(&source)?.context("A selected staged file is missing")?;
        if mode == &Mode::OriginalDotfiles && after.kind == "file" {
            let mut magic = [0u8; 16];
            let mut file = Parent::open(&source, false, &mut vec![])?.read_file()?;
            ensure!(
                file.read_exact(&mut magic).is_err() || &magic != b"SQLite format 3\0",
                "Quick Restore excludes SQLite stores; restore a separate copy"
            );
        }
        let mut target = mapped_target(store, c, mode, &plan.root, &node.path, *include_secrets)?;
        let mut before = fs_safe::fingerprint(&target)?;
        let action = if before.as_ref() == Some(&after) {
            "unchanged"
        } else if before.is_some() {
            match policy {
                Policy::Skip => "skip",
                Policy::Replace => "replace",
                Policy::KeepBoth => {
                    let name = target
                        .file_name()
                        .context("Missing file name")?
                        .to_os_string();
                    let mut keep = name;
                    keep.push(format!(".restored-{}-{}", &snapshot[..8], &plan.id[..8]));
                    target.set_file_name(keep);
                    before = fs_safe::fingerprint(&target)?;
                    ensure!(
                        before.is_none(),
                        "Keep-both destination exists; create a new plan"
                    );
                    "create"
                }
            }
        } else {
            "create"
        };
        let parent = target.parent().context("Missing parent")?;
        let index = plan.entries.len();
        plan.entries.push(Entry {
            source: node.path,
            target: target.clone(),
            before,
            after,
            action: action.into(),
            status: "pending".into(),
            previous: parent.join(format!(".nyx-restore-{}-{index}.previous", plan.id)),
            incoming: parent.join(format!(".nyx-restore-{}-{index}.incoming", plan.id)),
            undone: parent.join(format!(".nyx-restore-{}-{index}.undone", plan.id)),
            message: String::new(),
        });
    }
    ensure!(
        !plan.entries.is_empty() || !plan.empty_directories.is_empty(),
        "No restorable entries selected"
    );
    ensure!(
        serde_json::to_vec(&plan)?.len() <= 12 * 1024 * 1024,
        "Restore plan is too large to review; select a smaller folder"
    );
    save(store, &plan)?;
    Ok(plan)
}
fn validate_plan(store: &Store, c: &Config, plan: &Plan) -> Result<()> {
    fs_safe::clean_absolute(&plan.root)?;
    for entry in &plan.entries {
        protected(store, c, &entry.target)?;
        match plan.mode {
            Mode::Separate => ensure!(
                entry.target.starts_with(&plan.root) && entry.target != plan.root,
                "Restore entry escapes its selected root"
            ),
            Mode::OriginalDotfiles => ensure!(
                quick_target(&archive_path(&entry.source)?, true)? == entry.target,
                "Original dotfile target changed"
            ),
        }
        let parent = entry.target.parent().context("Missing target parent")?;
        for reserved in [&entry.previous, &entry.incoming, &entry.undone] {
            ensure!(
                reserved.parent() == Some(parent)
                    && reserved.file_name().is_some_and(|n| n
                        .to_string_lossy()
                        .starts_with(&format!(".nyx-restore-{}-", plan.id))),
                "Invalid rollback journal path"
            );
        }
    }
    for directory in &plan.empty_directories {
        protected(store, c, directory)?;
        if plan.mode == Mode::Separate {
            ensure!(
                directory.starts_with(&plan.root),
                "Directory escapes restore root"
            );
        }
    }
    Ok(())
}
fn apply(
    store: &Store,
    c: &Config,
    p: &Profile,
    id: &str,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<Plan> {
    let mut plan = load(store, &p.id, id)?;
    ensure!(
        plan.state == "ready",
        "This plan was already applied or interrupted; review its rollback receipt and create a new plan"
    );
    validate_plan(store, c, &plan)?;
    // Validate all target fingerprints before changing any existing file.
    for entry in &plan.entries {
        check(cancel)?;
        ensure!(
            fs_safe::fingerprint(&entry.target)? == entry.before,
            "A target changed after preview: {}. Review a new restore plan.",
            entry.target.display()
        );
    }
    plan.state = "applying".into();
    save(store, &plan)?;
    for index in 0..plan.entries.len() {
        check(cancel)?;
        if ["skip", "unchanged"].contains(&plan.entries[index].action.as_str()) {
            plan.entries[index].status = plan.entries[index].action.clone();
            save(store, &plan)?;
            continue;
        }
        let entry = plan.entries[index].clone();
        let target = Parent::open(&entry.target, true, &mut plan.created_directories)?;
        save(store, &plan)?;
        let incoming = Parent::open(&entry.incoming, false, &mut vec![])?;
        plan.entries[index].status = "installing".into();
        save(store, &plan)?;
        incoming.create_from(&stage_path(store, id, &entry.source)?, &entry.after, || {
            check(cancel)
        })?;
        ensure!(
            target.fingerprint()? == entry.before,
            "Target changed while staging; original file was not replaced"
        );
        if entry.before.is_some() {
            let previous = Parent::open(&entry.previous, false, &mut vec![])?;
            target.rename_to(&previous)?;
            if previous.fingerprint()? != entry.before {
                let _ = previous.rename_to(&target);
                anyhow::bail!(
                    "Target changed during replacement; original data was retained beside the target"
                )
            }
            plan.entries[index].status = "original_saved".into();
            save(store, &plan)?;
        }
        incoming.rename_to(&target)?;
        plan.entries[index].status = "installed".into();
        save(store, &plan)?;
        job.progress = serde_json::json!({"stage":"applying","files_done":index+1,"total_files":plan.entries.len(),"percent_done":(index+1) as f64/plan.entries.len() as f64});
        store.write_job(job)?;
        changed(job);
    }
    for directory in plan.empty_directories.clone() {
        check(cancel)?;
        fs_safe::ensure_directory(&directory, &mut plan.created_directories)?;
        save(store, &plan)?;
    }
    plan.state = "applied".into();
    save(store, &plan)?;
    Ok(plan)
}
fn rollback_check(entry: &Entry) -> Result<String> {
    if ["skip", "unchanged", "rolled_back"].contains(&entry.status.as_str())
        || ["skip", "unchanged"].contains(&entry.action.as_str())
    {
        return Ok("nothing".into());
    }
    let current = fs_safe::fingerprint(&entry.target)?;
    let previous = fs_safe::fingerprint(&entry.previous)?;
    if previous.is_none() && current == entry.before {
        return Ok("nothing".into());
    }
    if current.is_some() && current.as_ref() != Some(&entry.after) {
        anyhow::bail!("Target changed since restore; it will not be overwritten")
    }
    if entry.before.is_some() {
        ensure!(
            previous == entry.before,
            "Saved original changed or is missing; rollback will not replace the current file"
        );
    } else {
        ensure!(
            previous.is_none(),
            "Unexpected saved original; review this receipt"
        );
    }
    Ok("undo".into())
}
fn rollback_preview(store: &Store, p: &Profile, id: &str) -> Result<serde_json::Value> {
    let plan = load(store, &p.id, id)?;
    let mut paths = Vec::new();
    for entry in &plan.entries {
        let status = match rollback_check(entry) {
            Ok(action) => serde_json::json!({"action":action}),
            Err(e) => serde_json::json!({"conflict":e.to_string()}),
        };
        paths.push(serde_json::json!({"target":entry.target,"saved_original":entry.previous,"status":status}));
    }
    Ok(
        serde_json::json!({"version":1,"plan":id,"state":plan.state,"entries":paths,"message":"Changed files are preserved. Replaced originals remain beside their targets until an explicit rollback succeeds."}),
    )
}
fn rollback(
    store: &Store,
    c: &Config,
    p: &Profile,
    id: &str,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<Plan> {
    let mut plan = load(store, &p.id, id)?;
    ensure!(plan.state != "ready", "This plan has not been applied");
    validate_plan(store, c, &plan)?;
    plan.state = "rolling_back".into();
    save(store, &plan)?;
    for index in (0..plan.entries.len()).rev() {
        check(cancel)?;
        let entry = plan.entries[index].clone();
        let outcome = (|| -> Result<()> {
            if rollback_check(&entry)? == "nothing" {
                return Ok(());
            }
            let target = Parent::open(&entry.target, false, &mut vec![])?;
            if target.fingerprint()?.is_some() {
                let undone = Parent::open(&entry.undone, false, &mut vec![])?;
                target.rename_to(&undone)?;
                if undone.fingerprint()?.as_ref() != Some(&entry.after) {
                    let _ = undone.rename_to(&target);
                    anyhow::bail!("Restored file changed during rollback; it was retained")
                }
            }
            if entry.before.is_some() {
                let previous = Parent::open(&entry.previous, false, &mut vec![])?;
                previous.rename_to(&target)?;
            }
            Ok(())
        })();
        match outcome {
            Ok(_) => {
                plan.entries[index].status = "rolled_back".into();
                plan.entries[index].message.clear();
            }
            Err(e) => {
                plan.entries[index].status = "rollback_conflict".into();
                plan.entries[index].message = e.to_string();
            }
        }
        // Remove only an unchanged application-created incoming temporary file.
        if let Ok(incoming) = Parent::open(&entry.incoming, false, &mut vec![]) {
            if incoming.fingerprint().ok().flatten().as_ref() == Some(&entry.after) {
                let _ = incoming.remove_owned(&entry.after);
            }
        }
        save(store, &plan)?;
        job.progress = serde_json::json!({"stage":"rolling_back","files_done":plan.entries.len()-index,"total_files":plan.entries.len()});
        store.write_job(job)?;
        changed(job);
    }
    if plan.entries.iter().any(|e| e.status == "rollback_conflict") {
        plan.state = "rollback_conflict".into();
    } else {
        let mut directories = plan.created_directories.clone();
        directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        directories.dedup();
        for directory in directories {
            let _ = fs_safe::remove_empty_directory(&directory);
        }
        plan.state = "rolled_back".into();
    }
    save(store, &plan)?;
    Ok(plan)
}
fn summary(plan: &Plan) -> serde_json::Value {
    serde_json::json!({"version":1,"plan":plan.id,"snapshot":plan.snapshot,"state":plan.state,"target":plan.root,"files":plan.entries.len(),"installed":plan.entries.iter().filter(|e|e.status=="installed").count(),"skipped":plan.entries.iter().filter(|e|e.action=="skip").count(),"conflicts":plan.entries.iter().filter(|e|e.status=="rollback_conflict").count()})
}
pub fn execute_locked(
    _lock: RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    request: Request,
    cancel: Arc<AtomicBool>,
    job: &mut Job,
    mut changed: impl FnMut(&Job),
) -> Result<()> {
    request.validate()?;
    let operation = request.operation();
    let plan_id = match &request {
        Request::Apply { plan, .. } | Request::Rollback { plan, .. } => Some(plan.clone()),
        _ => None,
    };
    let result = (|| -> Result<Plan> {
        check(&cancel)?;
        match &request {
            Request::Plan { .. } => make_plan(store, c, p, &request, &cancel, job, &mut changed),
            Request::Apply { plan, .. } => apply(store, c, p, plan, &cancel, job, &mut changed),
            Request::Rollback { plan, .. } => {
                rollback(store, c, p, plan, &cancel, job, &mut changed)
            }
            _ => anyhow::bail!("This restore operation is read-only"),
        }
    })();
    match result {
        Ok(plan) => {
            job.result = summary(&plan);
            job.state = if plan.state == "rollback_conflict" {
                JobState::Incomplete
            } else {
                JobState::Success
            };
            job.message=if plan.state=="ready"{"Restore plan ready. Review the files and conflicts before applying."}else if plan.state=="rollback_conflict"{"Rollback retained changed files. Review the receipt's conflicts and saved originals."}else{"Completed; restore and rollback receipts retained."}.into();
        }
        Err(error) => {
            let mut partial = false;
            if let Some(id) = &plan_id {
                if let Ok(mut plan) = load(store, &p.id, id) {
                    if ["applying", "rolling_back", "partial"].contains(&plan.state.as_str()) {
                        plan.state = "partial".into();
                        plan.messages.push(error.to_string());
                        save(store, &plan)?;
                        partial = true;
                    }
                    job.result = summary(&plan);
                }
            }
            job.state = if error.is::<Cancelled>() {
                JobState::Cancelled
            } else if partial {
                JobState::Incomplete
            } else {
                JobState::Failed
            };
            job.message = format!(
                "{operation}: {error}. Review the restore receipt before retrying or rolling back."
            );
        }
    }
    job.finished = Some(now());
    store.write_job(job)?;
    changed(job);
    Ok(())
}
pub fn read(
    store: &Store,
    c: &Config,
    p: &Profile,
    request: &Request,
) -> Result<serde_json::Value> {
    request.validate()?;
    let _lock = RepoLock::acquire(store)?;
    let cancel = AtomicBool::new(false);
    match request {
        Request::Snapshots => {
            let mut cmd = command(store, c, p)?;
            cmd.args([
                "snapshots",
                "--tag",
                &format!("nyx-backup,profile:{}", p.id),
            ]);
            Ok(serde_json::from_slice(&capture(cmd, &cancel)?)?)
        }
        Request::Browse {
            snapshot,
            directory,
        } => {
            owned_snapshot(store, c, p, snapshot, &cancel)?;
            let mut entries = nodes(store, c, p, snapshot, &[directory.clone()], false, &cancel)?;
            entries.retain(|n| n.path != *directory);
            let truncated = entries.len() > 2000;
            entries.truncate(2000);
            Ok(
                serde_json::json!({"version":1,"snapshot":snapshot,"directory":directory,"entries":entries,"truncated":truncated}),
            )
        }
        Request::Inspect { plan } => Ok(serde_json::to_value(load(store, &p.id, plan)?)?),
        Request::Receipts => {
            let directory = store.state.join("restore");
            let mut plans = Vec::new();
            if directory.exists() {
                for entry in fs::read_dir(directory)? {
                    let entry = entry?;
                    if entry.path().join("plan.json").exists() {
                        let id = entry.file_name().to_string_lossy().into_owned();
                        let plan: Plan =
                            serde_json::from_slice(&fs::read(entry.path().join("plan.json"))?)?;
                        if plan.profile == p.id {
                            ensure!(plan.id == id, "Invalid restore receipt");
                            plans.push(serde_json::json!({"id":id,"created":plan.created,"snapshot":plan.snapshot,"state":plan.state,"target":plan.root}));
                        }
                    }
                }
            }
            plans.sort_by_key(|v| std::cmp::Reverse(v["created"].as_u64().unwrap_or(0)));
            Ok(serde_json::json!({"version":1,"receipts":plans}))
        }
        Request::Diff { plan, path } => {
            let plan = load(store, &p.id, plan)?;
            let entry = plan
                .entries
                .iter()
                .find(|e| e.source == *path)
                .context("File is not in this restore plan")?;
            ensure!(
                entry.after.kind == "file" && entry.after.size <= TEXT_LIMIT,
                "Text diffs are limited to ordinary UTF-8 files up to 1 MiB"
            );
            let staged = stage_path(store, &plan.id, &entry.source)?;
            let read_text = |path: &Path| -> Result<String> {
                let parent = Parent::open(path, false, &mut vec![])?;
                let file = parent.read_file()?;
                ensure!(
                    file.metadata()?.len() <= TEXT_LIMIT,
                    "Current file is too large for a text diff"
                );
                let mut bytes = Vec::new();
                file.take(TEXT_LIMIT + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= TEXT_LIMIT && !bytes.contains(&0),
                    "Binary or large file; text diff unavailable"
                );
                Ok(String::from_utf8(bytes).context("File is not UTF-8 text")?)
            };
            let snapshot = read_text(&staged)?;
            let current = if fs_safe::fingerprint(&entry.target)?.is_some() {
                read_text(&entry.target)?
            } else {
                String::new()
            };
            let diff = similar::TextDiff::from_lines(&current, &snapshot)
                .unified_diff()
                .context_radius(3)
                .header("current", "snapshot")
                .to_string();
            ensure!(
                diff.len() <= 4 * 1024 * 1024,
                "Diff is too large to display"
            );
            Ok(
                serde_json::json!({"version":1,"path":path,"target":entry.target,"diff":diff,"message":"Diff content is returned locally and is not stored in job state or logs."}),
            )
        }
        Request::RollbackPreview { plan } => rollback_preview(store, p, plan),
        _ => anyhow::bail!("Use a background restore job for this operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn fixture(action: &str) -> (PathBuf, Store, Config, Profile, Plan) {
        let root = std::env::temp_dir().join(format!("nyx-restore-unit-{}", id()));
        private_dir(&root).unwrap();
        let store = Store {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let p = Profile {
            id: "fixture".into(),
            name: "Fixture".into(),
            destination: "fixture".into(),
            sources: vec![root.join("source")],
            exclusions: vec![],
            include_secrets: false,
            approved: true,
            keep_last: 2,
            schedule: None,
        };
        let c = Config {
            destinations: vec![Destination {
                id: "fixture".into(),
                name: "Fixture".into(),
                kind: DestinationKind::Local {
                    path: root.join("repository"),
                },
            }],
            profiles: vec![p.clone()],
            ..Default::default()
        };
        let id = id();
        let source = "/data/item.txt";
        let staged = stage_path(&store, &id, source).unwrap();
        private_dir(staged.parent().unwrap()).unwrap();
        fs::write(&staged, "snapshot\n").unwrap();
        let target = root.join("output/data/item.txt");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "current\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        let before = fs_safe::fingerprint(&target).unwrap();
        let after = fs_safe::fingerprint(&staged).unwrap().unwrap();
        let parent = target.parent().unwrap();
        let entry = Entry {
            source: source.into(),
            target: target.clone(),
            before,
            after,
            action: action.into(),
            status: "pending".into(),
            previous: parent.join(format!(".nyx-restore-{id}-0.previous")),
            incoming: parent.join(format!(".nyx-restore-{id}-0.incoming")),
            undone: parent.join(format!(".nyx-restore-{id}-0.undone")),
            message: String::new(),
        };
        let plan = Plan {
            version: 1,
            id,
            profile: p.id.clone(),
            snapshot: "a".repeat(64),
            created: now(),
            mode: Mode::Separate,
            policy: Policy::Replace,
            root: root.join("output"),
            state: "ready".into(),
            entries: vec![entry],
            empty_directories: vec![],
            created_directories: vec![],
            messages: vec![],
        };
        save(&store, &plan).unwrap();
        (root, store, c, p, plan)
    }
    #[test]
    fn replace_and_rollback_preserve_original() {
        let (root, store, c, p, plan) = fixture("replace");
        let cancel = AtomicBool::new(false);
        let mut job = Job::new(&p.id, "restore-apply");
        let applied = apply(&store, &c, &p, &plan.id, &cancel, &mut job, &mut |_| {}).unwrap();
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "snapshot\n"
        );
        assert_eq!(
            fs::read_to_string(&plan.entries[0].previous).unwrap(),
            "current\n"
        );
        assert_eq!(applied.state, "applied");
        let undone = rollback(&store, &c, &p, &plan.id, &cancel, &mut job, &mut |_| {}).unwrap();
        assert_eq!(undone.state, "rolled_back");
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "current\n"
        );
        assert_eq!(
            fs::metadata(&plan.entries[0].target)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn post_preview_changes_are_refused_before_writes() {
        let (root, store, c, p, plan) = fixture("replace");
        fs::write(&plan.entries[0].target, "edited after preview\n").unwrap();
        let mut job = Job::new(&p.id, "restore-apply");
        assert!(
            apply(
                &store,
                &c,
                &p,
                &plan.id,
                &AtomicBool::new(false),
                &mut job,
                &mut |_| {}
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "edited after preview\n"
        );
        assert!(!plan.entries[0].previous.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rollback_never_overwrites_new_user_edits() {
        let (root, store, c, p, plan) = fixture("replace");
        let mut job = Job::new(&p.id, "restore-apply");
        apply(
            &store,
            &c,
            &p,
            &plan.id,
            &AtomicBool::new(false),
            &mut job,
            &mut |_| {},
        )
        .unwrap();
        fs::write(&plan.entries[0].target, "new user data\n").unwrap();
        let receipt = rollback(
            &store,
            &c,
            &p,
            &plan.id,
            &AtomicBool::new(false),
            &mut job,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(receipt.state, "rollback_conflict");
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "new user data\n"
        );
        assert_eq!(
            fs::read_to_string(&plan.entries[0].previous).unwrap(),
            "current\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_original_move_can_be_rolled_back() {
        let (root, store, c, p, mut plan) = fixture("replace");
        plan.state = "applying".into();
        save(&store, &plan).unwrap();
        Parent::open(&plan.entries[0].target, false, &mut vec![])
            .unwrap()
            .rename_to(&Parent::open(&plan.entries[0].previous, false, &mut vec![]).unwrap())
            .unwrap();
        let mut job = Job::new(&p.id, "restore-rollback");
        let receipt = rollback(
            &store,
            &c,
            &p,
            &plan.id,
            &AtomicBool::new(false),
            &mut job,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(receipt.state, "rolled_back");
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "current\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn symlink_ancestors_are_never_followed() {
        let (root, store, c, p, plan) = fixture("replace");
        let external = root.join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("item.txt"), "outside\n").unwrap();
        fs::rename(root.join("output/data"), root.join("original-parent")).unwrap();
        symlink(&external, root.join("output/data")).unwrap();
        let mut job = Job::new(&p.id, "restore-apply");
        assert!(
            apply(
                &store,
                &c,
                &p,
                &plan.id,
                &AtomicBool::new(false),
                &mut job,
                &mut |_| {}
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(external.join("item.txt")).unwrap(),
            "outside\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn default_skip_and_cancellation_preserve_current_files() {
        let (root, store, c, p, plan) = fixture("skip");
        let mut job = Job::new(&p.id, "restore-apply");
        let applied = apply(
            &store,
            &c,
            &p,
            &plan.id,
            &AtomicBool::new(false),
            &mut job,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(applied.entries[0].status, "skip");
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "current\n"
        );
        let lock = RepoLock::acquire(&store).unwrap();
        execute_locked(
            lock,
            &store,
            &c,
            &p,
            Request::Plan {
                snapshot: "a".repeat(64),
                paths: vec!["/data".into()],
                target: Some(root.join("cancelled")),
                mode: Mode::Separate,
                policy: Policy::Skip,
                include_secrets: false,
            },
            Arc::new(AtomicBool::new(true)),
            &mut job,
            |_| {},
        )
        .unwrap();
        assert_eq!(job.state, JobState::Cancelled);
        assert!(!root.join("cancelled").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn traversal_and_unconfirmed_requests_are_rejected() {
        assert!(archive_path("/home/user/../escape").is_err());
        assert!(
            Request::Apply {
                plan: "fixture".into(),
                confirmation: "different".into()
            }
            .validate()
            .is_err()
        );
        assert!(snapshot_id("latest").is_err());
    }
}

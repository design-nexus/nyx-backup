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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HomeMapping {
    pub source_home: PathBuf,
    pub source_config: PathBuf,
    pub target_home: PathBuf,
    pub target_config: PathBuf,
    pub monitor_format: String,
    pub rewrite_references: bool,
}
impl HomeMapping {
    pub fn validate(&self) -> Result<()> {
        for path in [
            &self.source_home,
            &self.source_config,
            &self.target_home,
            &self.target_config,
        ] {
            fs_safe::clean_absolute(path)?;
            ensure!(
                path != Path::new("/"),
                "Migration roots cannot be filesystem root"
            );
        }
        ensure!(
            self.target_home == fs_safe::resolved(&nyx_core::paths::home())?
                && self.target_config == fs_safe::resolved(&nyx_core::paths::config_home())?,
            "Migration targets must be the current user's home/configuration"
        );
        ensure!(
            ["lua", "conf", "unknown"].contains(&self.monitor_format.as_str()),
            "Unknown monitor syntax"
        );
        Ok(())
    }
    pub fn target(&self, source: &Path) -> Result<PathBuf> {
        fs_safe::clean_absolute(source)?;
        ensure!(
            !backup_engine::contains_recognized_store(source),
            "Migrate a database/VM export separately; do not replace a live store"
        );
        if let Ok(relative) = source.strip_prefix(&self.source_config) {
            if ["hypr/monitors.lua", "hypr/monitors.conf"]
                .iter()
                .any(|p| relative == Path::new(p))
            {
                ensure!(
                    self.monitor_format != "unknown",
                    "Target Hyprland syntax is unknown; monitor files require manual review"
                );
                return Ok(self
                    .target_config
                    .join(format!("hypr/monitors.{}", self.monitor_format)));
            }
            ensure!(
                !hardware_file(relative),
                "Hardware-sensitive configuration is held for manual review"
            );
            return Ok(self.target_config.join(relative));
        }
        if let Ok(relative) = source.strip_prefix(&self.source_home) {
            ensure!(
                !relative.starts_with(".local/state/nyx-backup") && !relative.starts_with(".cache"),
                "Backup state and caches are not migration inputs"
            );
            return Ok(self.target_home.join(relative));
        }
        anyhow::bail!("Migration only maps selected source home/configuration files")
    }
}
pub fn hardware_file(relative: &Path) -> bool {
    let value = relative.to_string_lossy();
    [
        "hypr/hyprland.lua",
        "hypr/hyprland.conf",
        "hypr/envs.conf",
        "hypr/env.lua",
        "hypr/envs.lua",
        "hypr/input.lua",
        "hypr/input.conf",
        "hypr/render.conf",
        "hypr/variables.conf",
        "hypr/monitor-workspaces.lua",
    ]
    .contains(&value.as_ref())
        || value.starts_with("hypr/nvidia")
        || value.starts_with("environment.d/")
        || value.starts_with("systemd/")
}
fn reference_path(path: &Path) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(b))
}
fn replace_path_prefix(text: &str, from: &Path, to: &Path) -> String {
    let needle = format!("{}/", from.display());
    let replacement = format!("{}/", to.display());
    let mut result = String::new();
    let mut offset = 0;
    for (index, _) in text.match_indices(&needle) {
        let allowed = text[..index]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || "\"'=([{,:".contains(c));
        if allowed {
            result.push_str(&text[offset..index]);
            result.push_str(&replacement);
            offset = index + needle.len();
        }
    }
    result.push_str(&text[offset..]);
    result
}
fn adapt_stage(
    store: &Store,
    id: &str,
    source: &str,
    mapping: &HomeMapping,
) -> Result<Option<String>> {
    let path = stage_path(store, id, source)?;
    let relative = Path::new(source).strip_prefix(&mapping.source_config).ok();
    // Staged links point outside the stage; writing through one would change a
    // real file at planning time.
    let regular = fs::symlink_metadata(&path)?.is_file();
    if relative.is_some_and(|p| {
        p == Path::new("hypr/monitors.lua") || p == Path::new("hypr/monitors.conf")
    }) {
        ensure!(
            regular,
            "The monitor configuration in this snapshot is a link; restore it separately and review it"
        );
        let data = if mapping.monitor_format == "lua" {
            r#"-- Prepared by Nyx Backup: use this machine's preferred display mode.
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })
"#
        } else {
            r#"# Prepared by Nyx Backup: use this machine's preferred display mode.
monitor = , preferred, auto, 1
"#
        };
        atomic(&path, data)?;
        return Ok(Some("Source display assignments replaced with target-format automatic monitor defaults; review before applying.".into()));
    }
    if !mapping.rewrite_references || !regular {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > TEXT_LIMIT {
        return Ok(None);
    }
    let Ok(mut data) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    let recognizable = path.extension().is_some_and(|p| {
        [
            "json", "toml", "ini", "conf", "yaml", "yml", "lua", "desktop", "sh",
        ]
        .iter()
        .any(|s| p == *s)
    }) || Path::new(source).file_name().is_some_and(|n| {
        [".bashrc", ".bash_profile", ".zshrc", ".profile"]
            .iter()
            .any(|s| n == *s)
    });
    if !recognizable
        || ![
            &mapping.source_home,
            &mapping.source_config,
            &mapping.target_home,
            &mapping.target_config,
        ]
        .iter()
        .all(|p| reference_path(p))
    {
        return Ok(None);
    }
    let original = data.clone();
    for (from, to) in [
        (&mapping.source_config, &mapping.target_config),
        (&mapping.source_home, &mapping.target_home),
    ] {
        if from != to {
            data = replace_path_prefix(&data, from, to);
        }
    }
    if data == original {
        return Ok(None);
    }
    if path.extension().is_some_and(|x| x == "json") {
        let _: serde_json::Value = serde_json::from_str(&data)
            .context("Rewritten JSON is invalid; restore it separately and merge manually")?;
    }
    atomic(&path, data)?;
    Ok(Some("Literal source home/configuration path prefixes rewritten in a small recognized UTF-8 configuration file; application compatibility still requires review.".into()))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Separate,
    OriginalDotfiles,
    MigratedUserData(HomeMapping),
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
                if let Mode::MigratedUserData(mapping) = mode {
                    mapping.validate()?;
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
    #[serde(default)]
    pub adaptation: Option<String>,
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
        Mode::MigratedUserData(mapping) => mapping.target(&source)?,
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
        include_secrets,
        ..
    } = request
    else {
        anyhow::bail!("Expected a restore plan")
    };
    owned_snapshot(store, c, p, snapshot, cancel)?;
    let root = match mode {
        Mode::Separate => fs_safe::resolved(target.as_ref().context("Choose a target folder")?)?,
        Mode::OriginalDotfiles => fs_safe::resolved(&nyx_core::paths::home())?,
        Mode::MigratedUserData(mapping) => {
            mapping.validate()?;
            mapping.target_home.clone()
        }
    };
    ensure!(
        mode != &Mode::Separate || root != Path::new("/"),
        "Choose a separate folder, not the filesystem root; full-system restoration is outside this phase"
    );
    protected(store, c, &root)?;
    let selected = selected_nodes(store, c, p, snapshot, paths, cancel)?;
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
    // Staging decrypts the selection; a plan that fails after that point has no
    // receipt to find it by, so its copies are removed with it.
    let staged = stage_plan(
        store, c, p, request, &id, &directory, selected, cancel, job, changed,
    );
    if staged.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    staged
}
#[allow(clippy::too_many_arguments)]
fn stage_plan(
    store: &Store,
    c: &Config,
    p: &Profile,
    request: &Request,
    id: &str,
    directory: &Path,
    selected: Vec<Node>,
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
    let root = match mode {
        Mode::Separate => fs_safe::resolved(target.as_ref().context("Choose a target folder")?)?,
        Mode::OriginalDotfiles => fs_safe::resolved(&nyx_core::paths::home())?,
        Mode::MigratedUserData(mapping) => mapping.target_home.clone(),
    };
    let id = id.to_string();
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
    let mut selected_targets = std::collections::HashSet::new();
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
        let adaptation = if let Mode::MigratedUserData(mapping) = mode {
            adapt_stage(store, &plan.id, &node.path, mapping)?
        } else {
            None
        };
        let after = fs_safe::fingerprint(&source)?.context("A selected staged file is missing")?;
        if !matches!(mode, Mode::Separate) && after.kind == "file" {
            let mut magic = [0u8; 16];
            let mut file = Parent::open(&source, false, &mut vec![])?.read_file()?;
            ensure!(
                file.read_exact(&mut magic).is_err() || &magic != b"SQLite format 3\0",
                "Quick Restore excludes SQLite stores; restore a separate copy"
            );
        }
        let mut target = mapped_target(store, c, mode, &plan.root, &node.path, *include_secrets)?;
        // A folder or special file where the file would go is never replaced.
        let occupied = fs::symlink_metadata(&target)
            .is_ok_and(|m| !m.is_file() && !m.file_type().is_symlink());
        let mut before = if occupied {
            None
        } else {
            fs_safe::fingerprint(&target)?
        };
        let action = if before.as_ref() == Some(&after) {
            "unchanged"
        } else if before.is_some() || occupied {
            match policy {
                Policy::Skip => "skip",
                Policy::Replace => {
                    ensure!(
                        !occupied,
                        "A folder or special file is in the way of {}; choose Skip or Keep both",
                        target.display()
                    );
                    "replace"
                }
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
        ensure!(
            selected_targets.insert(target.clone()),
            "Selected files map to the same destination; select one version"
        );
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
            adaptation,
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
fn policy_target(mapped: &Path, target: &Path, plan: &Plan) -> bool {
    if mapped == target {
        return true;
    }
    if plan.policy != Policy::KeepBoth {
        return false;
    }
    let Some(name) = mapped.file_name() else {
        return false;
    };
    let mut name = name.to_os_string();
    name.push(format!(
        ".restored-{}-{}",
        &plan.snapshot[..8],
        &plan.id[..8]
    ));
    mapped.with_file_name(name) == target
}
fn validate_plan(store: &Store, c: &Config, plan: &Plan) -> Result<()> {
    fs_safe::clean_absolute(&plan.root)?;
    if let Mode::MigratedUserData(mapping) = &plan.mode {
        mapping.validate()?;
    }
    for entry in &plan.entries {
        protected(store, c, &entry.target)?;
        match &plan.mode {
            Mode::Separate => ensure!(
                entry.target.starts_with(&plan.root) && entry.target != plan.root,
                "Restore entry escapes its selected root"
            ),
            Mode::OriginalDotfiles => {
                let mapped = quick_target(&archive_path(&entry.source)?, true)?;
                ensure!(
                    policy_target(&mapped, &entry.target, plan),
                    "Original dotfile target changed"
                );
            }
            Mode::MigratedUserData(mapping) => {
                let mapped = mapping.target(&archive_path(&entry.source)?)?;
                ensure!(
                    policy_target(&mapped, &entry.target, plan),
                    "Migration target changed"
                );
            }
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
        if entry.action == "skip" {
            continue;
        }
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
    let previous = fs_safe::fingerprint(&entry.previous)?;
    // Not yet installed and no original moved aside: the target is untouched.
    if ["pending", "installing"].contains(&entry.status.as_str()) && previous.is_none() {
        return Ok("nothing".into());
    }
    let current = fs_safe::fingerprint(&entry.target)?;
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
                for entry in fs::read_dir(directory)?.flatten() {
                    if entry.path().join("plan.json").exists() {
                        let id = entry.file_name().to_string_lossy().into_owned();
                        // One damaged receipt must not hide the others.
                        let Ok(plan) = fs::read(entry.path().join("plan.json"))
                            .map_err(anyhow::Error::from)
                            .and_then(|b| Ok(serde_json::from_slice::<Plan>(&b)?))
                        else {
                            continue;
                        };
                        if plan.profile == p.id && plan.id == id {
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
                .header(
                    "current",
                    if entry.adaptation.is_some() {
                        "prepared"
                    } else {
                        "snapshot"
                    },
                )
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
            adaptation: None,
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
    fn rollback_ignores_entries_that_were_never_installed() {
        let (root, store, c, p, mut plan) = fixture("replace");
        plan.state = "partial".into();
        save(&store, &plan).unwrap();
        // The user edits the target after an interrupted apply that never reached it.
        fs::write(&plan.entries[0].target, "edited later\n").unwrap();
        let cancel = AtomicBool::new(false);
        let mut job = Job::new(&p.id, "restore-rollback");
        let undone = rollback(&store, &c, &p, &plan.id, &cancel, &mut job, &mut |_| {}).unwrap();
        assert_eq!(undone.state, "rolled_back");
        assert_eq!(
            fs::read_to_string(&plan.entries[0].target).unwrap(),
            "edited later\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn skipped_entries_tolerate_a_folder_in_the_way() {
        let (root, store, c, p, mut plan) = fixture("skip");
        let target = plan.entries[0].target.clone();
        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        plan.entries[0].before = None;
        save(&store, &plan).unwrap();
        let cancel = AtomicBool::new(false);
        let mut job = Job::new(&p.id, "restore-apply");
        let applied = apply(&store, &c, &p, &plan.id, &cancel, &mut job, &mut |_| {}).unwrap();
        assert_eq!(applied.state, "applied");
        assert!(target.is_dir());
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

/// Application-owned migration uses these fixed operations while retaining RepoLock.
pub fn snapshot_metadata(
    _lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    cancel: &AtomicBool,
) -> Result<serde_json::Value> {
    owned_snapshot(store, c, p, snapshot, cancel)?;
    let mut cmd = command(store, c, p)?;
    cmd.args(["cat", "snapshot", snapshot]);
    let metadata: serde_json::Value = serde_json::from_slice(&capture(cmd, cancel)?)?;
    let suffix = format!("/nyx-backup/metadata/{}/system.json", p.id);
    let path = metadata["paths"]
        .as_array()
        .and_then(|paths| {
            paths
                .iter()
                .filter_map(|p| p.as_str())
                .find(|p| p.ends_with(&suffix))
        })
        .context("Snapshot has no Backup system metadata")?;
    let mut cmd = command(store, c, p)?;
    cmd.args(["dump", snapshot, path]);
    let bytes = capture(cmd, cancel)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "Snapshot metadata is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn snapshot_selection(
    _lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    paths: &[String],
    cancel: &AtomicBool,
) -> Result<Vec<Node>> {
    owned_snapshot(store, c, p, snapshot, cancel)?;
    selected_nodes(store, c, p, snapshot, paths, cancel)
}
/// The snapshot entries under the selected paths; every selected path must exist.
fn selected_nodes(
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    paths: &[String],
    cancel: &AtomicBool,
) -> Result<Vec<Node>> {
    let selection = nodes(store, c, p, snapshot, paths, true, cancel)?;
    let mut selected: Vec<_> = selection
        .into_iter()
        .filter(|n| related(Path::new(&n.path), paths))
        .collect();
    // Restic ls directory filters do not return a selected regular-file node on every backend.
    let missing = |selected: &[Node]| {
        paths.iter().any(|source| {
            !selected
                .iter()
                .any(|n| n.path == *source || Path::new(&n.path).starts_with(source))
        })
    };
    if missing(&selected) {
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
    Ok(selected)
}
pub fn prepare_migration(
    _lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    request: &Request,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<Plan> {
    request.validate()?;
    ensure!(
        matches!(
            request,
            Request::Plan {
                mode: Mode::MigratedUserData(_),
                ..
            }
        ),
        "Expected mapped migration preparation"
    );
    make_plan(store, c, p, request, cancel, job, changed)
}
pub fn append_plan_messages(
    store: &Store,
    profile: &str,
    id: &str,
    messages: Vec<String>,
) -> Result<()> {
    let mut plan = load(store, profile, id)?;
    plan.messages.extend(messages);
    save(store, &plan)
}

pub fn validate_snapshot(
    _lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    owned_snapshot(store, c, p, snapshot, cancel)
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    fn mapping() -> HomeMapping {
        HomeMapping {
            source_home: "/old/home".into(),
            source_config: "/old/home/.config".into(),
            target_home: "/new/home".into(),
            target_config: "/new/home/.config".into(),
            monitor_format: "lua".into(),
            rewrite_references: true,
        }
    }
    #[test]
    fn mapped_targets_are_bounded_and_hardware_files_held() {
        let m = mapping();
        assert_eq!(
            m.target(Path::new("/old/home/Documents/file.txt")).unwrap(),
            PathBuf::from("/new/home/Documents/file.txt")
        );
        assert_eq!(
            m.target(Path::new("/old/home/.config/hypr/monitors.conf"))
                .unwrap(),
            PathBuf::from("/new/home/.config/hypr/monitors.lua")
        );
        assert!(m.target(Path::new("/etc/fstab")).is_err());
        assert!(
            m.target(Path::new("/old/home/.config/hypr/envs.conf"))
                .is_err()
        );
        assert!(m.target(Path::new("/old/home/data.sqlite")).is_err());
    }
    #[test]
    fn literal_prefix_rewrite_avoids_urls_and_other_paths() {
        let text = "local='/old/home/Documents' url='https://host/old/home/Documents' other='/backup/old/home/Documents'";
        assert_eq!(
            replace_path_prefix(text, Path::new("/old/home"), Path::new("/new/home")),
            "local='/new/home/Documents' url='https://host/old/home/Documents' other='/backup/old/home/Documents'"
        );
    }
    #[test]
    fn monitor_adaptation_never_executes_or_copies_source_code() {
        let root = std::env::temp_dir().join(format!("nyx-migration-unit-{}", id()));
        let store = Store {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let source = "/old/home/.config/hypr/monitors.conf";
        let path = stage_path(&store, "fixture", source).unwrap();
        private_dir(path.parent().unwrap()).unwrap();
        fs::write(&path, "SOURCE-DP-9; os.execute('untrusted command')").unwrap();
        let result = adapt_stage(&store, "fixture", source, &mapping()).unwrap();
        assert!(result.is_some());
        let output = fs::read_to_string(&path).unwrap();
        assert!(output.contains("hl.monitor({ output = \"\""));
        assert!(output.contains('\n'));
        assert!(!output.contains("SOURCE-DP") && !output.contains("os.execute"));
        fs::remove_dir_all(root).unwrap();
    }
}

pub fn migration_target_allowed(store: &Store, c: &Config, path: &Path) -> Result<()> {
    protected(store, c, path)
}

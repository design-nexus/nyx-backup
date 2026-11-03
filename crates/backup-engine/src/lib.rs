//! Restic execution. Selection is shared by preview and backup; repository locks remain enabled.
use anyhow::{Context, Result, ensure};
use backup_core::*;
use backup_system::*;
use fs2::FileExt;
use globset::{Glob, GlobSetBuilder};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
pub struct Selection {
    pub preview: Preview,
    pub entries: Vec<PathBuf>,
}
const SECRET_NAMES: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".password-store",
    "keyrings",
    "keepassxc",
    "1Password",
    "Bitwarden",
    "nyx-passwords",
    ".env",
    "credentials.json",
    "credentials",
    "id_rsa",
    "id_ed25519",
    ".netrc",
    ".git-credentials",
    "rclone",
    "chromium",
    "google-chrome",
    "BraveSoftware",
    ".mozilla",
];
const STORE_NAMES: &[&str] = &[
    ".docker",
    "containers",
    "libvirt",
    ".local/share/containers",
    "postgresql",
    "mysql",
    ".var/app/com.usebottles.bottles",
];
fn recognized_secret(path: &Path) -> bool {
    path.components()
        .any(|p| SECRET_NAMES.iter().any(|s| p.as_os_str() == *s))
        || path
            .extension()
            .is_some_and(|s| s == "pem" || s == "key" || s == "kdbx")
}
fn recognized_store(path: &Path) -> bool {
    path.components()
        .any(|p| STORE_NAMES.iter().any(|s| p.as_os_str() == *s))
        || path.extension().is_some_and(|s| {
            ["db", "sqlite", "sqlite3", "qcow2", "vmdk", "vdi"]
                .iter()
                .any(|x| s == *x)
        })
}
fn active_marker(path: &Path) -> bool {
    let s = path.file_name().unwrap_or_default().to_string_lossy();
    s.ends_with("-wal") || s == "postmaster.pid" || s == "mongod.lock"
}
#[derive(Debug)]
struct PreparationCancelled;
impl std::fmt::Display for PreparationCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cancelled during preparation")
    }
}
impl std::error::Error for PreparationCancelled {}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(PreparationCancelled.into());
    }
    Ok(())
}
pub fn select(store: &Store, c: &Config, p: &Profile) -> Result<Selection> {
    select_with_check(store, c, p, || Ok(()))
}
fn select_with_check(
    store: &Store,
    c: &Config,
    p: &Profile,
    mut check: impl FnMut() -> Result<()>,
) -> Result<Selection> {
    check()?;
    let d = c.destination(&p.destination)?;
    validate_destination(d)?;
    let mut builder = GlobSetBuilder::new();
    for pattern in &p.exclusions {
        builder.add(Glob::new(pattern).with_context(|| format!("Invalid exclusion: {pattern}"))?);
    }
    let globs = builder.build()?;
    let mut preview=Preview { version:1,profile:p.id.clone(),sources:p.sources.clone(),exclusions:p.exclusions.clone(),logical_bytes:0,files:0,unreadable:0,warnings:vec!["Logical file size estimate, not compressed transfer size. Files may change after preview.".into(),"Database/VM detection is best effort. Stop applications or export consistent data. Any arbitrary file may contain secrets.".into()],blocked:vec![] };
    let mut forbidden = vec![
        store.config.clone(),
        store.state.clone(),
        store.cache.clone(),
    ];
    for dest in &c.destinations {
        if let Some(path) = dest.local_path() {
            forbidden.push(path.to_path_buf());
        }
    }
    let canonical_forbidden: Vec<_> = forbidden
        .iter()
        .map(|s| fs::canonicalize(s).unwrap_or(s.clone()))
        .collect();
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut scan_roots = Vec::new();
    for source in &p.sources {
        check()?;
        let absolute = match fs::canonicalize(source) {
            Ok(path) => path,
            Err(_) if fs::symlink_metadata(source).is_ok_and(|m| m.file_type().is_symlink()) => {
                fs::canonicalize(source.parent().context("Source has no parent")?)?
                    .join(source.file_name().context("Source has no name")?)
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Source is unavailable: {}", source.display()));
            }
        };
        ensure!(
            !canonical_forbidden.iter().any(|f| absolute.starts_with(f)),
            "Source is inside Backup's working files or a repository destination"
        );
        if let Some(repo) = d.local_path() {
            let repo = fs::canonicalize(repo).unwrap_or(repo.to_path_buf());
            ensure!(
                !absolute.starts_with(&repo),
                "Source and repository overlap"
            );
        }
        // Resolve traversal and ancestor aliases before matching exclusions.
        // Preserve a final symlink as a link, including a dangling one.
        let scan_source = if fs::symlink_metadata(source)?.file_type().is_symlink() {
            fs::canonicalize(source.parent().context("Source has no parent")?)?
                .join(source.file_name().context("Source has no name")?)
        } else {
            absolute
        };
        let requested_source = source;
        let source = &scan_source;
        let excluded = |path: &Path| -> bool {
            if forbidden
                .iter()
                .chain(&canonical_forbidden)
                .any(|f| path.starts_with(f))
            {
                return true;
            }
            if path.components().any(|p| {
                p.as_os_str() == ".cache" || p.as_os_str() == ".Trash" || p.as_os_str() == "Trash"
            }) {
                return true;
            }
            if !p.include_secrets && recognized_secret(path) {
                return true;
            }
            if recognized_store(path) {
                return true;
            }
            globs.is_match(path)
                || path
                    .strip_prefix(source)
                    .is_ok_and(|relative| globs.is_match(requested_source.join(relative)))
                || path
                    .file_name()
                    .is_some_and(|s| globs.is_match(Path::new(s)))
                || path.strip_prefix(source).is_ok_and(|s| globs.is_match(s))
        };
        ensure!(
            !excluded(source),
            "Selected source is excluded by safety rules; select a readable export or explicitly authorize recognized credentials"
        );
        if !fs::symlink_metadata(source)?.file_type().is_symlink() {
            scan_roots.push(source.clone());
        }
        let walk = walkdir::WalkDir::new(source)
            .follow_links(false)
            .follow_root_links(false)
            .into_iter()
            .filter_entry(|e| !excluded(e.path()));
        for e in walk {
            check()?;
            let e = match e {
                Ok(e) => e,
                Err(_) => {
                    preview.unreadable += 1;
                    continue;
                }
            };
            let path = e.path();
            if active_marker(path) {
                preview.blocked.push(format!(
                    "Active store marker: {}. Stop the application or back up an export.",
                    path.display()
                ));
                continue;
            }
            if e.file_type().is_dir() {
                if path.join("PG_VERSION").exists() || path.join("ibdata1").exists() {
                    preview.blocked.push(format!(
                        "Database store detected at {}; use an export",
                        path.display()
                    ));
                    continue;
                }
                if fs::read_dir(path).is_ok_and(|mut r| r.next().is_none())
                    && seen.insert(path.to_path_buf())
                {
                    entries.push(path.to_path_buf());
                }
                continue;
            }
            // SQLite files without conventional extensions are still outside defaults.
            if e.file_type().is_file() {
                let mut magic = [0u8; 16];
                if fs::File::open(path)
                    .and_then(|mut f| f.read_exact(&mut magic))
                    .is_ok()
                    && &magic == b"SQLite format 3\0"
                {
                    preview
                        .warnings
                        .push(format!("SQLite store excluded: {}", path.display()));
                    continue;
                }
            }
            if seen.insert(path.to_path_buf()) {
                if let Ok(m) = e.metadata() {
                    preview.logical_bytes += m.len();
                } else {
                    preview.unreadable += 1;
                }
                preview.files += 1;
                entries.push(path.to_path_buf());
            }
        }
    }
    // Open database and VM images are blocked even when detected by a running process.
    if let Ok(procs) = fs::read_dir("/proc") {
        for proc in procs.flatten().filter(|p| {
            p.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        }) {
            check()?;
            if let Ok(fds) = fs::read_dir(proc.path().join("fd")) {
                for fd in fds.flatten() {
                    check()?;
                    if let Ok(path) = fs::read_link(fd.path()) {
                        if scan_roots.iter().any(|s| path.starts_with(s)) && recognized_store(&path)
                        {
                            preview.blocked.push(format!(
                                "Open database/VM store: {}. Stop it or select an export.",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
    }
    preview.blocked.sort();
    preview.blocked.dedup();
    if !p.include_secrets {
        preview.warnings.push("Recognized credentials, private keys and secret stores excluded. Unknown configuration is included only when selected.".into());
    }
    if preview.unreadable > 0 {
        preview.warnings.push(format!(
            "{} unreadable paths; execution will be marked incomplete",
            preview.unreadable
        ));
    }
    Ok(Selection { preview, entries })
}
fn repo_args(store: &Store, d: &Destination) -> Result<Vec<String>> {
    let repo = match &d.kind {
        DestinationKind::Local { path }
        | DestinationKind::Mounted { path, .. }
        | DestinationKind::Nfs { path, .. } => path.to_string_lossy().into_owned(),
        DestinationKind::Sftp { host, path } => format!("sftp:{host}:{path}"),
        DestinationKind::Smb { .. } | DestinationKind::S3 { .. } => {
            format!("rclone:{}", smb_path(d)?)
        }
    };
    let mut a = vec![
        "--repo".into(),
        repo,
        "--password-command".into(),
        helper(&d.id, "repository"),
        "--json".into(),
        "--cache-dir".into(),
        store.cache.join("restic").to_string_lossy().into_owned(),
    ];
    if matches!(
        d.kind,
        DestinationKind::Smb { .. } | DestinationKind::S3 { .. }
    ) {
        validate_rclone_config(store, d)?;
        ensure_secret_unlocked(&d.id, "rclone")?;
        let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let command = format!(
            "rclone --config {} --password-command {} --ask-password=false --contimeout 15s --timeout 30s",
            q(&rclone_config(store, &d.id).to_string_lossy()),
            q(&helper(&d.id, "rclone"))
        );
        a.extend(["-o".into(), format!("rclone.program={command}")]);
    }
    if let DestinationKind::Sftp { .. } = &d.kind {
        a.extend([
            "-o".into(),
            "sftp.args=-oBatchMode=yes -oConnectTimeout=15".into(),
        ]);
    }
    Ok(a)
}
/// Configured repository access for application-owned domain operations.
/// Callers must hold RepoLock and must supply fixed Restic operations.
pub fn restic_command(store: &Store, c: &Config, p: &Profile) -> Result<std::process::Command> {
    let destination = c.destination(&p.destination)?;
    validate_destination(destination)?;
    let _password = password(&destination.id)?;
    let mut command = clean_command("restic");
    command.args(repo_args(store, destination)?);
    Ok(command)
}
pub fn contains_recognized_secrets(path: &Path) -> bool {
    recognized_secret(path)
}
pub fn contains_recognized_store(path: &Path) -> bool {
    recognized_store(path)
}
pub struct RepoLock(fs::File);
impl RepoLock {
    pub fn acquire(store: &Store) -> Result<Self> {
        private_dir(&store.state)?;
        let f = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(store.state.join("worker.lock"))?;
        f.try_lock_exclusive()
            .context("Another repository operation is running; wait for it to finish")?;
        Ok(Self(f))
    }
}
impl Drop for RepoLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub fn reconcile(store: &Store) -> Result<()> {
    let _guard = match RepoLock::acquire(store) {
        Ok(guard) => guard,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::WouldBlock) =>
        {
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    for mut j in store.jobs()? {
        if j.state == JobState::Running {
            j.state = JobState::Interrupted;
            j.finished = Some(now());
            j.message="Worker stopped. Start a new incremental backup. Repository locks have been preserved.".into();
            store.write_job(&j)?;
        }
    }
    Ok(())
}
fn terminate(child: &mut std::process::Child) {
    // Each Restic operation owns a process group, including SSH/Rclone descendants.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGTERM);
    }
    for _ in 0..30 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
}
pub fn terminal_state(code: Option<i32>, cancel: bool, unreadable: u64) -> JobState {
    if cancel {
        JobState::Cancelled
    } else if code == Some(3) || (code == Some(0) && unreadable > 0) {
        JobState::Incomplete
    } else if code == Some(0) {
        JobState::Success
    } else {
        JobState::Failed
    }
}
/// Run an operation using the admission lock already held by its dispatcher.
/// Keeping ownership across thread handoff prevents another caller taking it.
pub fn execute_locked(
    lock: RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    operation: &str,
    cancel: Arc<AtomicBool>,
    job: &mut Job,
    mut changed: impl FnMut(&Job),
) -> Result<()> {
    let _lock = lock;
    let result = execute_inner(store, c, p, operation, &cancel, job, &mut changed);
    match result {
        Err(e) if e.is::<PreparationCancelled>() => {
            job.state = JobState::Cancelled;
            job.finished = Some(now());
            job.message =
                "Cancelled before Restic started. No repository operation was performed.".into();
            store.write_job(job)?;
            changed(job);
            Ok(())
        }
        other => other,
    }
}
fn execute_inner(
    store: &Store,
    c: &Config,
    p: &Profile,
    operation: &str,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<()> {
    check_cancel(cancel)?;
    let d = c.destination(&p.destination)?;
    validate_destination(d)?;
    // Check availability before starting a child; never replace missing credentials.
    let _password = password(&d.id)?;
    let mut args = repo_args(store, d)?;
    let mut entries = Vec::new();
    let mut unreadable = 0;
    let tag = format!("nyx-backup,profile:{}", p.id);
    match operation {
        "init" => {
            args.push("init".into());
        }
        "validate" => {
            args.extend(["cat".into(), "config".into()]);
        }
        "backup" => {
            let s = select_with_check(store, c, p, || check_cancel(cancel))?;
            ensure!(
                s.preview.blocked.is_empty(),
                "{}",
                s.preview.blocked.join("\n")
            );
            unreadable = s.preview.unreadable;
            entries = s.entries;
            ensure!(
                !entries.is_empty(),
                "No selected files remain after exclusions"
            );
            let metadata_dir = store.state.join("metadata").join(&p.id);
            private_dir(&metadata_dir)?;
            let metadata_file = metadata_dir.join("system.json");
            check_cancel(cancel)?;
            let system_metadata = metadata();
            check_cancel(cancel)?;
            atomic(&metadata_file, serde_json::to_vec_pretty(&system_metadata)?)?;
            entries.push(metadata_file);
            args.extend([
                "backup".into(),
                "--tag".into(),
                tag.clone(),
                "--files-from-raw".into(),
                "-".into(),
            ]);
        }
        "history" => args.extend(["snapshots".into(), "--tag".into(), tag]),
        "stats" => args.extend(["stats".into(), "--mode".into(), "raw-data".into()]),
        "verify" => args.push("check".into()),
        "verify-full" => args.extend(["check".into(), "--read-data".into()]),
        "retention-preview" | "retention-apply" => {
            args.extend([
                "forget".into(),
                "--tag".into(),
                tag,
                "--group-by".into(),
                "tags".into(),
                "--keep-last".into(),
                p.keep_last.to_string(),
            ]);
            if operation == "retention-preview" {
                args.push("--dry-run".into());
            }
        }
        _ => anyhow::bail!("Unsupported operation"),
    }
    check_cancel(cancel)?;
    job.progress = serde_json::json!({"stage":"running"});
    store.write_job(job)?;
    changed(job);
    let mut command = clean_command("restic");
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if operation == "backup" {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().context("Install Restic to run backups")?;
    let input = child.stdin.take();
    let writer = thread::spawn(move || {
        if let Some(mut input) = input {
            for path in entries {
                if input
                    .write_all(path.as_os_str().as_bytes())
                    .and_then(|_| input.write_all(&[0]))
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let (tx, rx) = std::sync::mpsc::channel();
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let txout = tx.clone();
    thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if txout.send((true, line)).is_err() {
                break;
            }
        }
    });
    thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if tx.send((false, line)).is_err() {
                break;
            }
        }
    });
    let mut output = String::new();
    let mut errors = 0u64;
    let mut last_write = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            terminate(&mut child);
        }
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok((stdout, line)) => {
                if stdout {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if v["message_type"] == "status" {
                            job.progress = serde_json::json!({"percent_done":v["percent_done"],"bytes_done":v["bytes_done"],"total_bytes":v["total_bytes"],"files_done":v["files_done"],"total_files":v["total_files"]});
                        } else if v["message_type"] == "summary" {
                            job.result = v;
                        } else if operation != "backup" && output.len() < 32 * 1024 * 1024 {
                            output.push_str(&line);
                            output.push('\n');
                        }
                    } else if operation != "backup" && output.len() < 32 * 1024 * 1024 {
                        output.push_str(&line);
                        output.push('\n');
                    }
                } else {
                    errors += 1; /* Raw child errors may contain sensitive paths/remote diagnostics. Never persist them. */
                }
                if last_write.elapsed().as_millis() > 250 {
                    if let Err(e) = store.write_job(job) {
                        terminate(&mut child);
                        return Err(e);
                    }
                    changed(job);
                    last_write = std::time::Instant::now();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    let _ = writer.join();
    job.state = terminal_state(status.code(), cancel.load(Ordering::SeqCst), unreadable);
    job.finished = Some(now());
    if operation != "backup" {
        job.result = serde_json::from_str(&output).unwrap_or_else(
            |_| serde_json::json!({"message":"Operation completed; no structured result returned"}),
        );
    }
    job.message=match job.state {
  JobState::Success=>"Completed".into(), JobState::Cancelled=>"Cancelled. A new run will reuse completed repository data; locks are not automatically removed.".into(),
  JobState::Incomplete=>"Incomplete snapshot: some selected files could not be read. Review source permissions and run again.".into(),
  _=>format!("Restic failed (exit {:?}; {errors} diagnostic lines suppressed). Check repository password, connectivity, access permissions and repository locks. Initialization is always explicit.",status.code()),
 };
    store.write_job(job)?;
    changed(job);
    Ok(())
}
pub fn run(store: &Store, c: &Config, p: &Profile, op: &str) -> Result<Job> {
    let lock = RepoLock::acquire(store)?;
    let mut j = Job::new(&p.id, op);
    store.write_job(&j)?;
    if let Err(e) = execute_locked(
        lock,
        store,
        c,
        p,
        op,
        Arc::new(AtomicBool::new(false)),
        &mut j,
        |_| {},
    ) {
        j.state = JobState::Failed;
        j.finished = Some(now());
        j.message = e.to_string();
        store.write_job(&j)?;
    }
    Ok(j)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exit_states() {
        assert_eq!(terminal_state(Some(3), false, 0), JobState::Incomplete);
        assert_eq!(terminal_state(Some(0), false, 1), JobState::Incomplete);
        assert_eq!(terminal_state(Some(0), true, 0), JobState::Cancelled);
        assert_eq!(terminal_state(Some(1), false, 0), JobState::Failed);
    }
    #[test]
    fn excludes_safety_but_keeps_builds() {
        assert!(recognized_secret(Path::new("/home/test/.ssh/id_ed25519")));
        assert!(!recognized_store(Path::new(
            "/home/test/project/target/useful.txt"
        )));
        assert!(recognized_store(Path::new("/home/test/vm.qcow2")));
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    fn fixture() -> (PathBuf, Store, Config, Profile) {
        let root = std::env::temp_dir().join(format!("nyx-backup-unit-{}", id()));
        fs::create_dir_all(root.join("source/.cache")).unwrap();
        fs::create_dir_all(root.join("source/.ssh")).unwrap();
        fs::create_dir_all(root.join("source/project/target")).unwrap();
        for path in [
            "source/keep.txt",
            "source/.cache/cache",
            "source/.ssh/id_ed25519",
            "source/project/target/important",
        ] {
            fs::write(root.join(path), b"fixture").unwrap();
        }
        let store = Store {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let d = Destination {
            id: "test".into(),
            name: "Test".into(),
            kind: DestinationKind::Local {
                path: root.join("repo"),
            },
        };
        let p = Profile {
            id: "test".into(),
            name: "Test".into(),
            destination: "test".into(),
            sources: vec![root.join("source")],
            exclusions: vec![],
            include_secrets: false,
            approved: true,
            keep_last: 2,
            schedule: None,
        };
        let c = Config {
            destinations: vec![d],
            profiles: vec![p.clone()],
            ..Default::default()
        };
        (root, store, c, p)
    }
    #[test]
    fn selection_and_overlap() {
        let (root, store, c, mut p) = fixture();
        let s = select(&store, &c, &p).unwrap();
        assert_eq!(s.preview.files, 2);
        assert!(
            s.entries
                .contains(&root.join("source/project/target/important"))
        );
        p.include_secrets = true;
        assert_eq!(select(&store, &c, &p).unwrap().preview.files, 3);
        fs::create_dir_all(root.join("repo")).unwrap();
        p.sources = vec![root.join("repo")];
        assert!(select(&store, &c, &p).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn normalized_sources_keep_repository_exclusions() {
        let (root, store, mut c, mut p) = fixture();
        let repository = root.join("source/repository");
        fs::create_dir_all(&repository).unwrap();
        fs::write(repository.join("private-pack"), b"not a source").unwrap();
        c.destinations[0].kind = DestinationKind::Local {
            path: repository.clone(),
        };
        p.sources = vec![root.join("source/project/..")];
        let selection = select(&store, &c, &p).unwrap();
        assert_eq!(selection.preview.files, 2);
        assert!(
            selection
                .entries
                .iter()
                .all(|p| !p.starts_with(&repository))
        );
        p.exclusions = vec![
            root.join("source/project/../keep.txt")
                .display()
                .to_string(),
        ];
        assert_eq!(select(&store, &c, &p).unwrap().preview.files, 1);
        let _active_database = fs::File::create(root.join("source/active.db")).unwrap();
        assert!(
            select(&store, &c, &p)
                .unwrap()
                .preview
                .blocked
                .iter()
                .any(|message| message.contains("active.db"))
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn selected_root_symlinks_are_never_followed() {
        use std::os::unix::fs::symlink;
        let (root, store, c, mut p) = fixture();
        fs::create_dir(root.join("outside")).unwrap();
        fs::write(root.join("outside/sensitive"), b"fixture").unwrap();
        let link = root.join("selected-link");
        symlink(root.join("outside"), &link).unwrap();
        p.sources = vec![link.clone()];
        let selection = select(&store, &c, &p).unwrap();
        assert_eq!(selection.entries, vec![link]);
        assert_eq!(selection.preview.files, 1);
        let dangling = root.join("dangling");
        symlink(root.join("absent"), &dangling).unwrap();
        p.sources = vec![dangling.clone()];
        assert_eq!(select(&store, &c, &p).unwrap().entries, vec![dangling]);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn destination_traversal_and_nested_mount_are_refused() {
        let (root, _, mut c, _) = fixture();
        c.destinations[0].kind = DestinationKind::Local {
            path: root.join("absent/../../escaped"),
        };
        assert!(c.validate().is_err());
        let mount = mount_identity(Path::new("/")).unwrap();
        let d = Destination {
            id: "nested".into(),
            name: "Nested mount".into(),
            kind: DestinationKind::Mounted {
                path: PathBuf::from("/proc/nyx-backup-qualification"),
                mount_point: PathBuf::from("/"),
                source: mount["source"].as_str().unwrap().into(),
                filesystem: mount["fstype"].as_str().unwrap().into(),
                uuid: mount["uuid"].as_str().unwrap_or("").into(),
            },
        };
        assert!(
            validate_destination(&d)
                .unwrap_err()
                .to_string()
                .contains("different mount")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelling_preparation_never_needs_credentials_or_starts_restic() {
        let (root, store, c, p) = fixture();
        let mut checks = 0;
        let error = select_with_check(&store, &c, &p, || {
            checks += 1;
            if checks == 4 {
                Err(PreparationCancelled.into())
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert!(error.is::<PreparationCancelled>());
        assert_eq!(checks, 4);
        let lock = RepoLock::acquire(&store).unwrap();
        let mut job = Job::new(&p.id, "backup");
        let mut changes = 0;
        execute_locked(
            lock,
            &store,
            &c,
            &p,
            "backup",
            Arc::new(AtomicBool::new(true)),
            &mut job,
            |j| {
                changes += 1;
                assert_eq!(j.state, JobState::Cancelled);
                assert!(
                    RepoLock::acquire(&store).is_err(),
                    "Admission lock must remain owned through terminal reporting"
                );
            },
        )
        .unwrap();
        assert_eq!(changes, 1);
        assert_eq!(store.job(&job.id).unwrap().state, JobState::Cancelled);
        assert!(!root.join("repo").exists());
        assert!(RepoLock::acquire(&store).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn busy_local_operations_never_publish_phantom_jobs() {
        let (root, store, c, p) = fixture();
        let lock = RepoLock::acquire(&store).unwrap();
        assert!(run(&store, &c, &p, "init").is_err());
        assert!(store.jobs().unwrap().is_empty());
        let running = Job::new(&p.id, "backup");
        store.write_job(&running).unwrap();
        reconcile(&store).unwrap();
        assert_eq!(store.job(&running.id).unwrap().state, JobState::Running);
        drop(lock);
        reconcile(&store).unwrap();
        assert_eq!(store.job(&running.id).unwrap().state, JobState::Interrupted);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn concurrency_and_reconcile() {
        let (root, store, _, _) = fixture();
        let lock = RepoLock::acquire(&store).unwrap();
        assert!(RepoLock::acquire(&store).is_err());
        drop(lock);
        let j = Job::new("test", "backup");
        store.write_job(&j).unwrap();
        reconcile(&store).unwrap();
        assert_eq!(store.job(&j.id).unwrap().state, JobState::Interrupted);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_mount_refused() {
        let (root, _, _, _) = fixture();
        let d = Destination {
            id: "nas".into(),
            name: "NAS".into(),
            kind: DestinationKind::Nfs {
                path: root.join("repo"),
                mount_point: root.clone(),
                source: "missing:/share".into(),
                filesystem: "nfs4".into(),
            },
        };
        assert!(validate_destination(&d).is_err());
        assert!(!root.join("repo").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn schema_roundtrip() {
        let (root, _, c, _) = fixture();
        let s = toml::to_string(&c).unwrap();
        let loaded: Config = toml::from_str(&s).unwrap();
        loaded.validate().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}

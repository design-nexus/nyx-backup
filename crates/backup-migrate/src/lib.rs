//! Phase 3 user migration and a versioned Setup handoff. No root/system restore API.
use anyhow::{Context, Result, ensure};
use backup_core::{migration::*, *};
use backup_engine::RepoLock;
use backup_restore::{HomeMapping, Mode, Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Capture,
    Source {
        snapshot: String,
    },
    Prepare {
        snapshot: String,
        paths: Vec<String>,
        #[serde(default)]
        source: Option<Inventory>,
        #[serde(default)]
        policy: Policy,
        #[serde(default)]
        include_secrets: bool,
        #[serde(default = "yes")]
        rewrite_references: bool,
    },
    Inspect {
        bundle: String,
    },
    Bundles,
    Export {
        bundle: String,
        path: PathBuf,
    },
}
fn yes() -> bool {
    true
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Source { snapshot } | Self::Prepare { snapshot, .. } => ensure!(
                snapshot.len() == 64 && snapshot.bytes().all(|b| b.is_ascii_hexdigit()),
                "Choose an exact snapshot ID"
            ),
            Self::Inspect { bundle } | Self::Export { bundle, .. } => valid_id(bundle)?,
            _ => (),
        }
        if let Self::Prepare { paths, source, .. } = self {
            ensure!(paths.len() <= 256, "Select at most 256 file/folder paths");
            for path in paths {
                absolute(Path::new(path))?;
            }
            if let Some(source) = source {
                source.validate()?;
            }
        }
        if let Self::Export { path, .. } = self {
            absolute(path)?;
        }
        Ok(())
    }
    pub fn is_job(&self) -> bool {
        matches!(self, Self::Prepare { .. })
    }
    pub fn operation(&self) -> &'static str {
        if self.is_job() {
            "migration-plan"
        } else {
            "migration-read"
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageDecision {
    pub name: String,
    pub source_version: String,
    pub origin: String,
    pub action: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldFile {
    pub path: String,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub digest: String,
    pub created: u64,
    pub profile: String,
    pub snapshot: String,
    pub source: Inventory,
    pub source_attested: bool,
    pub target: Inventory,
    pub target_hardware: String,
    pub packages: Vec<PackageDecision>,
    pub held_files: Vec<HeldFile>,
    pub hardware_changes: Vec<String>,
    pub restore_plan: Option<String>,
    pub warnings: Vec<String>,
}
pub fn hardware_digest(hardware: &Hardware) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(hardware)?)
    ))
}
fn digest(bundle: &Bundle) -> Result<String> {
    let mut clean = bundle.clone();
    clean.digest.clear();
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&clean)?)))
}
impl Bundle {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == "nyx-backup-setup" && self.version == 1,
            "Unsupported Setup bundle"
        );
        valid_id(&self.id)?;
        valid_id(&self.profile)?;
        self.source.validate()?;
        self.target.validate()?;
        ensure!(
            self.target_hardware == hardware_digest(&self.target.hardware)?
                && self.digest == digest(self)?,
            "Setup bundle changed or is damaged; prepare it again"
        );
        for package in &self.packages {
            package_name(&package.name)?;
            ensure!(
                [
                    "install",
                    "review_hardware",
                    "review_foreign",
                    "dependency_only",
                    "review_platform",
                    "already_installed",
                    "review_repository",
                    "review_inventory",
                    "review_unavailable"
                ]
                .contains(&package.action.as_str()),
                "Unsupported Setup package action"
            );
        }
        if let Some(plan) = &self.restore_plan {
            valid_id(plan)?;
        }
        Ok(())
    }
}
pub fn hardware_package(name: &str) -> bool {
    if name.starts_with("linux-")
        || name.ends_with("-firmware")
        || name.contains("-dkms")
        || name.starts_with("virtualbox-host-modules")
    {
        return true;
    }
    [
        "linux",
        "linux-lts",
        "linux-zen",
        "linux-hardened",
        "intel-ucode",
        "amd-ucode",
        "mesa",
        "lib32-mesa",
        "asusctl",
        "supergfxctl",
        "rog-control-center",
        "broadcom-wl",
        "broadcom-wl-dkms",
    ]
    .contains(&name)
        || [
            "linux-firmware",
            "nvidia",
            "lib32-nvidia",
            "vulkan-",
            "lib32-vulkan-",
            "xf86-video-",
            "xf86-input-",
            "intel-media-driver",
            "libva-intel",
            "libva-mesa",
            "amdvlk",
            "lib32-amdvlk",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}
pub fn package_plan(
    source: &Inventory,
    target: &Inventory,
    available: Option<&std::collections::HashSet<String>>,
) -> Vec<PackageDecision> {
    let installed: std::collections::HashSet<_> =
        target.packages.iter().map(|p| p.name.as_str()).collect();
    let compatible = ["arch", "omarchy"].contains(&source.os_id.as_str())
        && ["arch", "omarchy"].contains(&target.os_id.as_str())
        && source.hardware.architecture == target.hardware.architecture;
    let inventory_known = !target
        .missing
        .iter()
        .any(|m| m.starts_with("Native/foreign package inventory unavailable"));
    source.packages.iter().map(|p|{let(action,reason)=if hardware_package(&p.name){("review_hardware","Keep the target installation's kernel, firmware and drivers; review this hardware package separately")}
 else if p.origin=="foreign"{("review_foreign","Foreign/local/AUR builds are inventoried; Setup never executes source builds automatically")}
 else if !p.explicit {("dependency_only","Dependency inventory retained; the target package manager resolves dependencies")}
 else if !compatible {("review_platform","Automatic package handoff requires matching Arch/Omarchy platform and architecture")}
 else if !inventory_known {("review_inventory","Target installed-package inventory is unavailable; repair it before handing packages to Setup")}
 else if installed.contains(p.name.as_str()){("already_installed","No downgrade or source-version pinning")}
 else if available.is_none(){("review_repository","Target sync databases were unavailable; no repository refresh is performed automatically")}
 else if !available.unwrap().contains(&p.name){("review_unavailable","Not found in the target's current configured sync databases")}
 else {("install","Explicit native package available in the target's configured repositories")};PackageDecision{name:p.name.clone(),source_version:p.version.clone(),origin:p.origin.clone(),action:action.into(),reason:reason.into()}}).collect()
}
fn hardware_changes(source: &Hardware, target: &Hardware) -> Vec<String> {
    let mut changes = Vec::new();
    if source.architecture != target.architecture {
        changes.push("CPU architecture differs; automatic package handoff is blocked".into());
    }
    if source.cpu_vendor != target.cpu_vendor {
        changes.push("CPU vendor differs; source microcode packages are withheld".into());
    }
    if source.gpus != target.gpus {
        changes.push(
            "Display devices differ; source graphics drivers/environment files are withheld".into(),
        );
    }
    if source.system_vendor != target.system_vendor || source.product != target.product {
        changes
            .push("Machine vendor/model differs; vendor-specific packages require review".into());
    }
    if source.displays != target.displays {
        changes.push("Display connectors/modes differ; prepared monitor configuration uses automatic target defaults".into());
    }
    if source.monitor_format != target.monitor_format {
        changes.push(
            "Hyprland monitor format differs; prepared monitor file uses the target format".into(),
        );
    }
    if changes.is_empty() {
        changes.push("No differences in the recorded hardware fields; physical compatibility is not guaranteed".into());
    }
    changes
}
fn bundle_path(store: &Store, id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    Ok(store.state.join("migrations").join(format!("{id}.json")))
}
pub fn load(store: &Store, id: &str, profile: &str) -> Result<Bundle> {
    let bundle: Bundle = serde_json::from_slice(&fs::read(bundle_path(store, id)?)?)?;
    bundle.validate()?;
    ensure!(
        bundle.profile == profile,
        "Setup bundle belongs to a different profile"
    );
    Ok(bundle)
}
fn source_inventory(
    lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    snapshot: &str,
    cancel: &AtomicBool,
) -> Result<Inventory> {
    let metadata = backup_restore::snapshot_metadata(lock, store, c, p, snapshot, cancel)?;
    let source:Inventory=serde_json::from_value(metadata["migration"].clone()).context("Snapshot predates the migration inventory. Supply an explicitly reviewed source manifest or make a new backup.")?;
    source.validate()?;
    Ok(source)
}
fn mapping(source: &Inventory, target: &Inventory, rewrite: bool) -> HomeMapping {
    HomeMapping {
        source_home: source.home.clone(),
        source_config: source.config.clone(),
        target_home: target.home.clone(),
        target_config: target.config.clone(),
        monitor_format: target.hardware.monitor_format.clone(),
        rewrite_references: rewrite,
    }
}
fn prepare(
    lock: &RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    request: &Request,
    cancel: &AtomicBool,
    job: &mut Job,
    changed: &mut impl FnMut(&Job),
) -> Result<Bundle> {
    let Request::Prepare {
        snapshot,
        paths,
        source,
        policy,
        include_secrets,
        rewrite_references,
    } = request
    else {
        anyhow::bail!("Expected migration preparation")
    };
    // Always validate snapshot ownership, even for an explicitly supplied source manifest.
    backup_restore::validate_snapshot(lock, store, c, p, snapshot, cancel)?;
    let attested = source.is_none();
    let source = match source {
        Some(source) => source.clone(),
        None => source_inventory(lock, store, c, p, snapshot, cancel)?,
    };
    source.validate()?;
    let target = backup_system::migration_inventory::capture();
    target.validate()?;
    let map = mapping(&source, &target, *rewrite_references);
    map.validate()?;
    let packages = package_plan(
        &source,
        &target,
        backup_system::migration_inventory::available_native().as_ref(),
    );
    let mut held = Vec::new();
    let mut allowed = Vec::new();
    if !paths.is_empty() {
        let nodes = backup_restore::snapshot_selection(lock, store, c, p, snapshot, paths, cancel)?;
        for node in nodes {
            if cancel.load(Ordering::SeqCst) {
                anyhow::bail!("Migration preparation cancelled")
            }
            if node.kind == "dir" {
                continue;
            }
            let path = Path::new(&node.path);
            let reason = if !*include_secrets && backup_engine::contains_recognized_secrets(path) {
                Some("Recognized credentials require explicit authorization".into())
            } else if node.path.contains(['\n', '\r']) {
                Some("Unusual filename requires a separate file restore".into())
            } else if node.kind == "symlink" {
                Some("Symlinks require a separate reviewed restore; migration never guesses how to remap their referents".into())
            } else if node.kind != "file" {
                Some("Special files are outside user migration".into())
            } else {
                map.target(path)
                    .and_then(|target| backup_restore::migration_target_allowed(store, c, &target))
                    .err()
                    .map(|e| e.to_string())
            };
            if let Some(reason) = reason {
                held.push(HeldFile {
                    path: node.path,
                    reason,
                });
            } else {
                allowed.push(node.path);
            }
        }
    }
    allowed.sort();
    allowed.dedup();
    ensure!(
        allowed.len() <= 256,
        "Migration supports at most 256 resolved files per reviewed plan; select a smaller configuration folder. Use separate-folder restore for bulk data."
    );
    let mut warnings: Vec<String> = source
        .missing
        .iter()
        .map(|m| format!("Source: {m}"))
        .collect();
    warnings.extend(target.missing.iter().map(|m| format!("Target: {m}")));
    warnings.push("Package versions are inventory only; no downgrades, removals, repository edits, system restore or automatic driver installation are performed.".into());
    warnings.push("Foreign packages, Flatpaks and hardware packages require separate review. Actual new-machine boot/recovery remains unqualified.".into());
    if !attested {
        warnings.push(
            "Source manifest explicitly supplied by user; it is not attested by snapshot metadata."
                .into(),
        );
    }
    let restore_plan = if allowed.is_empty() {
        None
    } else {
        let request = backup_restore::Request::Plan {
            snapshot: snapshot.clone(),
            paths: allowed,
            target: None,
            mode: Mode::MigratedUserData(map),
            policy: policy.clone(),
            include_secrets: *include_secrets,
        };
        let plan =
            backup_restore::prepare_migration(lock, store, c, p, &request, cancel, job, changed)?;
        backup_restore::append_plan_messages(store,&p.id,&plan.id,vec!["Migration plan: source home/configuration locations are mapped into this user's roots; hardware-sensitive files are held and prepared monitor defaults require review.".into()])?;
        Some(plan.id)
    };
    let mut bundle = Bundle {
        format: "nyx-backup-setup".into(),
        version: 1,
        id: id(),
        digest: String::new(),
        created: now(),
        profile: p.id.clone(),
        snapshot: snapshot.clone(),
        source_attested: attested,
        source: source.clone(),
        target_hardware: hardware_digest(&target.hardware)?,
        hardware_changes: hardware_changes(&source.hardware, &target.hardware),
        target,
        packages,
        held_files: held,
        restore_plan,
        warnings,
    };
    bundle.digest = digest(&bundle)?;
    bundle.validate()?;
    ensure!(
        serde_json::to_vec(&bundle)?.len() <= 8 * 1024 * 1024,
        "Setup bundle is too large"
    );
    atomic(
        &bundle_path(store, &bundle.id)?,
        serde_json::to_vec_pretty(&bundle)?,
    )?;
    Ok(bundle)
}
pub fn execute_locked(
    lock: RepoLock,
    store: &Store,
    c: &Config,
    p: &Profile,
    request: Request,
    cancel: Arc<AtomicBool>,
    job: &mut Job,
    mut changed: impl FnMut(&Job),
) -> Result<()> {
    request.validate()?;
    let result = prepare(&lock, store, c, p, &request, &cancel, job, &mut changed);
    match result {
        Ok(bundle) => {
            job.result = serde_json::json!({"version":1,"bundle":bundle.id,"restore_plan":bundle.restore_plan,"install_packages":bundle.packages.iter().filter(|p|p.action=="install").count(),"held_files":bundle.held_files.len(),"hardware_changes":bundle.hardware_changes});
            job.state = JobState::Success;
            job.message="Migration and Setup bundle ready. Review package decisions and the staged restore plan before applying anything.".into();
        }
        Err(e) => {
            job.state = if cancel.load(Ordering::SeqCst) {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            job.message = e.to_string();
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
    let lock = RepoLock::acquire(store)?;
    match request {
        Request::Capture => Ok(serde_json::to_value(
            backup_system::migration_inventory::capture(),
        )?),
        Request::Source { snapshot } => Ok(serde_json::to_value(source_inventory(
            &lock,
            store,
            c,
            p,
            snapshot,
            &AtomicBool::new(false),
        )?)?),
        Request::Inspect { bundle } => Ok(serde_json::to_value(load(store, bundle, &p.id)?)?),
        Request::Bundles => {
            let directory = store.state.join("migrations");
            let mut bundles = Vec::new();
            if directory.exists() {
                for entry in fs::read_dir(directory)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|s| s == "json") {
                        let bundle: Bundle = serde_json::from_slice(&fs::read(path)?)?;
                        bundle.validate()?;
                        if bundle.profile == p.id {
                            bundles.push(serde_json::json!({"id":bundle.id,"created":bundle.created,"snapshot":bundle.snapshot,"restore_plan":bundle.restore_plan}));
                        }
                    }
                }
            }
            Ok(serde_json::json!({"version":1,"bundles":bundles}))
        }
        Request::Export { bundle, path } => {
            let bundle = load(store, bundle, &p.id)?;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .context("Choose a new Setup bundle file; existing files are never overwritten")?;
            file.write_all(&serde_json::to_vec_pretty(&bundle)?)?;
            file.sync_all()?;
            Ok(serde_json::json!({"version":1,"bundle":bundle.id,"exported":path}))
        }
        _ => anyhow::bail!("Use a background migration job"),
    }
}
pub fn import_bundle(path: &Path) -> Result<Bundle> {
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "Setup bundle is too large"
    );
    let bundle: Bundle = serde_json::from_slice(&fs::read(path)?)?;
    bundle.validate()?;
    Ok(bundle)
}
/// Setup's consumer recomputes eligibility against the actual target; it never runs
/// commands or package lists supplied verbatim by the bundle.
pub fn setup_preview(bundle: &Bundle) -> Result<serde_json::Value> {
    bundle.validate()?;
    let current = backup_system::migration_inventory::capture();
    current.validate()?;
    ensure!(
        hardware_digest(&current.hardware)? == bundle.target_hardware
            && current.home == bundle.target.home
            && current.config == bundle.target.config,
        "Setup target hardware/home/configuration changed. Prepare a new migration plan on this target."
    );
    let decisions = package_plan(
        &bundle.source,
        &current,
        backup_system::migration_inventory::available_native().as_ref(),
    );
    let approved: std::collections::HashSet<_> = bundle
        .packages
        .iter()
        .filter(|p| p.action == "install")
        .map(|p| p.name.as_str())
        .collect();
    let install: Vec<_> = decisions
        .iter()
        .filter(|p| p.action == "install" && approved.contains(p.name.as_str()))
        .map(|p| p.name.clone())
        .collect();
    let blocked: Vec<_> = decisions
        .iter()
        .filter(|p| {
            approved.contains(p.name.as_str())
                && !["install", "already_installed"].contains(&p.action.as_str())
        })
        .cloned()
        .collect();
    ensure!(
        blocked.is_empty(),
        "A reviewed package is no longer eligible; prepare a new Setup plan"
    );
    Ok(
        serde_json::json!({"version":1,"bundle":bundle.id,"install":install,"decisions":decisions,"flatpaks_for_review":bundle.source.flatpaks,"restore_plan":bundle.restore_plan,"command_prefix":["sudo","pacman","-S","--needed","--"],"message":"Preview only. Apply requires exact bundle confirmation. Keep the target fully updated using its normal maintenance workflow; Setup does not refresh repositories or perform partial upgrades."}),
    )
}
pub fn setup_install(bundle: &Bundle, confirmation: &str) -> Result<()> {
    ensure!(
        confirmation == bundle.id,
        "Confirm the exact Setup bundle ID"
    );
    let preview = setup_preview(bundle)?;
    let packages: Vec<_> = preview["install"]
        .as_array()
        .context("Missing package decisions")?
        .iter()
        .filter_map(|p| p.as_str())
        .collect();
    for package in &packages {
        package_name(package)?;
    }
    if !packages.is_empty() {
        let mut child = std::process::Command::new("sudo")
            .args(["pacman", "-S", "--needed", "--"])
            .args(packages)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;
        if let Some(mut output) = child.stdout.take() {
            std::io::copy(&mut output, &mut std::io::stderr())?;
        }
        let status = child.wait()?;
        ensure!(
            status.success(),
            "Package installation failed; no configuration restore was started"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inventory(architecture: &str) -> Inventory {
        Inventory {
            version: 1,
            captured: 1,
            os_id: "arch".into(),
            home: "/home/source".into(),
            config: "/home/source/.config".into(),
            hardware: Hardware {
                architecture: architecture.into(),
                cpu_vendor: "vendor".into(),
                system_vendor: "vendor".into(),
                product: "model".into(),
                gpus: vec![],
                displays: vec![],
                monitor_format: "lua".into(),
            },
            packages: vec![],
            flatpaks: vec![],
            missing: vec![],
        }
    }
    fn package(name: &str, origin: &str, explicit: bool) -> Package {
        Package {
            name: name.into(),
            version: "1.0".into(),
            origin: origin.into(),
            explicit,
        }
    }
    #[test]
    fn package_selection_never_replays_hardware_or_builds() {
        let mut source = inventory("x86_64");
        source.packages = vec![
            package("git", "native", true),
            package("nvidia-open", "native", true),
            package("foreign-app", "foreign", true),
            package("lib-dependency", "native", false),
            package("unknown", "native", true),
        ];
        let target = inventory("x86_64");
        let available = std::collections::HashSet::from(["git".into(), "nvidia-open".into()]);
        let decisions = package_plan(&source, &target, Some(&available));
        assert_eq!(
            decisions
                .iter()
                .filter(|p| p.action == "install")
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["git"]
        );
        assert_eq!(decisions[1].action, "review_hardware");
        assert_eq!(decisions[2].action, "review_foreign");
        assert_eq!(decisions[3].action, "dependency_only");
        assert_eq!(decisions[4].action, "review_unavailable");
    }
    #[test]
    fn architecture_and_missing_database_block_handoff() {
        let mut source = inventory("aarch64");
        source.packages = vec![package("git", "native", true)];
        let target = inventory("x86_64");
        let available = std::collections::HashSet::from(["git".into()]);
        assert_eq!(
            package_plan(&source, &target, Some(&available))[0].action,
            "review_platform"
        );
        source.hardware.architecture = "x86_64".into();
        assert_eq!(
            package_plan(&source, &target, None)[0].action,
            "review_repository"
        );
    }
    #[test]
    fn package_arguments_cannot_be_options_urls_or_commands() {
        for invalid in [
            "--overwrite",
            "bad;touch",
            "repo/pkg",
            "https://server/pkg",
            "line\nbreak",
            "$(id)",
        ] {
            assert!(package_name(invalid).is_err());
        }
        assert!(package_name("valid-package+2").is_ok());
    }
    #[test]
    fn bundle_digest_detects_changed_handoff() {
        let source = inventory("x86_64");
        let mut bundle = Bundle {
            format: "nyx-backup-setup".into(),
            version: 1,
            id: "fixture".into(),
            digest: String::new(),
            created: 1,
            profile: "fixture".into(),
            snapshot: "a".repeat(64),
            source: source.clone(),
            source_attested: true,
            target: source.clone(),
            target_hardware: hardware_digest(&source.hardware).unwrap(),
            packages: vec![],
            held_files: vec![],
            hardware_changes: vec![],
            restore_plan: None,
            warnings: vec![],
        };
        bundle.digest = digest(&bundle).unwrap();
        bundle.validate().unwrap();
        bundle.target.home = "/home/other".into();
        assert!(bundle.validate().is_err());
    }
}

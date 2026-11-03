//! Versioned on-disk models. No GTK or service runtime dependencies.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
pub const APP_ID: &str = "io.github.design_nexus.nyx.Backup";
pub const BUS: &str = "io.github.design_nexus.nyx.Backup1";
pub const OBJECT: &str = "/io/github/design_nexus/nyx/Backup1";
pub fn init() {
    nyx_core::app::init(nyx_core::app::App {
        slug: "nyx-backup",
        name: "Backup",
    });
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn valid_id(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "Use 1–64 letters, digits, dashes or underscores for IDs"
    );
    Ok(())
}
pub fn private_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn atomic(p: &Path, data: impl AsRef<[u8]>) -> Result<()> {
    private_dir(p.parent().context("Missing parent directory")?)?;
    nyx_core::cmd::atomic_write(p, data)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[derive(Clone)]
pub struct Store {
    pub config: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            config: nyx_core::paths::app_dir(),
            state: nyx_core::paths::state_dir(),
            cache: nyx_core::paths::cache_dir(),
        }
    }
}
impl Store {
    pub fn load(&self) -> Result<Config> {
        let p = self.config.join("config.toml");
        if !p.exists() {
            return Ok(Config::default());
        }
        let c: Config = toml::from_str(&fs::read_to_string(p)?)?;
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self, c: &Config) -> Result<()> {
        c.validate()?;
        atomic(&self.config.join("config.toml"), toml::to_string_pretty(c)?)
    }
    pub fn jobs(&self) -> Result<Vec<Job>> {
        let p = self.state.join("jobs");
        if !p.exists() {
            return Ok(vec![]);
        }
        let mut jobs = Vec::new();
        for e in fs::read_dir(p)? {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "json") {
                jobs.push(serde_json::from_slice::<Job>(&fs::read(p)?)?);
            }
        }
        jobs.sort_by_key(|j| std::cmp::Reverse(j.started));
        Ok(jobs)
    }
    pub fn job(&self, id: &str) -> Result<Job> {
        valid_id(id)?;
        Ok(serde_json::from_slice(&fs::read(
            self.state.join("jobs").join(format!("{id}.json")),
        )?)?)
    }
    pub fn write_job(&self, j: &Job) -> Result<()> {
        valid_id(&j.id)?;
        atomic(
            &self.state.join("jobs").join(format!("{}.json", j.id)),
            serde_json::to_vec_pretty(j)?,
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default = "yes")]
    pub notifications: bool,
    #[serde(default)]
    pub destinations: Vec<Destination>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}
fn yes() -> bool {
    true
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            notifications: true,
            destinations: vec![],
            profiles: vec![],
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported configuration version");
        let mut ids = std::collections::HashSet::new();
        for d in &self.destinations {
            valid_id(&d.id)?;
            ensure!(ids.insert(&d.id), "Duplicate destination ID");
            d.validate()?;
        }
        ids.clear();
        for p in &self.profiles {
            valid_id(&p.id)?;
            ensure!(ids.insert(&p.id), "Duplicate profile ID");
            self.destination(&p.destination)?;
            ensure!(!p.name.trim().is_empty(), "Profile needs a name");
            ensure!(!p.sources.is_empty(), "Select at least one source");
            for s in &p.sources {
                ensure!(
                    s.is_absolute() && !s.to_string_lossy().contains(['\n', '\r']),
                    "Sources must be absolute paths without newlines"
                );
            }
            ensure!(p.keep_last > 0, "Keep at least one snapshot");
            ensure!(
                p.exclusions.iter().all(|s| !s.contains(['\n', '\r'])),
                "Exclusions must be single-line patterns"
            );
        }
        Ok(())
    }
    pub fn destination(&self, id: &str) -> Result<&Destination> {
        self.destinations
            .iter()
            .find(|d| d.id == id)
            .context("Unknown destination")
    }
    pub fn profile(&self, id: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .context("Unknown profile")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub kind: DestinationKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DestinationKind {
    Local {
        path: PathBuf,
    },
    Mounted {
        path: PathBuf,
        mount_point: PathBuf,
        source: String,
        filesystem: String,
        #[serde(default)]
        uuid: String,
    },
    Nfs {
        path: PathBuf,
        mount_point: PathBuf,
        source: String,
        filesystem: String,
    },
    Sftp {
        host: String,
        path: String,
    },
    S3 {
        endpoint: String,
        region: String,
        bucket: String,
        path: String,
    },
    Smb {
        server: String,
        share: String,
        path: String,
        username: String,
        #[serde(default)]
        domain: String,
    },
}
impl Destination {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "Destination needs a name");
        match &self.kind {
            DestinationKind::Local { path }
            | DestinationKind::Mounted { path, .. }
            | DestinationKind::Nfs { path, .. } => ensure!(
                path.is_absolute()
                    && !path.to_string_lossy().contains(['\n', '\r'])
                    && !path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir),
                "Destination needs an absolute path without parent-directory (..) components"
            ),
            DestinationKind::Sftp { host, path } => {
                ensure!(
                    !host.is_empty()
                        && host
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b))
                        && !host.starts_with('-'),
                    "Use an SSH host alias, optionally user@host"
                );
                ensure!(
                    path.starts_with('/') && !path.contains(['\n', '\r']),
                    "SFTP path must be absolute"
                );
            }
            DestinationKind::S3 {
                endpoint,
                region,
                bucket,
                path,
            } => {
                if !endpoint.is_empty() {
                    let url = url::Url::parse(endpoint).context("Use an HTTPS S3 endpoint")?;
                    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
                    ensure!(
                        (url.scheme() == "https" || (url.scheme() == "http" && loopback))
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none()
                            && url.path() == "/",
                        "S3 endpoint must be HTTPS without credentials, paths or query parameters; HTTP is limited to literal loopback fixtures"
                    );
                }
                ensure!(
                    !bucket.is_empty()
                        && bucket.len() <= 63
                        && bucket.bytes().all(|b| b.is_ascii_lowercase()
                            || b.is_ascii_digit()
                            || b".-".contains(&b))
                        && !bucket.starts_with(['.', '-'])
                        && !bucket.ends_with(['.', '-']),
                    "Use a valid lower-case S3 bucket name"
                );
                ensure!(
                    !region.is_empty()
                        && region
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
                    "S3 region is required"
                );
                ensure!(
                    path.split('/').all(|s| s != ".." && s != ".")
                        && path
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b)),
                    "S3 repository prefix may contain letters, digits, slashes, dashes, underscores and dots; no traversal"
                );
            }
            DestinationKind::Smb {
                server,
                share,
                path,
                username,
                domain,
            } => {
                for s in [server, share, path, username, domain] {
                    ensure!(
                        !s.contains(['\n', '\r', '[', ']']),
                        "SMB fields cannot contain newlines or brackets"
                    );
                }
                ensure!(
                    !server.is_empty() && !share.is_empty() && !username.is_empty(),
                    "SMB server, share and username are required"
                );
                ensure!(
                    !share.contains('/') && !path.split('/').any(|s| s == ".."),
                    "Invalid SMB share or path"
                );
            }
        }
        if let DestinationKind::Mounted {
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
        } = &self.kind
        {
            ensure!(
                mount_point.is_absolute()
                    && !mount_point
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
                    && path.starts_with(mount_point)
                    && !source.is_empty()
                    && !filesystem.is_empty(),
                "Record an absolute mount point, source and filesystem; keep the repository inside that mount"
            );
        }
        Ok(())
    }
    pub fn local_path(&self) -> Option<&Path> {
        match &self.kind {
            DestinationKind::Local { path }
            | DestinationKind::Mounted { path, .. }
            | DestinationKind::Nfs { path, .. } => Some(path),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub destination: String,
    pub sources: Vec<PathBuf>,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub include_secrets: bool,
    #[serde(default)]
    pub approved: bool,
    #[serde(default = "retention")]
    pub keep_last: u32,
    #[serde(default)]
    pub schedule: Option<String>,
}
fn retention() -> u32 {
    7
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub version: u32,
    pub profile: String,
    pub sources: Vec<PathBuf>,
    pub exclusions: Vec<String>,
    pub logical_bytes: u64,
    pub files: u64,
    pub unreadable: u64,
    pub warnings: Vec<String>,
    pub blocked: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Success,
    Incomplete,
    Failed,
    Cancelled,
    Interrupted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub version: u32,
    pub id: String,
    pub profile: String,
    pub operation: String,
    pub started: u64,
    pub finished: Option<u64>,
    pub state: JobState,
    pub progress: serde_json::Value,
    pub result: serde_json::Value,
    pub message: String,
}
impl Job {
    pub fn new(profile: &str, operation: &str) -> Self {
        Self {
            version: 1,
            id: id(),
            profile: profile.into(),
            operation: operation.into(),
            started: now(),
            finished: None,
            state: JobState::Running,
            progress: serde_json::Value::Null,
            result: serde_json::Value::Null,
            message: String::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Health {
    pub version: u32,
    pub coverage: String,
    pub active: Option<Job>,
    pub last_success: Option<Job>,
    pub next_schedule: Vec<serde_json::Value>,
    pub available_actions: Vec<String>,
    pub warning: Option<String>,
    #[serde(default)]
    pub storage: Option<serde_json::Value>,
}
pub fn health(store: &Store) -> Result<Health> {
    let jobs = store.jobs()?;
    Ok(Health {
        version: 1,
        coverage: "Selected user data and readable system metadata; no system recovery guarantee"
            .into(),
        active: jobs.iter().find(|j| j.state == JobState::Running).cloned(),
        last_success: jobs
            .iter()
            .find(|j| j.operation == "backup" && j.state == JobState::Success)
            .cloned(),
        next_schedule: vec![],
        available_actions: vec![
            "preview".into(),
            "backup".into(),
            "history".into(),
            "verify".into(),
        ],
        storage: jobs
            .iter()
            .find(|j| j.operation == "stats" && j.state == JobState::Success)
            .map(|j| j.result.clone()),
        warning: jobs
            .iter()
            .find(|j| j.operation == "backup" && j.state != JobState::Running)
            .filter(|j| j.state != JobState::Success)
            .map(|j| j.message.clone()),
    })
}

pub mod migration;

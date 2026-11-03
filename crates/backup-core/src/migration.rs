//! Versioned, credential-free package and hardware inventory for user-data migration.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub explicit: bool,
    pub origin: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Flatpak {
    pub application: String,
    pub branch: String,
    pub architecture: String,
    pub origin: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Gpu {
    pub vendor: String,
    pub device: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub connector: String,
    pub modes: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Hardware {
    pub architecture: String,
    pub cpu_vendor: String,
    pub system_vendor: String,
    pub product: String,
    pub gpus: Vec<Gpu>,
    pub displays: Vec<Display>,
    pub monitor_format: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub version: u32,
    pub captured: u64,
    pub os_id: String,
    pub home: PathBuf,
    pub config: PathBuf,
    pub hardware: Hardware,
    pub packages: Vec<Package>,
    pub flatpaks: Vec<Flatpak>,
    pub missing: Vec<String>,
}
pub fn absolute(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path.components().any(|c| matches!(c, Component::ParentDir))
            && !path.as_os_str().is_empty(),
        "Inventory paths must be absolute without parent traversal"
    );
    Ok(())
}
pub fn package_name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 200
            && !s.starts_with(['-', '.'])
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b)),
        "Invalid package name"
    );
    Ok(())
}
impl Inventory {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported inventory version");
        absolute(&self.home)?;
        absolute(&self.config)?;
        ensure!(
            self.home != Path::new("/") && self.config != Path::new("/"),
            "Inventory home/config cannot be filesystem root"
        );
        ensure!(
            !self.hardware.architecture.is_empty() && self.hardware.architecture.len() < 80,
            "Missing architecture"
        );
        ensure!(
            self.packages.len() <= 100_000 && self.flatpaks.len() <= 10_000,
            "Inventory is too large"
        );
        let mut seen = std::collections::HashSet::new();
        for p in &self.packages {
            package_name(&p.name)?;
            ensure!(seen.insert(&p.name), "Duplicate package name");
            ensure!(
                ["native", "foreign"].contains(&p.origin.as_str()),
                "Unknown package origin"
            );
        }
        Ok(())
    }
}

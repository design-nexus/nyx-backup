//! Read-only local queries. No repository refresh, driver selection, serial/MAC or credentials.
use backup_core::{migration::*, now};
use std::{fs, path::Path, process::Stdio};
fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
fn query(program: &str, args: &[&str]) -> Option<String> {
    let out = super::clean_command(program)
        .args(args)
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if (!out.status.success()
        && !(out.status.code() == Some(1)
            && ["-Qmq", "-Qeq"].iter().any(|a| args == [*a])
            && out.stdout.is_empty()
            && out.stderr.is_empty()))
        || out.stdout.len() > 32 * 1024 * 1024
    {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
pub fn hardware() -> Hardware {
    let architecture = query("uname", &["-m"])
        .unwrap_or_else(|| std::env::consts::ARCH.into())
        .trim()
        .to_string();
    let cpu = text("/proc/cpuinfo")
        .lines()
        .find_map(|l| {
            l.strip_prefix("vendor_id")
                .and_then(|s| s.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_default();
    let mut gpus = Vec::new();
    if let Ok(devices) = fs::read_dir("/sys/bus/pci/devices") {
        for device in devices.flatten() {
            if text(device.path().join("class")).starts_with("0x03") {
                gpus.push(Gpu {
                    vendor: text(device.path().join("vendor")),
                    device: text(device.path().join("device")),
                });
            }
        }
    }
    gpus.sort_by(|a, b| (&a.vendor, &a.device).cmp(&(&b.vendor, &b.device)));
    gpus.dedup();
    let mut displays = Vec::new();
    if let Ok(connectors) = fs::read_dir("/sys/class/drm") {
        for connector in connectors.flatten() {
            if text(connector.path().join("status")) == "connected" {
                let modes = text(connector.path().join("modes"))
                    .lines()
                    .take(64)
                    .map(Into::into)
                    .collect();
                displays.push(Display {
                    connector: connector.file_name().to_string_lossy().to_string(),
                    modes,
                });
            }
        }
    }
    displays.sort_by(|a, b| a.connector.cmp(&b.connector));
    let config = nyx_core::paths::config_home();
    let monitor_format = if config.join("hypr/hyprland.lua").exists() {
        "lua"
    } else if config.join("hypr/hyprland.conf").exists() {
        "conf"
    } else if Path::new("/usr/share/omarchy/config/hypr/monitors.lua").exists() {
        "lua"
    } else {
        "unknown"
    };
    Hardware {
        architecture,
        cpu_vendor: cpu,
        system_vendor: text("/sys/class/dmi/id/sys_vendor"),
        product: text("/sys/class/dmi/id/product_name"),
        gpus,
        displays,
        monitor_format: monitor_format.into(),
    }
}
pub fn capture() -> Inventory {
    let raw_home = nyx_core::paths::home();
    let raw_config = nyx_core::paths::config_home();
    let home = fs::canonicalize(&raw_home).unwrap_or(raw_home.clone());
    let config = fs::canonicalize(&raw_config).unwrap_or(raw_config.clone());
    let mut missing =
        vec!["Local user binaries and language-specific tool managers are not inventoried".into()];
    if home != raw_home || config != raw_config {
        missing.push("Home/config aliases differ from physical roots; references to aliases require manual review".into());
    }
    let all = query("pacman", &["-Q"]);
    let foreign = query("pacman", &["-Qmq"]);
    let explicit = query("pacman", &["-Qeq"]);
    let mut packages = Vec::new();
    if let (Some(all), Some(foreign), Some(explicit)) = (all, foreign, explicit) {
        let foreign: std::collections::HashSet<_> = foreign.lines().collect();
        let explicit: std::collections::HashSet<_> = explicit.lines().collect();
        for line in all.lines() {
            let Some((name, version)) = line.split_once(' ') else {
                missing.push("Malformed native package record".into());
                continue;
            };
            if package_name(name).is_err() {
                missing.push("Invalid package name omitted".into());
                continue;
            }
            packages.push(Package {
                name: name.into(),
                version: version.trim().into(),
                explicit: explicit.contains(name),
                origin: if foreign.contains(name) {
                    "foreign"
                } else {
                    "native"
                }
                .into(),
            });
        }
    } else {
        missing.push("Native/foreign package inventory unavailable (pacman read queries)".into());
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let mut flatpaks = Vec::new();
    if let Some(output) = query(
        "flatpak",
        &["list", "--app", "--columns=application,branch,arch,origin"],
    ) {
        for line in output.lines() {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() == 4 {
                flatpaks.push(Flatpak {
                    application: fields[0].into(),
                    branch: fields[1].into(),
                    architecture: fields[2].into(),
                    origin: fields[3].into(),
                });
            }
        }
    } else {
        missing.push("Flatpak application inventory unavailable".into());
    }
    let os_id = text("/etc/os-release")
        .lines()
        .find_map(|line| {
            line.strip_prefix("ID=")
                .map(|s| s.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    let hardware = hardware();
    if hardware.system_vendor.is_empty() || hardware.product.is_empty() {
        missing.push("DMI model/vendor unavailable".into());
    }
    if hardware.gpus.is_empty() {
        missing.push("PCI display-device inventory unavailable or empty".into());
    }
    Inventory {
        version: 1,
        captured: now(),
        os_id,
        home,
        config,
        hardware,
        packages,
        flatpaks,
        missing,
    }
}
pub fn available_native() -> Option<std::collections::HashSet<String>> {
    query("pacman", &["-Slq"]).map(|s| {
        s.lines()
            .filter(|s| package_name(s).is_ok())
            .map(Into::into)
            .collect()
    })
}

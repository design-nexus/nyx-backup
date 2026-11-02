//! Versioned session API. No credentials, destination mutation, deletion or restore methods.
use anyhow::Result;
use backup_core::*;
pub fn client_call(
    method: &str,
    body: impl serde::ser::Serialize + zbus::zvariant::DynamicType,
) -> Result<String> {
    let c = zbus::blocking::Connection::session()?;
    let p = zbus::blocking::Proxy::new(&c, BUS, OBJECT, BUS)?;
    Ok(p.call(method, &body)?)
}
pub fn status() -> Result<String> {
    client_call("Status", ())
}
pub fn start(profile: &str, operation: &str) -> Result<String> {
    client_call("Start", (profile, operation))
}
pub fn capabilities() -> serde_json::Value {
    serde_json::json!({"version":1,"coverage":"selected_user_data_and_readable_metadata","operations":["preview","backup","history","stats","verify","verify-full","cancel"],"restore":false,"local_restore":true,"local_restore_operations":["snapshots","browse","plan","diff","apply","rollback"],"full_system_recovery":false,"automatic_retention":false})
}

/// Private local administration requests. These operations are never D-Bus methods.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminRequest {
    pub version: u32,
    pub profile: String,
    pub operation: String,
    pub confirm_profile: Option<String>,
    pub configuration: std::path::PathBuf,
    #[serde(default)]
    pub restore: Option<backup_restore::Request>,
}
impl AdminRequest {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.version == 1, "Unsupported administration protocol");
        backup_core::valid_id(&self.profile)?;
        anyhow::ensure!(
            self.configuration.is_absolute(),
            "Administration requires an absolute configuration path"
        );
        if let Some(restore) = &self.restore {
            restore.validate()?;
            anyhow::ensure!(
                self.operation == restore.operation(),
                "Restore request operation does not match its payload"
            );
            return Ok(());
        }
        anyhow::ensure!(
            ["init", "validate", "retention-preview", "retention-apply"]
                .contains(&self.operation.as_str()),
            "Unsupported local administration operation"
        );
        if self.operation == "retention-apply" {
            anyhow::ensure!(
                self.confirm_profile.as_deref() == Some(&self.profile),
                "Confirm the exact profile ID before applying retention"
            );
        }
        Ok(())
    }
}
pub fn configuration_file(store: &Store) -> std::path::PathBuf {
    let path = store.config.join("config.toml");
    std::fs::canonicalize(&path).unwrap_or(path)
}
pub fn admin_socket(store: &Store) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    store.state.hash(&mut hash);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| format!("/run/user/{}", unsafe { libc::geteuid() }).into());
    runtime
        .join("nyx-backup")
        .join(format!("admin-{:016x}.sock", hash.finish()))
}
pub fn admin_start(
    profile: &str,
    operation: &str,
    confirm_profile: Option<&str>,
) -> Result<String> {
    let request = AdminRequest {
        version: 1,
        profile: profile.into(),
        operation: operation.into(),
        confirm_profile: confirm_profile.map(Into::into),
        configuration: configuration_file(&Store::default()),
        restore: None,
    };
    let response = local_request(request)?;
    let id = response["job"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Administration listener returned no job ID"))?;
    backup_core::valid_id(id)?;
    Ok(id.into())
}
fn local_request(request: AdminRequest) -> Result<serde_json::Value> {
    use std::io::{Read, Write};
    request.validate()?;
    // Activate the same application-owned service used by ordinary backup jobs.
    client_call("Capabilities", ())?;
    let socket = admin_socket(&Store::default());
    let mut stream = None;
    for _ in 0..40 {
        match std::os::unix::net::UnixStream::connect(&socket) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut stream=stream.ok_or_else(||anyhow::anyhow!("Backup administration listener is unavailable; update or restart the idle background service"))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    writeln!(stream, "{}", serde_json::to_string(&request)?)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = String::new();
    stream
        .take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut response)?;
    anyhow::ensure!(
        response.len() <= 16 * 1024 * 1024,
        "Local restore result is too large; browse a smaller folder"
    );
    let response: serde_json::Value = serde_json::from_str(&response)?;
    if let Some(error) = response["error"].as_str() {
        anyhow::bail!("{error}")
    }
    Ok(response)
}
pub fn restore_read(profile: &str, restore: backup_restore::Request) -> Result<serde_json::Value> {
    anyhow::ensure!(
        !restore.is_job(),
        "Use a background job for restore changes"
    );
    let response = local_request(AdminRequest {
        version: 1,
        profile: profile.into(),
        operation: restore.operation().into(),
        confirm_profile: None,
        configuration: configuration_file(&Store::default()),
        restore: Some(restore),
    })?;
    Ok(response["result"].clone())
}
pub fn restore_start(profile: &str, restore: backup_restore::Request) -> Result<String> {
    anyhow::ensure!(restore.is_job(), "This restore operation is read-only");
    let response = local_request(AdminRequest {
        version: 1,
        profile: profile.into(),
        operation: restore.operation().into(),
        confirm_profile: None,
        configuration: configuration_file(&Store::default()),
        restore: Some(restore),
    })?;
    let id = response["job"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Restore listener returned no job ID"))?;
    backup_core::valid_id(id)?;
    Ok(id.into())
}

pub fn wait_job(id: &str) -> Result<serde_json::Value> {
    loop {
        let job: serde_json::Value = serde_json::from_str(&client_call("Job", (id,))?)?;
        if job["state"] != "running" {
            return Ok(job);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
pub fn admin_run(
    profile: &str,
    operation: &str,
    confirm_profile: Option<&str>,
) -> Result<serde_json::Value> {
    wait_job(&admin_start(profile, operation, confirm_profile)?)
}

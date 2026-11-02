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
    serde_json::json!({"version":1,"coverage":"selected_user_data_and_readable_metadata","operations":["preview","backup","history","stats","verify","verify-full","cancel"],"restore":false,"full_system_recovery":false,"automatic_retention":false})
}

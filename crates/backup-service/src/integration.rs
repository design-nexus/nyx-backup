//! App-owned grants, bound to an authenticated session-bus connection, never bearer secrets.
use anyhow::{Context, Result, ensure};
use backup_core::*;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
pub const CONTROL: &str = "io.github.design_nexus.nyx.Control";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub version: u32,
    pub id: String,
    pub consumer: String,
    pub owner: String,
    pub uid: u32,
    pub profile: String,
    pub actions: Vec<String>,
    pub expires: u64,
}
fn path(s: &Store, id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    Ok(s.state.join("grants").join(format!("{id}.json")))
}
pub fn list(s: &Store) -> Result<Vec<Grant>> {
    let p = s.state.join("grants");
    if !p.exists() {
        return Ok(vec![]);
    }
    let mut a = Vec::new();
    for entry in fs::read_dir(p)?.take(1024) {
        let p = entry?.path();
        if p.extension().is_some_and(|s| s == "json") {
            a.push(serde_json::from_slice(&fs::read(p)?)?);
        }
    }
    Ok(a)
}
pub fn issue(
    s: &Store,
    consumer: &str,
    profile: &str,
    actions: &[String],
    seconds: u64,
) -> Result<Grant> {
    zbus::names::WellKnownName::try_from(consumer)?;
    ensure!(
        !consumer.starts_with("org.freedesktop."),
        "Choose a specific application consumer bus name"
    );
    ensure!(
        (60..=86400).contains(&seconds),
        "Grant lifetime must be 60–86400 seconds"
    );
    ensure!(
        !actions.is_empty()
            && actions
                .iter()
                .all(|s| ["status", "backup", "verify", "cancel"].contains(&s.as_str())),
        "Grant supports status, backup, verify and cancel only"
    );
    ensure!(
        s.load()?.profile(profile)?.approved,
        "Approve the profile before granting integration access"
    );
    // Explicit grant setup may activate the on-demand service; passive tiles never do.
    crate::client_call("Capabilities", ())?;
    let (owner, uid) = zbus::block_on(async {
        let c = zbus::Connection::session().await?;
        let p = zbus::Proxy::new(
            &c,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
        )
        .await?;
        let owner: String = p.call("GetNameOwner", &(consumer,)).await?;
        let uid: u32 = p.call("GetConnectionUnixUser", &(&owner,)).await?;
        ensure!(
            uid == unsafe { libc::geteuid() },
            "Consumer must run as the current user"
        );
        Ok::<_, anyhow::Error>((owner, uid))
    })?;
    let g = Grant {
        version: 1,
        id: id(),
        consumer: consumer.into(),
        owner,
        uid,
        profile: profile.into(),
        actions: actions.to_vec(),
        expires: now() + seconds,
    };
    atomic(&path(s, &g.id)?, serde_json::to_vec_pretty(&g)?)?;
    Ok(g)
}
pub fn revoke(s: &Store, id: &str) -> Result<()> {
    fs::remove_file(path(s, id)?)?;
    Ok(())
}
pub fn check(s: &Store, id: &str, owner: &str, action: &str) -> Result<Grant> {
    let g: Grant =
        serde_json::from_slice(&fs::read(path(s, id)?).context("Grant is missing or revoked")?)?;
    ensure!(
        g.version == 1
            && g.id == id
            && g.owner == owner
            && g.uid == unsafe { libc::geteuid() }
            && g.expires > now()
            && g.actions.iter().any(|a| a == action),
        "Grant does not authorize this connection/action or has expired"
    );
    ensure!(
        s.load()?.profile(&g.profile)?.approved,
        "Profile approval was revoked"
    );
    Ok(g)
}
pub fn summary(s: &Store, g: &Grant) -> Result<serde_json::Value> {
    let c = s.load()?;
    let p = c.profile(&g.profile)?;
    let jobs: Vec<_> = s
        .jobs()?
        .into_iter()
        .filter(|j| j.profile == p.id)
        .collect();
    let last = jobs
        .iter()
        .find(|j| j.operation == "backup" && j.state == JobState::Success);
    let active = jobs.iter().find(|j| j.finished.is_none());
    let latest = jobs.iter().find(|j| j.operation == "backup");
    let destination = c.destination(&p.destination)?;
    let available = backup_system::validate_destination(destination).is_ok();
    let timers = backup_system::schedules();
    let next = timers
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|t| t["unit"] == format!("nyx-backup-{}.timer", p.id))
        })
        .and_then(|t| t["next"].as_u64());
    Ok(
        serde_json::json!({"version":1,"profile":p.id,"name":p.name,"coverage":"selected_user_data_and_readable_metadata","health":if active.is_some(){"working"}else if !available{"destination_unavailable"}else if latest.is_some_and(|j|j.state!=JobState::Success){"attention"}else if last.is_some(){"protected"}else{"no_success"},"active":active.map(|j|serde_json::json!({"id":j.id,"operation":j.operation,"progress":j.progress.get("percent_done"),"state":j.state})),"last_success":last.and_then(|j|j.finished),"schedule":p.schedule,"next_schedule_us":next,"destination_available":available,"remote_connectivity":"not_probed","available_actions":g.actions,"expires":g.expires}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grants_bind_owner_action_expiry_and_approval() {
        let root = std::env::temp_dir().join(format!("nyx-grants-{}", id()));
        let s = Store {
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
        };
        let mut c = Config::default();
        c.destinations.push(Destination {
            id: "d".into(),
            name: "d".into(),
            kind: DestinationKind::Local {
                path: root.join("repo"),
            },
        });
        c.profiles.push(Profile {
            id: "p".into(),
            name: "p".into(),
            destination: "d".into(),
            sources: vec![root.join("source")],
            exclusions: vec![],
            approved: true,
            include_secrets: false,
            keep_last: 1,
            schedule: None,
        });
        s.save(&c).unwrap();
        let mut g = Grant {
            version: 1,
            id: "g".into(),
            consumer: "test.Consumer".into(),
            owner: ":1.5".into(),
            uid: unsafe { libc::geteuid() },
            profile: "p".into(),
            actions: vec!["status".into()],
            expires: now() + 60,
        };
        let save =
            |g: &Grant| atomic(&path(&s, "g").unwrap(), serde_json::to_vec(g).unwrap()).unwrap();
        save(&g);
        assert!(check(&s, "g", ":1.5", "status").is_ok());
        assert!(check(&s, "g", ":1.6", "status").is_err());
        assert!(check(&s, "g", ":1.5", "backup").is_err());
        g.expires = 0;
        save(&g);
        assert!(check(&s, "g", ":1.5", "status").is_err());
        g.expires = now() + 60;
        save(&g);
        c.profiles[0].approved = false;
        s.save(&c).unwrap();
        assert!(check(&s, "g", ":1.5", "status").is_err());
        revoke(&s, "g").unwrap();
        assert!(check(&s, "g", ":1.5", "status").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

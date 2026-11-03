//! Explicit private-GitHub synchronization of Backup settings, never user file contents.
use anyhow::{Context, Result, ensure};
use backup_core::*;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::{Command, Stdio},
};
const FILE: &str = ".nyx-backup/profiles.toml";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub id: String,
    pub repository: String,
    pub repository_id: u64,
    pub branch: String,
    pub previous_sha: Option<String>,
    pub sha256: String,
    pub configuration: String,
    pub state: String,
}
fn identifiers(repo: &str, branch: &str) -> Result<()> {
    let p: Vec<_> = repo.split('/').collect();
    ensure!(
        p.len() == 2
            && p.iter().all(|s| !s.is_empty()
                && s.len() <= 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                && !s.starts_with('.')),
        "Use an explicit GitHub owner/repository"
    );
    ensure!(
        !branch.is_empty()
            && branch.len() <= 200
            && branch
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b))
            && !branch.starts_with(['-', '.', '/'])
            && !branch.ends_with('/')
            && !branch.contains(".."),
        "Use a simple existing branch name"
    );
    Ok(())
}
fn api(
    method: &str,
    endpoint: &str,
    body: Option<serde_json::Value>,
    missing: bool,
) -> Result<Option<serde_json::Value>> {
    let mut c = Command::new("gh");
    for k in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_HOST",
    ] {
        c.env_remove(k);
    }
    c.env("GH_PROMPT_DISABLED", "1")
        .args([
            "api",
            "--hostname",
            "github.com",
            "--include",
            "--method",
            method,
            endpoint,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if body.is_some() {
        c.args(["--input", "-"]).stdin(Stdio::piped());
    } else {
        c.stdin(Stdio::null());
    }
    let mut child = c
        .spawn()
        .context("Install GitHub CLI and sign in separately using its credential store")?;
    if let Some(body) = body {
        child
            .stdin
            .take()
            .context("GitHub input unavailable")?
            .write_all(&serde_json::to_vec(&body)?)?;
    }
    let out = child.wait_with_output()?;
    ensure!(
        out.stdout.len() <= 2 * 1024 * 1024,
        "GitHub response is too large"
    );
    let text = String::from_utf8(out.stdout)?.replace("\r\n", "\n");
    let (headers, data) = text
        .split_once("\n\n")
        .context("Invalid GitHub API response")?;
    let status = headers
        .lines()
        .next()
        .and_then(|s| s.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .context("Missing GitHub HTTP status")?;
    if status == 404 && missing {
        return Ok(None);
    }
    ensure!(
        out.status.success() && (200..300).contains(&status),
        "GitHub operation refused (HTTP {status}); check authentication, repository permissions and concurrent changes"
    );
    Ok(Some(serde_json::from_str(data)?))
}
fn repository(repo: &str) -> Result<serde_json::Value> {
    let v = api("GET", &format!("repos/{repo}"), None, false)?.context("Repository missing")?;
    ensure!(
        v["private"] == true && v["archived"] == false,
        "Choose an existing private, non-archived repository"
    );
    Ok(v)
}
fn current(repo: &str, branch: &str) -> Result<Option<serde_json::Value>> {
    api(
        "GET",
        &format!("repos/{repo}/contents/{FILE}?ref={branch}"),
        None,
        true,
    )
}
fn path(store: &Store, id: &str) -> Result<std::path::PathBuf> {
    valid_id(id)?;
    Ok(store.state.join("github-sync").join(format!("{id}.json")))
}
fn safe_configuration(mut config: Config) -> Result<String> {
    for p in &mut config.profiles {
        p.approved = false;
        p.include_secrets = false;
    }
    config.notifications = false;
    config.validate()?;
    Ok(toml::to_string_pretty(&config)?)
}
pub fn prepare(
    store: &Store,
    repo: &str,
    branch: &str,
    profiles: &[String],
) -> Result<serde_json::Value> {
    identifiers(repo, branch)?;
    ensure!(
        !profiles.is_empty(),
        "Select at least one profile to synchronize"
    );
    let c = store.load()?;
    let mut selected = Config::default();
    for id in profiles {
        selected.profiles.push(c.profile(id)?.clone());
    }
    selected.profiles.sort_by(|a, b| a.id.cmp(&b.id));
    selected.profiles.dedup_by(|a, b| a.id == b.id);
    for d in &c.destinations {
        if selected.profiles.iter().any(|p| p.destination == d.id) {
            selected.destinations.push(d.clone());
        }
    }
    let configuration = safe_configuration(selected)?;
    let r = repository(repo)?;
    ensure!(
        r["permissions"]["push"] == true,
        "Repository write permission is required"
    );
    api(
        "GET",
        &format!("repos/{repo}/branches/{branch}"),
        None,
        false,
    )?;
    let previous = current(repo, branch)?;
    let p = Plan {
        version: 1,
        id: id(),
        repository: repo.into(),
        repository_id: r["id"].as_u64().context("Missing repository identity")?,
        branch: branch.into(),
        previous_sha: previous
            .as_ref()
            .and_then(|v| v["sha"].as_str())
            .map(Into::into),
        sha256: format!("{:x}", Sha256::digest(configuration.as_bytes())),
        configuration,
        state: "reviewed".into(),
    };
    atomic(&path(store, &p.id)?, serde_json::to_vec_pretty(&p)?)?;
    Ok(
        serde_json::json!({"version":1,"plan":p.id,"repository":p.repository,"branch":p.branch,"path":FILE,"previous_sha":p.previous_sha,"sha256":p.sha256,"configuration":p.configuration,"warning":"This publishes profile names, source paths and destination metadata to the selected private repository. No passwords, tokens, grants, file contents or job history are included. Profile execution and recognized-secret inclusion are reset to false. Review this exact text before confirming."}),
    )
}
pub fn inspect(store: &Store, id: &str) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(load(store, id)?)?)
}
fn load(store: &Store, id: &str) -> Result<Plan> {
    let bytes = fs::read(path(store, id)?)?;
    ensure!(bytes.len() <= 1024 * 1024, "Sync plan is too large");
    let p: Plan = serde_json::from_slice(&bytes)?;
    identifiers(&p.repository, &p.branch)?;
    ensure!(
        p.version == 1
            && p.id == id
            && p.sha256 == format!("{:x}", Sha256::digest(p.configuration.as_bytes())),
        "Sync plan changed or is invalid"
    );
    let c: Config = toml::from_str(&p.configuration)?;
    ensure!(
        safe_configuration(c)? == p.configuration,
        "Plan contains unapproved configuration fields"
    );
    Ok(p)
}
pub fn push(store: &Store, id: &str, confirmation: &str) -> Result<serde_json::Value> {
    ensure!(
        id == confirmation,
        "Confirm the exact reviewed GitHub sync plan ID"
    );
    let mut p = load(store, id)?;
    ensure!(
        p.state == "reviewed",
        "Prepare a new sync plan after upload"
    );
    let r = repository(&p.repository)?;
    ensure!(
        r["id"].as_u64() == Some(p.repository_id) && r["permissions"]["push"] == true,
        "Repository identity or permissions changed"
    );
    let old = current(&p.repository, &p.branch)?;
    ensure!(
        old.as_ref().and_then(|v| v["sha"].as_str()) == p.previous_sha.as_deref(),
        "Remote settings changed after preview; prepare and review a new plan"
    );
    let mut body = serde_json::json!({"message":"Synchronize reviewed Nyx Backup settings","branch":p.branch,"content":base64::engine::general_purpose::STANDARD.encode(p.configuration.as_bytes())});
    if let Some(sha) = &p.previous_sha {
        body["sha"] = serde_json::Value::String(sha.clone());
    }
    let result = api(
        "PUT",
        &format!("repos/{}/contents/{FILE}", p.repository),
        Some(body),
        false,
    )?
    .context("Missing upload result")?;
    p.state = "uploaded".into();
    atomic(&path(store, id)?, serde_json::to_vec_pretty(&p)?)?;
    Ok(
        serde_json::json!({"version":1,"plan":id,"state":"uploaded","commit":result["commit"]["sha"]}),
    )
}
pub fn fetch(repo: &str, branch: &str, output: &Path) -> Result<serde_json::Value> {
    identifiers(repo, branch)?;
    ensure!(
        output.is_absolute(),
        "Choose an absolute new export filename"
    );
    repository(repo)?;
    let remote = current(repo, branch)?.context("No Backup settings on this branch")?;
    ensure!(
        remote["type"] == "file" && remote["size"].as_u64().is_some_and(|s| s <= 512 * 1024),
        "Remote settings must be a small regular file"
    );
    let encoded = remote["content"]
        .as_str()
        .context("Missing settings content")?
        .replace('\n', "");
    let data = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let c: Config = toml::from_str(std::str::from_utf8(&data)?)?;
    let clean = safe_configuration(c)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)?;
    f.write_all(clean.as_bytes())?;
    f.sync_all()?;
    Ok(
        serde_json::json!({"version":1,"exported":output,"message":"Fetched sanitized settings into a new private file. Review paths and destination identities before explicitly importing; nothing was applied or approved."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn github_ids_are_fixed_host_paths() {
        assert!(identifiers("owner/repository", "main").is_ok());
        for repo in [
            "https://evil/repo",
            "owner/repo?token=x",
            "../repo",
            "owner/repo/extra",
        ] {
            assert!(identifiers(repo, "main").is_err());
        }
        for branch in ["../main", "main?ref=secret", "--branch", "main\nother"] {
            assert!(identifiers("owner/repo", branch).is_err());
        }
    }
}

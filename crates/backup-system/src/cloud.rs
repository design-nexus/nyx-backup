//! S3-compatible providers. Credential values stay in Secret Service/encrypted Rclone config.
use super::*;
fn binding(d: &Destination) -> Result<String> {
    use sha2::{Digest, Sha256};
    d.validate()?;
    ensure!(
        matches!(d.kind, DestinationKind::S3 { .. }),
        "Select an S3 destination"
    );
    Ok(format!(
        "# Nyx Backup S3 target v1: {:x}",
        Sha256::digest(serde_json::to_vec(&d.kind)?)
    ))
}
pub fn validate_rclone_config(store: &Store, d: &Destination) -> Result<()> {
    if matches!(d.kind, DestinationKind::Smb { .. }) {
        return validate_smb_config(store, d);
    }
    let text = fs::read_to_string(rclone_config(store, &d.id))
        .context("Set up cloud credentials first")?;
    ensure!(
        text.lines().next() == Some(binding(d)?.as_str()),
        "Cloud endpoint/bucket/region/prefix changed; set up credentials again before connecting"
    );
    Ok(())
}
pub fn setup_s3(
    store: &Store,
    d: &Destination,
    access: &Secret,
    key: &Secret,
    token: Option<&Secret>,
) -> Result<()> {
    let DestinationKind::S3 {
        endpoint, region, ..
    } = &d.kind
    else {
        anyhow::bail!("Select an S3 destination")
    };
    d.validate()?;
    for s in [Some(access), Some(key), token].into_iter().flatten() {
        ensure!(
            !s.expose().contains(['\n', '\r', '\0']) && s.expose().trim() == s.expose(),
            "Cloud credentials must be single-line values without surrounding whitespace"
        );
    }
    let unlock = if rclone_config(store, &d.id).exists() {
        ensure_secret_unlocked(&d.id, "rclone")?;
        lookup_secret("nyx-backup", &d.id, "rclone")?
    } else {
        let u = generated_secret()?;
        store_secret("nyx-backup", &d.id, "rclone", &u)?;
        u
    };
    let provider = if endpoint.is_empty() { "AWS" } else { "Other" };
    let plain = format!(
        "[cloud]\ntype = s3\nprovider = {provider}\nenv_auth = false\naccess_key_id = {}\nsecret_access_key = {}\nsession_token = {}\nregion = {region}\nendpoint = {endpoint}\nforce_path_style = true\n",
        access.expose(),
        key.expose(),
        token.map(|s| s.expose()).unwrap_or("")
    );
    // Build ciphertext before replacing the old file. Existing unlock material is retained.
    store_secret("nyx-backup", &d.id, "s3-access", access)?;
    store_secret("nyx-backup", &d.id, "s3-secret", key)?;
    if let Some(t) = token {
        store_secret("nyx-backup", &d.id, "s3-session", t)?;
    }
    encrypt_rclone(store, d, &unlock, &binding(d)?, &plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_cannot_smuggle_credentials_or_disable_tls() {
        let make = |endpoint: &str, path: &str| Destination {
            id: "cloud".into(),
            name: "Cloud".into(),
            kind: DestinationKind::S3 {
                endpoint: endpoint.into(),
                region: "us-east-1".into(),
                bucket: "fixture-bucket".into(),
                path: path.into(),
            },
        };
        for endpoint in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com/?key=secret",
            "https://example.com/#fragment",
        ] {
            assert!(make(endpoint, "backup").validate().is_err(), "{endpoint}");
        }
        assert!(make("https://s3.example.com", "backup").validate().is_ok());
        assert!(make("http://127.0.0.1:9191", "backup").validate().is_ok());
        assert!(
            make("https://s3.example.com", "../backup")
                .validate()
                .is_err()
        );
    }
}

# Phases 5 and 6: cloud, settings sync and granted consumers

Version 0.6.0 adds S3-compatible destinations, explicit private-GitHub synchronization of Backup settings, and an app-owned Core Control adapter with a Control Center tile. All are user-invoked. No cloud account is discovered, connected, created or charged automatically; no watcher or automatic GitHub sync is installed.

## S3-compatible cloud repositories

Choose **Settings → New destination → S3-compatible cloud**. Enter a pre-existing bucket, region and repository prefix. Leave the endpoint blank for AWS, or supply an HTTPS S3-compatible endpoint for another provider. Use the cloud credential button to save an access key ID, secret access key and optional session token. Repository encryption passwords remain separate.

```sh
nyx-backupctl config import /reviewed/config.toml
nyx-backupctl s3-credentials DESTINATION
nyx-backupctl destination DESTINATION test
nyx-backupctl destination DESTINATION validate PROFILE
# Only for an explicitly new repository:
nyx-backupctl destination DESTINATION init PROFILE --confirm
```

TOML destination example (no credentials):

```toml
[[destinations]]
id = "cloud"
name = "Cloud repository"
type = "s3"
endpoint = "https://s3.example.com"
region = "us-east-1"
bucket = "existing-bucket"
path = "nyx/restic"
```

`--stdin` on `s3-credentials` accepts access ID, secret and, with `--session-token`, token as successive input lines. Use pipes; never command-line credential values. They are saved in Secret Service and the standard encrypted Rclone configuration. The config unlock key is in Secret Service. Endpoint, region, bucket and prefix are bound into its non-secret fingerprint; changing them requires credential setup again. Ambient AWS/Rclone authentication is not selected. Existing SMB ciphertext remains compatible.

HTTPS is required except literal loopback HTTP for disposable fixtures. TLS verification stays enabled. Connectivity lists the bucket without creating a prefix/repository. Validation never initializes one. Explicit initialization is separate and repository mutations use the same service and native Restic locks as local backups. Bucket listing permission and object read/write/delete permissions within the chosen repository prefix are needed. Provider policies, quotas, billing and session-token expiry are managed outside Backup. No bucket lifecycle or automatic retention rule is created. [Rclone S3 reference](https://rclone.org/s3/).

Actual S3 protocol, signature authentication, backup/verification/restoration and failure handling were qualified against a disposable signed loopback endpoint. Individual commercial providers and real cloud credentials have not been tested. No live account was contacted during qualification.

## GitHub Backup settings synchronization

This synchronizes **selected Backup profile settings**, not source-file contents or dotfiles. Only referenced destination metadata is included. Source paths, profile names and remote endpoint metadata are sensitive even without passwords: review the entire payload. Profile execution approval and recognized-secret inclusion are removed; notifications are disabled in exports. Credentials, keyrings, grants, history and user-file contents are excluded by construction.

Use **Settings → Cloud, GitHub sync and integration grants**, or:

```sh
nyx-backupctl github plan OWNER/PRIVATE_REPOSITORY --branch main --profile PROFILE
nyx-backupctl github inspect PLAN_ID
nyx-backupctl github push PLAN_ID --confirm PLAN_ID
nyx-backupctl github fetch OWNER/PRIVATE_REPOSITORY --branch main \
  --output /new/private-settings.toml
```

CLI `plan` can select multiple `--profile` values. The fixed remote file is `.nyx-backup/profiles.toml` on the specified existing branch. The repository must already exist, be private and not archived; upload requires write permission. Plan preparation records repository identity, exact payload hash and prior file SHA. Upload rechecks privacy, identity, permissions and remote SHA; concurrent edits require a new preview. GitHub's SHA precondition also protects the final update. No repository creation, deletion, branch creation, force push, arbitrary path or source build is exposed.

GitHub CLI handles authentication through its existing credential store. Backup never requests its token, logs in automatically, or passes a token in argv/environment. Inherited token/host overrides are removed; calls explicitly target github.com. `fetch` sanitizes remote execution/secret-inclusion settings and creates a new 0600 file without applying it. Review machine-specific paths and mount identities before explicit import; reapprove profiles and set credentials on the receiving machine yourself. [GitHub contents API](https://docs.github.com/en/rest/repos/contents).

The API workflow was tested with a local `gh` recording stub, including privacy, confirmation, inherited-token stripping, download permissions and concurrent-change refusal. Real GitHub account/network behavior is unverified. Qualification never uploaded actual user settings.

## Receiving-app integration grants

Backup owns `io.github.design_nexus.nyx.Control` on bus `io.github.design_nexus.nyx.Backup1`, object `/io/github/design_nexus/nyx/Backup1`. `Describe() → s` returns the suite-standard array of tools with `name`, `description` and `inputSchema`; only currently granted tools are listed, with caller-bound grant IDs in the allowed argument values. `Call(s name, s arguments) → s` uses the standard result envelope. Unlike the generic shared handler, this adapter retains the authenticated D-Bus sender and enforces grants at Backup itself.

A local user issues a grant for a running well-known consumer name, an approved profile, fixed actions and an expiry (60 seconds to 24 hours). The grant pins the current unique bus owner and UID. Calls recheck that owner, current well-known-name ownership, action, expiry and profile approval. Grant IDs are metadata, **not bearer tokens**: copying one to another connection does not authorize it. Restarted consumers need a new grant; restarting Backup does not authorize a different consumer. Revocation and unapproval take effect on the next call.

```sh
nyx-backupctl integrations grant io.github.design_nexus.nyx.ControlCenter PROFILE \
  --actions status,backup,verify,cancel --seconds 3600
nyx-backupctl integrations list
nyx-backupctl integrations revoke GRANT_ID
```

The app provides equivalent explicit controls. Only `status` is granted by default. Control calls return `{"ok": VALUE}` or `{"error": MESSAGE}` and accept strict JSON:

| Action | Arguments | Effect |
| --- | --- | --- |
| `backup.status` | `{"grant":"ID"}` | Profile-scoped health, progress, last success, configured schedule/actual next timer, destination availability and allowed actions |
| `backup.start` | `{"grant":"ID"}` | Starts that approved profile; returns its job ID |
| `backup.verify` | `{"grant":"ID"}` | Metadata verification for that profile |
| `backup.cancel` | `{"grant":"ID","job":"JOB_ID"}` | Cancels only that profile's active job |

No restore, disk recovery, credential retrieval, arbitrary command, initialization, destination mutation, retention deletion or GitHub upload is exposed. Ungranted legacy D-Bus `Start`/`Cancel` now reject requests; otherwise they would bypass the new grant checks. Updated local GUI/CLI job controls use the existing private UID-checked channel, with approved-profile enforcement for normal jobs. Same-user read/status APIs remain available for compatibility. This is not an application sandbox against malicious code already running as the user, which can read/edit private configuration or invoke the local CLI. No remote session-bus bridge is installed.

`data/nyx-backup-control.json` is the installed discovery descriptor. Core/Assistant consumers can use the standard interface directly while preserving sender identity; forwarding all calls through one unauthenticated shared identity does not satisfy the contract. No shared Core revision was changed.

## Control Center tile

The independent `nyx-control-center` repository now provides a **Backup** tile using its existing widget/provider framework. Add it through Control Center's widget picker; no layout is changed automatically. It reads Backup only while mapped, every five seconds, through the same GIO session connection as Control Center's application identity. It does not autostart Backup for background status reads. An absent service or missing grant yields an explanatory status. The Open Backup button is explicit user action.

Grant Control Center status access in Backup Settings, optionally enabling backup/cancel. The tile shows profile health, measured progress when available, last success and the actual next timer run (or that only a calendar is saved). Actions remain disabled without permission. It selects an available status grant; use a single intended profile grant for a predictable tile. Unmapping drops its subscription and polling timer. The headless provider probe verifies real contract/subscription behavior without launching a window. Rendered appearance and interactive GUI behavior remain unverified.

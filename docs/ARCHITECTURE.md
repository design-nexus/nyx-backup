# Architecture and integration contract

Rust 2024 workspace with independent domain crates:

- `backup-core`: version 1 TOML configuration, profile/destination models, preview/snapshot/job/health schemas, shared Nyx identity/XDG paths, private atomic JSON state.
- `backup-system`: Secret Service through public `nyx-sync` helpers; Rclone SMB configuration encryption; mount identity and NFS management; readable system metadata; explicit user schedules and notifications.
- `backup-engine`: a common selection walker for preview/execution, Restic argument arrays and JSON progress, cancellation, app worker lock, exit-state mapping and reconciliation.
- `backup-service`: versioned session D-Bus API, approved-profile execution, CLI administration and a systemd-owned daemon.
- `backup-restore`: profile-owned snapshot browsing, private staging/verification, exact plans, content diffs, descriptor-relative file replacement, conflict policies and rollback journals.
- `apps/backup`: GTK4 0.11, pinned Nyx Core/Design 2.0 widgets, theme engine and icon system. Blocking work stays off the GTK thread.

The restore crate is now delivered in Phase 2. A hardware crate remains deferred until its functionality works. No Settings engine, Core Control destructive API, `omarchy-backup` replacement, global watcher or setup integration is introduced.

Configuration: `$XDG_CONFIG_HOME/nyx-backup/config.toml`; encrypted per-destination SMB files in `rclone/`; GUI theme preferences in `appearance.json`. State: `$XDG_STATE_HOME/nyx-backup/jobs/ID.json`, metadata under `metadata/PROFILE/system.json`, and a process-held `worker.lock`. Cache: `$XDG_CACHE_HOME/nyx-backup/restic`. Parent app directories are 0700; ordinary app files are 0600. Atomic writes reuse `nyx-core`'s synced rename implementation.

One application lock serializes repository operations across CLI and daemon processes. Restic's native repository locking remains enabled, protecting against other tools and hosts. Repository operations publish running/progress/terminal state. Startup reconciles stale running jobs only after acquiring the application lock; it never clears repository locks. The systemd control group owns descendants, and cancellation terminates Restic's process group (including SSH/Rclone), escalating only when needed.

Metadata includes readable OS release, kernel, filesystem/mount descriptions, boot command line and Omarchy version. Missing commands/access are recorded as missing metadata coverage. This is an inventory, not recovery qualification. Selected files are stored at their native absolute paths inside snapshots; the metadata has a stable profile-specific path. No source root is silently replaced with the whole home directory.

## Session D-Bus v1

Name/interface `io.github.design_nexus.nyx.Backup1`; object `/io/github/design_nexus/nyx/Backup1`. Parameters and responses use strings; structured responses are JSON. A service-side bus-daemon UID lookup is performed for every method. The trust boundary is the logged-in user: another same-user process can request an already approved profile, and same-user code can edit the private configuration. This is not an application-identity sandbox.

| Method | Arguments | Response |
| --- | --- | --- |
| Capabilities | none | Version and supported operations |
| Status | none | Health contract |
| Destinations | none | IDs, names, bound destination availability; no credentials |
| Preview | profile ID | Selection size, sources, warnings, blocking conditions |
| Start | profile ID, operation | Job ID; approved profiles only |
| Jobs / Job | none / job ID | Shared persisted jobs |
| Cancel | job ID | Cancellation acknowledgement for the active worker |
| JobChanged (signal) | JSON job | Initial, progress and terminal changes |

`Start` permits only backup, history, stats, verify and verify-full. History returns profile-tagged snapshot records in the completed job result. No arbitrary command, credentials, destination mutation, initialization, deletion or restoration method is exposed on D-Bus.

Explicit local initialization, validation and retention also run in the same daemon, with the same job status, cancellation and repository admission lock. The GUI/CLI sends a small versioned request over a private Unix socket below `$XDG_RUNTIME_DIR/nyx-backup/`, keyed to the app's state directory. Its parent is 0700, its socket is 0600, and the daemon verifies kernel peer UID and the client's configuration path. Requests contain only a configured profile ID, a fixed operation and, for retention application, exact-profile confirmation. No secrets or arbitrary executable/path operations are accepted. Unsupported operations and mismatched configuration contexts are rejected. This local administration channel is distinct from the integration D-Bus API; same-user code already has equivalent access to the explicit local CLI and configuration files. It is not an application-identity sandbox or a remote administration interface.

All repository jobs survive the initiating client closing. File-source selection checks cancellation during traversal and active-store inspection, and cancellation before Restic starts records a cancelled result without running a repository command. Preparation and indeterminate operations appear as preparing/working states; Restic backup JSON supplies measured progress once available. Dispatch transfers the held admission lock into the worker rather than releasing/reacquiring it. A rejected busy local operation does not publish a phantom running job. Startup leaves jobs alone when a live process owns the application lock; stale jobs are reconciled once it is free.

The future Control Center status contract is `Health` v1: coverage, active job/progress, last successful backup, next schedule information and available actions. Timer status is sourced from `systemctl --user list-timers --all --output=json`. The Control Center widget is deferred. Destination connection testing is separate from bound-mount status; status does not silently connect a remote or initialize a repository.

## Local restoration protocol

The private local request accepts a typed `restore` payload. Read-only actions (snapshot history/browse, plan/receipt inspection, diff and rollback preview) return bounded results directly; plaintext diffs are never inserted into a job journal. Plan preparation, apply and rollback are admitted as background jobs with the same held operation lock, cancellation and job-change signals. Requests are tied to the same configuration context and UID as other local administration. Apply and rollback require exact plan-ID confirmation. D-Bus `Start` retains its original allowlist and rejects every restore operation. `Capabilities.restore` still describes integration exposure (`false`); `local_restore` and `local_restore_operations` describe explicit local functionality.

Plans pin an exact owned snapshot, selected archive paths, target mapping, conflict policy and fingerprints. Apply uses staged data and current target checks. Intent and retained-original locations are synced before replacement; no-replace renames protect concurrent target creation. Rollback consults both the journal and filesystem, retaining changed files and recovery copies on conflict. See RESTORE.md for metadata, concurrency, storage and filesystem limits.

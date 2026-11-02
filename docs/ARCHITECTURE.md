# Architecture and integration contract

Rust 2024 workspace with independent domain crates:

- `backup-core`: version 1 TOML configuration, profile/destination models, preview/snapshot/job/health schemas, shared Nyx identity/XDG paths, private atomic JSON state.
- `backup-system`: Secret Service through public `nyx-sync` helpers; Rclone SMB configuration encryption; mount identity and NFS management; readable system metadata; explicit user schedules and notifications.
- `backup-engine`: a common selection walker for preview/execution, Restic argument arrays and JSON progress, cancellation, app worker lock, exit-state mapping and reconciliation.
- `backup-service`: versioned session D-Bus API, approved-profile execution, CLI administration and a systemd-owned daemon.
- `apps/backup`: GTK4 0.11, pinned Nyx Core/Design 2.0 widgets, theme engine and icon system. Blocking work stays off the GTK thread.

Restore/hardware crates do not exist until those capabilities work. No Settings engine, Core Control destructive API, `omarchy-backup` replacement, global watcher or setup integration is introduced.

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

`Start` permits only backup, history, stats, verify and verify-full. History returns profile-tagged snapshot records in the completed job result. No arbitrary command, credentials, destination mutation, initialization, deletion or restoration method is exposed. CLI/GUI-local administration performs explicit initialization, credential setup and confirmed retention; these still take the same worker lock. History, verification and statistics run in the daemon. GUI initialization/retention run on a background thread; keep the GUI open for these administrative actions. Backups continue independently when the GUI closes.

The future Control Center status contract is `Health` v1: coverage, active job/progress, last successful backup, next schedule information and available actions. Timer status is sourced from `systemctl --user list-timers --all --output=json`. The Control Center widget is deferred. Destination connection testing is separate from bound-mount status; status does not silently connect a remote or initialize a repository.

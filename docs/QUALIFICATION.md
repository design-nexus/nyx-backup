# Phase 1 qualification

This records actual evidence separately from implemented source. No real user sources or NAS credentials are used in qualification.

- Headless domain/service and GTK workspace compile against pinned shared dependencies.
- Focused domain checks cover terminal states (including Restic exit 3), source/exclusion behavior, retained build folders, source-inside-repository refusal, app-operation serialization, stale-job reconciliation, missing bound mounts and TOML round-trip.
- `tools/smoke.py` uses a private D-Bus session and disposable Secret Service keyring. Its recorded result covers actual Restic operations and native restoration.
- GUI opening/interaction and rendered visual review are not performed. Installation must not open or restart the app.
- SFTP is qualified against a disposable loopback SFTP-only OpenSSH endpoint. SMB and NFS endpoints remain unverified. Native Rclone encrypted-config interoperability is tested independently from connectivity.
- Physical drive removal, service crashes during filesystem sync, host reboot/power loss and large-data performance remain unverified.
- Complete-system, hardware migration and boot recovery are planned, with no qualifying VM recovery evidence.

The results below record the completed checks and their limits.

## Recorded results (2026-11-02)

- `cargo test --locked -p backup-engine`: **6 passed**.
- `cargo build --release --locked --workspace`: **passed**. GTK Dialog APIs produce deprecation warnings; the current GTK4 APIs compile. Shared upstream `proc-macro-error2` has a future-compatibility warning.
- `cargo tree -p backup-service --edges normal`: no GTK dependency in the headless service graph.
- `dbus-run-session -- python3 tools/smoke.py`: **passed** with an isolated Secret Service and disposable repository. Proved initial/incremental snapshots, matching native restore contents and mode 0640, cache/key/repository exclusions, retained build files, metadata/full-data verification, snapshot history/statistics, scoped retention preview/deletion, lock refusal, actual Restic partial-read status, cancellation, API operation/approval restrictions, interrupted-job reconciliation, locked credentials and absence of secret values in TOML/job state. Native Rclone decoded the encrypted SMB configuration. No SMB connection was attempted.
- `python3 tools/service-smoke.py`: **passed**. Private-bus D-Bus activation, generated service/timer syntax, transient systemd user timer → CLI → approved profile → Restic, and persisted success after the scheduling client exits. Persistent enable/disable was deliberately not exercised in the real user's configuration. Runtime fixture timer/service units were stopped afterward.
- Only fixture sources under temporary directories were selected. Fixtures remain local for inspection; none are shipped or configured as real profiles.

No full-system, cross-PC or boot recovery claim is accepted. SMB/NFS endpoint behavior and GUI visual/interaction behavior remain unverified.

Installation verification: all three installed binaries match the release SHA-256 values; the installed D-Bus activation returned Health v1 from the systemd-owned service. The service is on demand (not enabled at login); no Backup timers are installed/enabled in the live manager. The launcher validates and the canonical icon SVG parses. The GUI was not opened or restarted.

## SFTP-only qualification (2026-11-02)

`python3 tools/sftp-smoke.py`: **passed** against an unprivileged loopback OpenSSH server restricted to `internal-sftp`, with ephemeral client/host keys, strict host-key checking, private SSH configuration and a private Secret Service/D-Bus session. Confirmed the old remote-shell probe fails for this account; the corrected SFTP subsystem probe succeeds without creating a repository. Explicit initialization, encrypted backup, full-data verification, snapshot history and native Restic restoration succeeded, with matching contents and file mode 0640. Missing repository validation and disconnected transport checks failed without initializing a replacement. No real SSH configuration, data or NAS credentials were used.

The locked workspace release build, formatting and diff checks passed after this change. Updated binaries, launcher and icon were installed without opening or restarting the GUI.

## Path, mount and credential safety (2026-11-02)

- Focused domain checks: **9 passed**, including source paths containing `..`, exclusions expressed through the selected source spelling, open database detection through normalized paths, selected and dangling symlink roots, destination parent-path rejection, and a real nested-mount refusal using existing `/` and `/proc` mount metadata. The mount test is read-only and does not create mounts.
- Locked workspace release build, formatting and diff checks: **passed**.
- Updated disposable integration fixture: **passed**. Actual Restic backup/restoration preserves repository/cache/key exclusions when the selected source contains `..`. An injected Rclone failure during SMB credential update leaves the original ciphertext and unlock key unchanged, and native Rclone still decodes that original configuration. This qualifies credential-update failure handling, not an SMB endpoint.
- Locking the isolated Secret Service collection causes repository validation to fail with an explicit locked-credential error in under five seconds, without waiting for an unlock prompt.
- Installation includes all three binaries, launcher and icon. Only the idle background service is restarted to load these safety changes; the GUI is not opened or restarted.

SMB/NFS endpoint qualification, physical unplug races and GUI interaction/visual review remain unverified.

## Phase 1 completion: job ownership and administration (2026-11-02)

The remaining client-lifetime limitation has been removed: initialization, repository validation, retention preview and explicitly confirmed retention application now execute in the daemon. Local GUI/CLI requests use a private Unix socket with kernel peer-UID checks, strict operation allowlisting, exact-profile retention confirmation, matching configuration context, bounded messages and timeouts. Integration D-Bus methods still expose no initialization, deletion, credentials, destination mutation or restoration.

- Focused domain checks: **11 passed**, including deterministic cancellation during source traversal, cancellation before credential lookup/Restic startup, admission-lock ownership through terminal reporting, no phantom jobs on busy rejection and preservation of a live worker during reconciliation.
- `python3 tools/smoke.py`: **passed**. Includes ordinary initial/incremental backup, full verification, native restoration and permission checks, source safety, concurrency, cancellation, interrupted-job reconciliation, locked credentials, encrypted Rclone interoperability, failed credential-update preservation, server/account binding before any SMB connection, private socket permissions, denied local operations/confirmation and administration continuing after client disconnect.
- `python3 tools/service-smoke.py`: **passed** for activation, generated unit syntax and transient timer → CLI → approved profile → Restic.
- `python3 tools/sftp-smoke.py`: **passed** for SFTP-only connectivity, service-owned initialization, encrypted backup, full verification, history, native restore and absent/disconnected destination refusal.
- Locked release build, formatting and whitespace checks: **passed**. The GUI displays preparation and indeterminate job states without inventing a percentage.

Phase 1 implementation is complete. Qualification limits remain explicit: SMB/NFS endpoints, physical unplug/reboot/power-loss behavior and GUI interaction/visual review are unverified. Persistent schedule enable/disable was not exercised in the live user configuration. Complete-system, cross-PC and boot recovery remain later phases with no qualifying VM evidence. No real user sources or NAS credentials were used.

# Phase 1 qualification

This records actual evidence separately from implemented source. No real user sources or NAS credentials are used in qualification.

- Headless domain/service and GTK workspace compile against pinned shared dependencies.
- Focused domain checks cover terminal states (including Restic exit 3), source/exclusion behavior, retained build folders, source-inside-repository refusal, app-operation serialization, stale-job reconciliation, missing bound mounts and TOML round-trip.
- `tools/smoke.py` uses a private D-Bus session and disposable Secret Service keyring. Its recorded result covers actual Restic operations and native restoration.
- GUI opening/interaction and rendered visual review are not performed. Installation must not open or restart the app.
- No disposable SMB, SFTP or NFS endpoint was available for runtime transport qualification. These transports are implemented, unverified. Native Rclone encrypted-config interoperability is tested independently from connectivity.
- Physical drive removal, service crashes during filesystem sync, host reboot/power loss and large-data performance remain unverified.
- Complete-system, hardware migration and boot recovery are planned, with no qualifying VM recovery evidence.

Final command results and fixture evidence are added after validation.

## Recorded results (2026-11-02)

- `cargo test --locked -p backup-engine`: **6 passed**.
- `cargo build --release --locked --workspace`: **passed**. GTK Dialog APIs produce deprecation warnings; the current GTK4 APIs compile. Shared upstream `proc-macro-error2` has a future-compatibility warning.
- `cargo tree -p backup-service --edges normal`: no GTK dependency in the headless service graph.
- `dbus-run-session -- python3 tools/smoke.py`: **passed** with an isolated Secret Service and disposable repository. Proved initial/incremental snapshots, matching native restore contents and mode 0640, cache/key/repository exclusions, retained build files, metadata/full-data verification, snapshot history/statistics, scoped retention preview/deletion, lock refusal, actual Restic partial-read status, cancellation, API operation/approval restrictions, interrupted-job reconciliation, locked credentials and absence of secret values in TOML/job state. Native Rclone decoded the encrypted SMB configuration. No SMB connection was attempted.
- `python3 tools/service-smoke.py`: **passed**. Private-bus D-Bus activation, generated service/timer syntax, transient systemd user timer → CLI → approved profile → Restic, and persisted success after the scheduling client exits. Persistent enable/disable was deliberately not exercised in the real user's configuration. Runtime fixture timer/service units were stopped afterward.
- Only fixture sources under temporary directories were selected. Fixtures remain local for inspection; none are shipped or configured as real profiles.

No full-system, cross-PC or boot recovery claim is accepted. NAS endpoint behavior and GUI visual/interaction behavior remain unverified.

Installation verification: all three installed binaries match the release SHA-256 values; the installed D-Bus activation returned Health v1 from the systemd-owned service. The service is on demand (not enabled at login); no Backup timers are installed/enabled in the live manager. The launcher validates and the canonical icon SVG parses. The GUI was not opened or restarted.

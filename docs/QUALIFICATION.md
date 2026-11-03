# Phase 1 qualification

This records actual evidence separately from implemented source. No real user sources or NAS credentials are used in qualification.

- Headless domain/service and GTK workspace compile against pinned shared dependencies.
- Focused domain checks cover terminal states (including Restic exit 3), source/exclusion behavior, retained build folders, source-inside-repository refusal, app-operation serialization, stale-job reconciliation, missing bound mounts and TOML round-trip.
- `tools/smoke.py` uses a private D-Bus session and disposable Secret Service keyring. Its recorded result covers actual Restic operations and native restoration.
- GUI opening/interaction and rendered visual review are not performed. Installation must not open or restart the app.
- SFTP is qualified against a disposable loopback SFTP-only OpenSSH endpoint. SMB and NFS endpoints remain unverified. Native Rclone encrypted-config interoperability is tested independently from connectivity.
- Physical drive removal, service crashes during filesystem sync, host reboot/power loss and large-data performance remain unverified.
- Complete-system and boot recovery remain planned. Phase 3 implements user migration planning; physical cross-machine recovery has no qualifying evidence.

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

## Phase 2 complete: selective restoration (2026-11-02)

Delivered as version 0.2.0: snapshot browsing, selected file/folder restoration, staged review, Quick Dotfiles Restore, text diffs, skip/keep-both/replace policies, restore receipts and explicit rollback. These run through the private local service channel; the integration D-Bus API still rejects restoration.

- Focused checks: **7 restore tests plus 11 existing engine tests passed**. Restore tests cover original preservation, rollback, post-preview edits, rollback conflicts, an interrupted original-file move recovered from its journal, symlink ancestors, skip defaults, cancellation and invalid path/confirmation requests.
- `python3 tools/restore-smoke.py`: **passed** against native Restic with a private session bus, keyring, home and disposable repository. Proved snapshot browsing; preview without target writes; selected restore contents and mode 0640; literal glob filenames; symlinks and empty folders; skip/keep-both/replace behavior; changed-after-preview refusal; actual Quick Dotfiles Restore/diff/rollback; preservation of post-restore edits and originals; explicit credential authorization; traversal/symlink/confirmation guards; deterministic stalled-child process-group cancellation; receipts; profile ownership for snapshots/receipts; filesystem-root refusal; no restored text or secrets in ordinary job journals; and D-Bus refusal of restore operations.
- `python3 tools/sftp-smoke.py`: **passed**, extended to app-driven selective restore and rollback through the disposable SFTP-only endpoint, in addition to native recovery and Phase 1 transport checks.
- Locked workspace release build, formatting, fixture syntax and whitespace checks: **passed**. Headless restoration is a separate domain crate. Existing GTK Dialog deprecation and upstream future-compatibility warnings remain.

Coverage is explicit: contents, symlinks, ordinary mode bits and modification times. Ownership, ACLs, xattrs and hard-link topology are not applied. GUI visual/interaction review, SMB/NFS restore endpoints, physical failure modes and host power-loss recovery remain unverified. The GUI is not opened or restarted during delivery. No real user file sources or live NAS credentials were used.


## Phase 3: user migration and Setup (2026-11-02)

Delivered as version 0.3.0. New encrypted snapshots include versioned package/hardware inventories. The app and private CLI/service prepare reviewed user-root mapping, hardware/file holds, bounded reference rewriting, target monitor defaults, package decisions and private Setup exports. The installed `nyx-backup-setup` consumer supports preview and separately confirmed package/file application. The legacy generated setup repository and `omarchy-backup` are unchanged.

- Focused checks: **25 passed** (11 engine, 10 restore, 4 migration). Cover existing restore guards plus target-root mapping, hardware-file holds, Lua/legacy monitor preparation, literal reference boundaries, package hardware/foreign/dependency/platform/repository decisions, invalid package arguments and handoff digest changes.
- `python3 tools/migration-smoke.py`: **passed** with native Restic, isolated home/config/state, private session/keyring and synthetic old-home/hardware inventory. Proved inventory in encrypted snapshot metadata; legitimate empty foreign-package queries; marked external-manifest provenance; staging without target changes; different-home mapping; automatic target Lua monitor syntax (`luac -p`); preserved embedded URLs; held hardware/Backup files; correct package decisions; private export; Setup prepare/inspect; confirmation refusal; exact fixed installer arguments; forged/damaged bundle refusal; confirmed file apply and rollback; integration D-Bus rejection. **Installer execution used a recording sudo stub; no host package installation or repository refresh occurred.**
- `python3 tools/restore-smoke.py`: **passed** as a targeted regression for shared restore planning/application, including existing confirmation, cancellation, conflict and rollback guards.
- Exported fixture JSON matches the published schema's field structure, types and enum values. Runtime additionally verifies digest, target binding and actual current package eligibility; the digest is not a signature.
- Locked workspace release build and formatting/whitespace checks passed. GTK deprecation and upstream future-compatibility warnings remain. GUI was not launched or visually reviewed.

Read-only host inventory was inspected, but no real user sources or live NAS credentials were selected. This proves synthetic migration mechanics, not physical different-machine compatibility, display behavior, real package installation, full-system or boot recovery. Flatpaks, foreign builds and recognized hardware packages require manual review; local binaries/language-manager inventory is missing coverage. Power-loss, SMB/NFS endpoints and GUI interaction remain unverified.

Installation: all three Rust binaries match the release artifacts; the Setup consumer and contract match their sources. Launcher/icon and activation resources are installed. The idle background service was restarted and returned `local_migration: true` and `setup_contract: nyx-backup-setup/v1`. No schedule was enabled and the GUI was not opened or restarted.


## Phase 4: offline disk recovery (2026-11-02)

Delivered as version 0.4.0: independent offline root-run helper, standard encrypted Restic disk captures, verified target-bound overwrite plans/receipts, local boot-kit builder and Restore-page kit creation. No privileged desktop service or destructive integration method is installed.

- `cargo test --locked -p backup-recovery`: **3 passed**, covering exact IDs/clean paths, short-read/hash behavior and ordinary-file refusal.
- `python3 tools/recovery-smoke.py`: **passed** with KVM/QEMU, installed kernel 7.2.11-arch1-1, OVMF UEFI firmware and disposable regular disk files. The first guest creates a 384 MiB GPT/ESP/ext4 fixture with boot files and a minimal root system. It refuses capture while mounted, marks the source disk read-only, captures into an encrypted Restic repository, rejects wrong credentials, prepares an exact-image target plan, refuses wrong confirmation and post-preview target changes, restores and syncs a separate target and compares whole-disk SHA-256. Reapplying the completed plan is refused.
- A second VM attaches **only the recovered target** and UEFI firmware, without `-kernel`/`-initrd` shortcuts. It boots the restored EFI image, mounts the recovered ext4 root, verifies file content, mode 0640 and a symlink, and uses `switch_root` into the recovered `/sbin/init`, which emits the explicit boot-success marker and powers off.
- Locked release workspace build passed, including GTK. Existing deprecated GTK APIs and upstream future-compatibility warnings remain. Recovery mutation from the ordinary desktop was refused before opening the supplied device path.
- All destructive operations ran inside guests; no host block devices, real user sources or live NAS credentials were selected or attached. The fixture source, target and logs are under ignored `target/nyx-recovery-vm-*` directories.

This demonstrates exact disk recovery and boot of **one synthetic x86-64 UEFI GPT/ext4 system**, not complete Omarchy, physical-machine or cross-PC recovery. Secure Boot, TPM/encrypted-root boot, RAID/LVM, alternate disk geometry, physical storage drivers, remote recovery transports and real power-loss behavior remain unqualified. Boot kit images are unsigned. No rendered GUI/interaction review was run. The desktop still reports `full_system_recovery: false`.


Phase 4 delivery checks: all 21 existing engine/restore domain tests passed alongside the 3 new recovery tests. Formatting and whitespace checks passed. All four installed Rust executables match release artifacts; launcher/icon, Setup consumer, kit builder, recovery documentation and activation resources are installed. The installed builder created a ready kit at `~/.local/share/nyx-backup/recovery-kits/0.4.0` using the VM-tested kernel; every artifact matches its checksum manifest. The idle background service was restarted and reports the offline tool/kit capabilities while retaining `full_system_recovery: false`. No schedule was enabled; the GUI was not opened or restarted.


## Phases 5 and 6: ecosystem and local qualification (2026-11-02)

Delivered together as 0.6.0: S3-compatible destinations, private-GitHub Backup settings sync, expiring caller-bound integration grants, receiving-app Core Control enforcement, selectable Control Center tile, focused security/failure/performance qualification and extended VM recovery. No live account or real user source was selected.

- **31 focused Backup tests passed**: 11 engine, 10 restore, 4 migration, 3 offline recovery, 2 service (grant binding and GitHub input restrictions) and 1 system (S3 endpoint/path restrictions).
- `python3 tools/smoke.py`: **passed** after moving local job controls to the private channel and closing the legacy ungranted D-Bus route. Includes initial/incremental/native restoration, exclusions, partial-read result, cancellation/concurrency, stale-job reconciliation, locked credentials, encrypted Rclone interoperability, failed SMB credential update preservation and service-owned local administration.
- `python3 tools/ecosystem-smoke.py`: **passed** with private home/config/state/keyring/session bus, a signed loopback S3 server using an stdin/stdout auth proxy, and a local GitHub CLI/API stub. Proved S3 connectivity without initialization, absent-repository refusal, encrypted initial/incremental backup, metadata/full verification, selective restore contents/mode 0640, endpoint binding, wrong key and disconnected endpoint refusal. No credential values appear in persistent app configuration, journals, grants or sync payloads.
- The same fixture proves grant connection/action/profile/expiry/revocation/current-name enforcement; unknown actions/arguments and ungranted legacy mutation refusal; actual cancellation through the granted adapter; GitHub plan/upload/fetch, exact confirmation, approval stripping, private-repository requirement, remote-SHA race refusal, inherited-token stripping and new-file mode 0600. GitHub operations use a local stub only.
- The actual `nyx-providers` `backup_probe` example is run headlessly inside that private bus: a registered GIO consumer gets its own status-only grant, reads the versioned contract and verifies the Live provider stops after its subscription is dropped. No Control Center/Backup window was opened. Control Center workspace tests: **39 passed** (including status parsing); release build and Clippy passed with existing style suggestions only.
- `python3 tools/recovery-smoke.py --extended`: **passed**. In addition to capture/restore/real UEFI boot of the disposable GPT/ext4 fixture, a mismatched-size disk and altered staging are refused. A throttled virtual target permits stopping/killing the worker after the durable `applying` journal is observed; reuse of that interrupted plan is refused. Fresh planning and retry restore matching bytes and the recovered disk boots with the source absent. This is a worker-interruption test, not electrical power-loss evidence.
- Locked Backup workspace release build, focused headless Clippy, formatting and whitespace checks passed. Existing GTK deprecation, Clippy style and upstream future-compatibility warnings remain. The OSV/RustSec review of 325 locked crates.io packages returned five informational unmaintained-package advisories, recorded in SECURITY-REVIEW.md; none are silently suppressed.

Small-data loopback baseline: **32 MiB random payload plus 200 small files**. Initial backup 1.18 s, incremental backup 1.208 s, metadata verification 0.861 s. Restic reported 33,815,996 bytes added initially versus 261,383 incrementally (including captured metadata). These are one local fixture's wall times, not repeatable throughput promises or large-NAS measurements.

Remaining qualification limits: real commercial cloud/GitHub providers, live SMB/NFS endpoints, rendered GUI/interaction, large datasets, physical unplug and true power loss, physical Omarchy/different-machine recovery, Secure Boot/TPM/encrypted-root, RAID/LVM and alternate geometry. Existing same-user and filesystem race boundaries still apply. `full_system_recovery` stays false.


Phases 5–6 installation: Backup 0.6.0's four Rust binaries, launcher/icon, local consumers, installed control descriptor and boot-kit resources match their release/source artifacts. A ready refreshed kit at `~/.local/share/nyx-backup/recovery-kits/0.6.0` matches its checksum manifest. Control Center's updated binary matches its release artifact. Only the idle Backup background service was restarted; it reports S3/GitHub/granted-control support and retains `full_system_recovery: false`. Neither GUI was opened/restarted, no panel layout was edited and no schedule/account was enabled.


## Completion update: 0.6.1 (2026-11-03)

- Migrated Backup to zbus 6.2.1 and pinned published Core revision `1c309f2912d5eff5e1a3f5eeca8b91cc3c5bcf76`. Backup selects credentials-only nyx-sync; default shared sync behavior is unchanged (20 default-feature and 4 credentials-only shared tests passed).
- OSV query: **183 locked crates.io packages, zero findings**. The five initial maintenance advisories are absent from Backup's graph without suppression.
- **24 focused Backup checks passed** (11 engine, 10 restore, 2 service, 1 system). The existing signed-S3/grant/GitHub-stub/provider fixture passed with the new D-Bus runtime. No real account was accessed.
- `tools/smb-smoke.py`: **passed** against Samba 4.25.0 with private namespace/account state, authenticated connectivity, absent-repository refusal, explicit initialization, initial/incremental encrypted backups, full verification, selective contents/mode restoration, wrong-password/disconnection refusal, and no credential values in ordinary app state. Test-only port injection leaves the production transport unchanged.
- `tools/ui-smoke.py`: **passed** for dark and light themes. Eight page renders per theme (1600/800 pixels), all six transport field sets, invalid-save refusal, destination/profile Cancel, ecosystem/recovery Escape, Ctrl+Shift+P and Ctrl+Q. Renders were visually inspected; physical input is not injected. The opt-in `ui-review` feature is excluded from installed builds.
- Corrected stale recovery copy, excessive destination form clutter, missing advertised shared shortcuts and enabled idle cancellation. Release workspace build and formatting/whitespace checks passed.

This closes the local maintenance, targeted GUI and SMB protocol gaps. Real commercial provider/account behavior, live NFS, physical failures and production full-system/cross-PC recovery are not asserted. Ordinary profiles remain selected-data backups; the qualified disk recovery path remains the disposable UEFI VM fixture documented above.

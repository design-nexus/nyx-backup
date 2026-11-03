# Security and qualification review — updated 2026-11-03

This is a source/boundary review plus focused runtime qualification, not an independent penetration test or security certification. Scope: Backup 0.6.1, its selected-data engine/restore/migration/offline helpers, new S3/GitHub/grant boundaries, and the Control Center provider. Installed external tools and operating-system services retain their own trust boundaries.

## Changes resulting from the review

- Closed the legacy session D-Bus start/cancel bypass when adding sender-bound grants. Explicit local GUI/CLI mutations moved to the private peer-UID/configuration-bound channel. Integration changes now require the receiving application's grant check, current name ownership and current profile approval. Unknown actions/arguments and jobs from another profile are refused.
- S3 configuration binds endpoint, bucket, region and prefix before using credentials. Endpoint credentials/query injection and non-loopback plaintext HTTP are rejected. Secrets stay in Secret Service/encrypted Rclone config; inherited backend overrides are stripped. Connectivity probes use bounded Rclone retry counts and never initialize missing repositories.
- GitHub settings export is typed, profile-selected, private-repository-only and resets execution/secret-inclusion authorization. The fixed contents API route cannot become a workflow/script or arbitrary repository path. Remote SHA and repository identity/privacy are rechecked. Inherited GitHub token/host overrides are stripped; no user files are uploaded by qualification.
- Reviewed cancellation, exclusive admission, incomplete outcomes, read-only source/root-environment checks, no-follow staged restoration, retained rollback originals, exact disk target identity/fingerprints, durable applying-state handling, and credential/job-journal separation. Existing guards are preserved rather than relaxed for integrations.

## Dependency advisory review

The OSV batch API was queried for all **325 crates.io packages** in the locked Backup dependency graph, including build dependencies. It returned five informational unmaintained-package advisories, not vulnerability findings in that result:

| Package | Locked version | Advisory / disposition |
| --- | --- | --- |
| basic-toml | 0.1.10 | [RUSTSEC-2026-0374](https://rustsec.org/advisories/RUSTSEC-2026-0374.html); transitive age/i18n configuration build path through pinned shared nyx-sync |
| proc-macro-error2 | 2.0.1 | [RUSTSEC-2026-0173](https://rustsec.org/advisories/RUSTSEC-2026-0173.html); transitive age/i18n macro build path; existing compiler future-compatibility warning |
| zbus_names | 4.3.5 | [RUSTSEC-2026-0371](https://rustsec.org/advisories/RUSTSEC-2026-0371.html); standalone types moved into zbus 6 |
| zvariant | 5.15.1 | [RUSTSEC-2026-0370](https://rustsec.org/advisories/RUSTSEC-2026-0370.html); standalone wire crate moved into zbus 6 |
| zvariant_derive | 5.15.1 | [RUSTSEC-2026-0372](https://rustsec.org/advisories/RUSTSEC-2026-0372.html); macros moved into zbus 6 |

Resolved for Backup in 0.6.1: zbus 6.2.1 replaces the standalone unmaintained D-Bus crates. A published credentials-only nyx-sync feature removes the unused age/HTTP synchronization stack from Backup while retaining existing shared consumers’ default behavior. The current locked graph contains **183 crates.io packages; the repeated OSV query returned no findings**. No advisory is suppressed. The table above records the initial findings, not remaining dependencies. This check does not audit Git sources, external Restic/Rclone/GitHub CLI binaries, distro packages, GTK C libraries, the kernel, firmware or unknown vulnerabilities.

## Runtime evidence and remaining boundaries

QUALIFICATION.md records exact commands/results. New qualification uses signed loopback S3, a private D-Bus/keyring/configuration, a GitHub API stub and disposable QEMU disks. It exercises owner/action/expiry/profile/revocation/name changes, ungranted legacy calls, credential/endpoint failure, private export and concurrent-update refusal, native encrypted backup/restore, cancellation, image tampering, geometry mismatch and interrupted disk overwrite/replanning/boot.

A local 32 MiB random-data + 200-small-file fixture records initial/incremental timings and actual Restic data added. It is a reproducible small-data baseline, not a throughput guarantee, large-NAS benchmark or capacity recommendation. The provider's subscription stop behavior is qualified headlessly; hidden panels have no repeating Backup poll timer.

No real cloud/GitHub account, real user sources or live NAS credentials are used. Real commercial providers, live SMB/NFS endpoints, GUI appearance/interaction, large datasets, physical removal and genuine host power loss remain unqualified. The interrupted-worker VM test is not electrical power-loss testing. Physical Omarchy, different-machine, Secure Boot, TPM/encrypted-root, RAID/LVM and alternate geometry recovery retain the Phase 4 limits. `full_system_recovery` stays false.


## Completion pass (0.6.1)

Rendered all four Backup pages at 1600 and 800 pixels in an isolated GTK Broadway fixture, in dark and light themes. Scripted checks cover all six destination field sets, invalid-save refusal without configuration mutation, Cancel/Escape, profile/recovery dialogs, shared command palette and Ctrl+Q. Destination forms now show relevant fields and help, Overview recovery copy is current, idle cancellation is disabled, and advertised close/palette shortcuts are installed through the shared Nyx handler. Physical desktop input and native file/keyring prompts are not claimed by this software-rendered fixture.

Authenticated SMB initial/incremental backup, full-data verification and selective restoration (contents/mode 0640) passed against Samba 4.25.0. The server runs in a private user/mount namespace with delegated UID/GID mappings, private account database and loopback port. No host mount definitions, real NAS credentials or user sources were changed. Wrong credentials and disconnected-server checks fail safely. Commercial NAS firmware and live NFS endpoints remain environment-specific qualification.

The installed Rclone 1.75.2 is newer than the 1.75.1 fix listed for [GHSA-38xv-hf3p-h7mq](https://github.com/rclone/rclone/security/advisories/GHSA-38xv-hf3p-h7mq). This does not constitute a complete external-tool audit. Broad physical-system recovery and live cloud/GitHub account claims remain outside the qualified scope.

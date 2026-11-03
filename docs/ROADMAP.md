# Capabilities and six-phase roadmap

## Phase 1: selected user data

Implemented: versioned private configuration/jobs; explicit profile approval; common preview/execution selection; local and identity-bound mounted repositories; SMB credential setup with encrypted Rclone configuration; direct SFTP using existing SSH settings; preconfigured user-mountable NFS management; explicit service-owned repository initialization/validation; encrypted initial/incremental backups; logical-size preview; progress, history, statistics, metadata/full verification; incomplete/cancelled/interrupted states; application and native repository locks; scoped manual retention preview/application; session service/CLI; explicit user timers; basic notifications; GTK Overview/Backups/Restore/Settings and shared Appearance/Keyboard/About; installer/launcher/icon.

Partial/limits: database/VM detection is heuristic; no live-filesystem consistency guarantee. SFTP has been qualified against a disposable SFTP-only endpoint; SMB/NFS endpoint behavior remains unverified. Mounted-device checks have synthetic absent-mount evidence, not a physical unplug test. GUI compilation is verified; GUI interaction and visual review were not run. Timer/activation evidence is recorded in qualification. Initialization, validation and retention use explicit local administration requests and run in the background service. Phase 5 adds an explicit grant-controlled consumer and Control Center tile; no always-running Backup polling is installed.

Deferred beyond Phase 1: application restoration is delivered in Phase 2 below. Automatic retention deletion, complete-system and cross-PC/boot recovery remain later work.

## Phase 2: restoration — implemented

Snapshot browsing, selected-file/folder restoration, Quick Dotfiles Restore, local UTF-8 text diffs, skip/keep-both/replace conflict policies, durable restore receipts and explicit rollback are implemented as one phase. Restores stage and verify selected data first, default to a separate folder and require plan confirmation. Replaced originals are retained; post-preview changes and changed rollback targets are refused. Jobs belong to the background service. Restore methods remain outside the integration D-Bus API.

See [restore workflow and boundaries](RESTORE.md) and [qualification](QUALIFICATION.md). File contents, symlinks, ordinary modes and modification times are supported; ownership, ACLs, xattrs and hard-link topology are not applied. GUI visual/interaction review and SMB/NFS restore endpoints are unverified. Interrupted staging/recovery data is retained. No full-system or cross-PC recovery claim is introduced.

## Phase 3: migration and Setup — implemented

Versioned native/foreign package and Flatpak inventories are included in new encrypted snapshots. Reviewed migration maps source home/config roots to the current user, stages bounded literal path adaptation, generates neutral target-format monitor defaults, holds hardware-sensitive files and preserves Phase 2 confirmation/rollback. Hardware comparison and package decisions retain target drivers, kernels and firmware. The installed `nyx-backup-setup` consumer previews target-bound JSON handoffs and supports separately confirmed native package installation and staged user-file restoration.

See [migration workflow and contract](MIGRATION.md). Local binary/language-manager inventory, arbitrary configuration conversion, automatic foreign/Flatpak/hardware installation, actual cross-machine/boot qualification and GUI interaction remain outside delivered coverage. A disposable fixture proves synthetic different-home/hardware planning, file apply/rollback and a recording-stub package installer boundary. No real package install was performed. The legacy generated Setup workflow and `omarchy-backup` remain unchanged.

## Phase 4: offline disk recovery — implemented, VM-qualified path

Version 0.4.0 delivers an independent root-run offline disk helper and a bootable x86-64 kernel/initramfs/unsigned UEFI kit. Read-only, unmounted whole disks can be captured into encrypted Restic repositories. Recovery stages and hashes the image, binds a plan to a same-geometry target and its full content fingerprint, requires exact overwrite confirmation, journals before writing, then syncs and verifies every target byte. No desktop privilege service, integration recovery method or automatic disk writer is introduced.

The end-to-end disposable QEMU test captures a GPT/UEFI/ext4 system fixture, restores a separate disk, compares every byte, then boots only the recovered disk through UEFI firmware and switches into its restored root. Destructive tests attach only disposable regular disk files. See [workflow and limits](RECOVERY.md) and [qualification](QUALIFICATION.md).

Qualification is narrow: this is not a demonstrated full Omarchy/physical-machine recovery. Hardware changes, Secure Boot, TPM/root-encryption unlock, RAID/LVM, remote recovery transports, power-loss recovery and physical storage drivers remain unqualified. The integration flag `full_system_recovery` remains false. Ordinary selected-data profiles are not whole-disk captures.

## Phase 5: ecosystem and remote providers — implemented

Version 0.6.0 adds S3-compatible providers via endpoint-bound encrypted Rclone configuration and Secret Service, explicit private-GitHub sync of selected Backup profile settings, an application-owned Core Control adapter with expiring sender/profile/action grants, and a Control Center Backup tile in its independent repository. Legacy ungranted D-Bus mutation routes are closed; local GUI/CLI jobs use the private channel. No credentials, restore/deletion, arbitrary commands or GitHub writes are exposed to integration consumers.

See [ecosystem workflow and contract](ECOSYSTEM.md). Signed S3 transport is qualified against a disposable loopback endpoint. GitHub API behavior is qualified against a local stub; real provider/account behavior is unverified. The actual Control Center provider is exercised headlessly, including stopping on unsubscribe. GUI visual/interaction review remains unverified. GitHub synchronization covers Backup settings, not arbitrary source-file contents. No shared Core revision or existing desktop layout was changed.

## Phase 6: local qualification and security review — delivered with explicit limits

Focused domain checks, existing engine/service regression, signed-S3 backup/restore/failure handling, GitHub-stub safety, sender/action/profile/expiry/name/revocation checks, provider lifecycle, a reproducible small-data performance baseline, dependency advisory review, and interrupted-disk recovery qualification are delivered together. The recovery VM refuses mismatched geometry and altered staging, retains the interrupted applying state, requires fresh planning, completes retry and boots the recovered disk. All destructive qualification stays in disposable VMs.

See [security review](SECURITY-REVIEW.md) and [recorded qualification](QUALIFICATION.md). The dependency scan found five informational maintenance advisories in transitive dependencies, recorded without suppression. This is local evidence, not certification or unrestricted recovery qualification. Live SMB/NFS and commercial cloud/GitHub endpoints, rendered GUI/interaction, large datasets, physical unplug/power loss, physical Omarchy, cross-PC, Secure Boot/TPM/encrypted-root and RAID/LVM recovery remain unverified. `full_system_recovery` remains false; new claims need their own demonstrated tests.

# Capabilities and six-phase roadmap

## Phase 1: selected user data

Implemented: versioned private configuration/jobs; explicit profile approval; common preview/execution selection; local and identity-bound mounted repositories; SMB credential setup with encrypted Rclone configuration; direct SFTP using existing SSH settings; preconfigured user-mountable NFS management; explicit service-owned repository initialization/validation; encrypted initial/incremental backups; logical-size preview; progress, history, statistics, metadata/full verification; incomplete/cancelled/interrupted states; application and native repository locks; scoped manual retention preview/application; session service/CLI; explicit user timers; basic notifications; GTK Overview/Backups/Restore/Settings and shared Appearance/Keyboard/About; installer/launcher/icon.

Partial/limits: database/VM detection is heuristic; no live-filesystem consistency guarantee. SFTP has been qualified against a disposable SFTP-only endpoint; SMB/NFS endpoint behavior remains unverified. Mounted-device checks have synthetic absent-mount evidence, not a physical unplug test. GUI compilation is verified; GUI interaction and visual review were not run. Timer/activation evidence is recorded in qualification. Initialization, validation and retention use explicit local administration requests and run in the background service. No background integration consumer or Control Center widget is shipped.

Deferred beyond Phase 1: application restoration is delivered in Phase 2 below. Automatic retention deletion, hardware migration, complete-system and cross-PC/boot recovery remain later work.

## Phase 2: restoration — implemented

Snapshot browsing, selected-file/folder restoration, Quick Dotfiles Restore, local UTF-8 text diffs, skip/keep-both/replace conflict policies, durable restore receipts and explicit rollback are implemented as one phase. Restores stage and verify selected data first, default to a separate folder and require plan confirmation. Replaced originals are retained; post-preview changes and changed rollback targets are refused. Jobs belong to the background service. Restore methods remain outside the integration D-Bus API.

See [restore workflow and boundaries](RESTORE.md) and [qualification](QUALIFICATION.md). File contents, symlinks, ordinary modes and modification times are supported; ownership, ACLs, xattrs and hard-link topology are not applied. GUI visual/interaction review and SMB/NFS restore endpoints are unverified. Interrupted staging/recovery data is retained. No full-system or cross-PC recovery claim is introduced.

## Phase 3: migration and Setup

Package manifests, hardware adaptation and Setup consumer integration. No dedicated Setup checkout was found during the planning audit. Settings' own preferences export is not a system backup engine.

## Phase 4: system recovery

Privileged recovery tooling and a bootable environment. Destructive recovery tests are confined to VMs. No complete-system/boot recovery claim before an end-to-end VM recovery succeeds.

## Phase 5: ecosystem and remote providers

Cloud destinations, GitHub configuration sync, Core Control/Control Center consumers with authenticated grants and receiving-app enforcement. Status contract exists; widget is deferred.

## Phase 6: qualification

Performance work, security review, failure testing and broader recovery qualification. Cross-PC recovery claims require their own demonstrated tests.

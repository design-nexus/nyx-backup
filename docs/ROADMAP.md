# Capabilities and six-phase roadmap

## Phase 1: selected user data

Implemented: versioned private configuration/jobs; explicit profile approval; common preview/execution selection; local and identity-bound mounted repositories; SMB credential setup with encrypted Rclone configuration; direct SFTP using existing SSH settings; preconfigured user-mountable NFS management; explicit repository initialization/validation; encrypted initial/incremental backups; logical-size preview; progress, history, statistics, metadata/full verification; incomplete/cancelled/interrupted states; application and native repository locks; scoped manual retention preview/application; session service/CLI; explicit user timers; basic notifications; GTK Overview/Backups/Restore/Settings and shared Appearance/Keyboard/About; installer/launcher/icon.

Partial/limits: database/VM detection is heuristic; no live-filesystem consistency guarantee. Native NAS transport support has not been qualified against disposable SMB/SFTP/NFS endpoints. Mounted-device checks have synthetic absent-mount evidence, not a physical unplug test. GUI compilation is verified; GUI interaction and visual review were not run. Timer/activation evidence is recorded in qualification. Administrative initialization/retention is local; keep its initiating GUI/CLI alive. No background integration consumer or Control Center widget is shipped.

Planned, not implemented in Phase 1: app snapshot browser, app-driven restoration, Quick Dotfiles Restore, automatic retention deletion, physical hardware migration, complete-system recovery, cross-PC/boot recovery and privileged operations.

## Phase 2: restoration

Snapshot browsing, selective file restoration, dotfile diffs, conflict handling and rollback. Restores must default to safe separate destinations and preserve unsaved/change guards.

## Phase 3: migration and Setup

Package manifests, hardware adaptation and Setup consumer integration. No dedicated Setup checkout was found during the planning audit. Settings' own preferences export is not a system backup engine.

## Phase 4: system recovery

Privileged recovery tooling and a bootable environment. Destructive recovery tests are confined to VMs. No complete-system/boot recovery claim before an end-to-end VM recovery succeeds.

## Phase 5: ecosystem and remote providers

Cloud destinations, GitHub configuration sync, Core Control/Control Center consumers with authenticated grants and receiving-app enforcement. Status contract exists; widget is deferred.

## Phase 6: qualification

Performance work, security review, failure testing and broader recovery qualification. Cross-PC recovery claims require their own demonstrated tests.

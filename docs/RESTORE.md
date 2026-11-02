# Phase 2: selected-file restoration

Snapshot browsing, selective restore, Quick Dotfiles Restore, text diffs, conflict handling and rollback are implemented in the app and `nyx-backupctl restore`. These are explicit local operations. The integration D-Bus API continues to expose no restore, arbitrary-command or credential retrieval method.

## Workflow

1. Open Restore, choose a profile and snapshot, then browse folders and select files or directories. Snapshots must carry both `nyx-backup` and the selected profile's ownership tag. Exact snapshot IDs pin every operation.
2. Choose a separate target folder (the default) or Quick Dotfiles Restore. The latter maps only this computer's current home/configuration paths, requires explicit credential authorization for recognized secrets, excludes database/container/VM stores, and asks you to close affected apps.
3. Choose a conflict policy: **Skip** existing files, **Keep both** under a restored suffix, or **Replace** while retaining originals for rollback.
4. Prepare a plan. Restic decrypts the selection into private staging and verifies its contents. No selected target is written during planning. The review lists every chosen source/target/action and supports local text diffs against current files.
5. Confirm the exact plan ID to apply it. Target fingerprints are checked before changes and again before replacement. Changed files require a fresh review. Jobs run in the background service, so closing the app or CLI does not stop them.
6. Use Restore receipts to preview and confirm rollback. Rollback refuses files whose contents, mode or modification time changed after restore. Saved originals and changed current files are retained when a conflict needs manual resolution.

A kept-both version is named with a snapshot/plan suffix. A replaced original is retained beside its target as `.nyx-restore-PLAN-INDEX.previous`. Rollback moves the restored version to `.nyx-restore-PLAN-INDEX.undone`, then returns the saved original. New-file rollback removes the visible restored file by moving it to the retained `.undone` path. No unrelated files are deleted.

Text diffs support ordinary UTF-8 files up to 1 MiB and are returned over the private channel for display; their contents are not stored in job state or application logs. Binary and oversized files still restore normally but have no text diff. Recognized credentials require explicit restore authorization; arbitrary files may also contain secrets.

## CLI

All commands support `--json`. Read-only inspection exits successfully even when the inspected receipt records a previous conflict. Waiting change commands return nonzero for failed, incomplete or cancelled jobs.

```sh
nyx-backupctl restore snapshots personal
nyx-backupctl restore browse personal SNAPSHOT_ID --directory /home/user/Documents
nyx-backupctl restore plan personal SNAPSHOT_ID \
  --path /home/user/Documents/report.txt --target /chosen/recovery-folder --wait
nyx-backupctl restore inspect personal PLAN_ID
nyx-backupctl restore diff personal PLAN_ID /home/user/Documents/report.txt
nyx-backupctl restore apply personal PLAN_ID --confirm PLAN_ID --wait
nyx-backupctl restore receipts personal
nyx-backupctl restore rollback-preview personal PLAN_ID
nyx-backupctl restore rollback personal PLAN_ID --confirm PLAN_ID --wait
```

`--path` can be repeated up to 256 times and can select a directory. `--policy skip` is the default; alternatives are `keep-both` and `replace`. Use `--original-dotfiles` instead of `--target` for Quick Dotfiles Restore and `--include-secrets` only when deliberately restoring recognized credential paths. Close affected applications before applying original configuration. Without `--wait`, change commands return a background job ID, which supports the existing `job`, `status` and `cancel` commands.

## Filesystem and recovery boundaries

- Restored contents, symlinks, ordinary permission bits and modification times are supported. Ownership, ACLs, extended attributes and hard-link topology are not applied by this phase. Existing originals retain their metadata because they are moved rather than rewritten.
- Writes use directory descriptors, refuse symlink ancestors and use Linux no-replace renames. Symlinks are recreated as links; their referents are not followed. Directory/special-file replacement is refused. Unsupported filesystem rename semantics fail safely; they do not fall back to an overwrite.
- Snapshot and target paths reject traversal. Backup's own configuration/state/cache and configured repository destinations are protected. Quick Restore is for current home/configuration paths, not hardware adaptation or cross-PC recovery.
- Plans and receipts are versioned private JSON under `$XDG_STATE_HOME/nyx-backup/restore/PLAN/`. Staged plaintext is below its private `stage/` directory; metadata contains fingerprints and paths, not file contents. Preparing a plan explicitly authorizes this local decryption. Interrupted staging data and recovery copies are retained; adequate disk space is needed. There is no automatic expiry or deletion of recovery data.
- Each file's intent is journalled before a replacement. Interrupted apply/rollback receipts can be inspected and rolled back; the actual target/recovery files are checked as well as the last recorded step. This is file-level recovery, not a filesystem-wide atomic transaction. Some empty directories or application temporary files may remain after interruption. Concurrent directory renames and unusual filesystem failure modes are not qualified.
- An application can have unsaved in-memory changes that filesystem checks cannot detect. Close it before restoring its configuration. Rollback protects the supported content/mode/mtime fingerprint; it does not claim to detect every ownership or extended-metadata change.

Large folder listings and plans are bounded; narrow the selection when asked. Full-system, bootable, destructive disk and cross-PC recovery remain outside this phase. Native Restic remains available independently of the app.

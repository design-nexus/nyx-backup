# Phase 3: user migration and Setup handoff

Version 0.3.0 adds package/hardware inventories, reviewed home/configuration mapping and a working `nyx-backup-setup` consumer. It is selected user-data migration, not a system image, driver installer or boot-recovery tool. A different physical machine has not been qualified.

New backups include a version 1 inventory in their encrypted profile metadata. It records native/foreign pacman packages, explicit/dependency status, versions, installed Flatpak application references, OS/architecture, CPU vendor, machine vendor/model, display PCI IDs and connected display modes. Serial numbers, network addresses and credentials are not collected by this inventory. Missing commands and inaccessible fields are reported. Local user binaries and language-specific tool managers are outside package coverage. Inventory includes resolved home/config paths; differing aliases require manual review.

## Review workflow

In Restore, inspect the current migration inventory, browse a profile-owned snapshot and choose **Migration / Setup**. Select a small configuration folder or individual files, close the affected applications, and prepare. With no selected files, preparation produces a package-only bundle. Review package decisions, held files, hardware differences and staged file diffs. Export is explicit and creates a new private JSON file. Apply files through the existing confirmed restore plan; rollback retains the existing Phase 2 safeguards.

Source home and XDG config roots map to the receiving user's resolved roots. Known hardware-sensitive Hyprland/environment/service files, recognized stores, special files, symlinks and Backup's own files/repositories are held rather than guessed. Recognized credentials need explicit authorization. Unknown files can contain secrets or executable configuration; review the selection and diff. At most 256 resolved regular files are supported per plan; use separate-folder restoration for bulk data.

A selected monitor file becomes a neutral automatic-output configuration in the target's detected Lua or legacy syntax. This removes source connector/mode/graphics assumptions. It does not tune monitors or guarantee the target includes that file. Unknown target syntax is held for review. Existing main Hyprland configuration is preserved.

Small recognized UTF-8 configuration formats can have literal source-home/config path prefixes rewritten to target roots. This is bounded text adaptation, not semantic conversion: embedded URLs are preserved, unsupported/large/binary formats are unchanged, and aliases, escaped paths, scripts and application-specific semantics may need manual work. JSON is parsed after rewriting. Disable rewriting in CLI if needed. Generated content is staged and shown in diffs before apply.

## CLI and installed Setup consumer

```sh
nyx-backupctl migration capture --output /new/source-inventory.json
nyx-backupctl migration source PROFILE EXACT_SNAPSHOT_ID
nyx-backupctl migration prepare PROFILE EXACT_SNAPSHOT_ID \
  --path /old/home/.config/example --policy replace --wait
nyx-backupctl migration bundles PROFILE
nyx-backupctl migration inspect PROFILE BUNDLE_ID
nyx-backupctl migration export PROFILE BUNDLE_ID /new/setup.json

# Prepare and export in one operation, on the receiving target:
nyx-backup-setup prepare PROFILE EXACT_SNAPSHOT_ID \
  --path /old/home/.config/example --bundle /new/setup.json
nyx-backup-setup inspect /new/setup.json
nyx-backup-setup packages /new/setup.json
# Explicit package installation (normal sudo/pacman interactive authorization):
nyx-backup-setup packages /new/setup.json --apply --confirm BUNDLE_ID
# Separately confirm applying the staged user files:
nyx-backup-setup restore /new/setup.json --confirm BUNDLE_ID --wait
```

Commands accept `--json`. `migration prepare` and `setup prepare` accept `--source-manifest FILE`, `--include-secrets`, `--policy skip|keep-both|replace` and `--no-rewrite-references`. Older snapshots without inventories require an explicitly supplied reviewed source manifest or a new backup. `source_attested` means inventory came from the selected encrypted snapshot; an explicit override is marked false. It is provenance within a user-owned repository, not independent certification.

Setup previews by default. Only missing, explicit, native packages available in the target's existing configured sync databases qualify for automatic installation, and both inventories must be Arch/Omarchy with matching architecture. Versions are provenance only. Dependencies, already-installed packages, recognized kernels/firmware/drivers/vendor packages, foreign/AUR builds and unavailable packages get separate decisions. Flatpaks are listed for manual review; no remotes or source build scripts are replayed. Hardware classification is heuristic and cannot recognize every hardware-dependent package.

The consumer rechecks the actual target, current package inventory and availability at application time, then runs the fixed argument-array command `sudo pacman -S --needed -- VALIDATED_NAMES`. It does not refresh databases, remove packages, downgrade, edit repositories or suppress package-manager confirmation. Keep the target updated using its ordinary maintenance workflow; old sync databases can cause installation failure. [Pacman command reference](https://man.archlinux.org/man/pacman.8.en). Package installation may run the target repositories' normal hooks and requires the user's ordinary system authorization. No privileged service is installed and no package installer is exposed over Backup D-Bus.

## Contract and boundaries

The public handoff format is `nyx-backup-setup`, version 1. Its [JSON Schema](../data/setup-contract-v1.schema.json) is installed in the XDG data directory under `nyx-backup/`. Typed runtime validation additionally enforces identities, path restrictions, target binding, size bounds, SHA-256 consistency and current package eligibility. Digest calculation serializes the Rust `Bundle` with its digest field empty, using compact serde JSON in declared field order. It detects accidental changes; it is not a signature or trust grant. Treat bundles as private metadata: they contain usernames/paths, package and hardware information.

A bundle is a **target-bound review handoff**, not a portable archive of file contents. Prepare it on the receiving machine after connecting its repository and credentials. Embedded restore-plan IDs refer to that machine's private staging state; copying a bundle alone cannot restore files. Reprepare after changing target hardware/home/config roots. Exports never include repository or NAS passwords. Plaintext staged data and rollback copies remain private local state as in Phase 2.

No independent `nyx-setup` checkout was available. The consumer is shipped and versioned with Backup rather than modifying the legacy generated setup repository or `omarchy-backup`. Other consumers can read the contract; they must preserve the receiving application's checks. Public session D-Bus remains read/status and approved-backup only. Migration planning uses the private UID-checked local channel.

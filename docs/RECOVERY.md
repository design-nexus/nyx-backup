# Phase 4: offline disk recovery

Nyx Backup 0.4.0 adds a separate root-run offline helper, `nyx-backup-recovery`, and a bootable x86-64 kernel/initramfs/UEFI kit builder. No privileged desktop daemon, setuid binary, polkit grant or destructive D-Bus operation is installed. Normal desktop backup profiles still cover selected data; **they are not complete-system captures**.

The qualified path is exact offline whole-disk capture and recovery on a disposable x86-64 UEFI GPT/ext4 VM. It preserves the complete byte stream, including partition tables and boot files. A recovered fixture actually boots through UEFI firmware and switches into its restored root filesystem. This is not qualification of an Omarchy installation, physical machine, cross-PC migration, Secure Boot, TPM unlock, encrypted-root boot, RAID, LVM or vendor firmware behavior.

## Create and boot a kit

Use **Restore → Offline disk recovery → Create a new recovery boot kit**, or:

```sh
nyx-backup-recovery-kit --output /new/kit-directory
# In a checkout:
python3 tools/build-recovery.py --output /new/kit-directory --kernel-version INSTALLED_VERSION
```

The builder reads installed kernel/module files, ELF dependencies, mkinitcpio BusyBox, Restic and the recovery helper. It creates `vmlinuz`, `initramfs.cpio.gz`, `nyx-recovery.efi`, a checksum manifest and BOOT.txt in a new directory. It uses Python 3, kmod, binutils (`objcopy`/`objdump`), systemd's x64 EFI stub and the installed mkinitcpio BusyBox. It needs no root and never writes host boot files, partitions or disks. The kit contains tools/libraries, not your repository passwords or user files. Modules cover virtio, ext4 and FAT; additional physical storage/network support is not promised. Libraries and kernel retain their upstream licenses; the local kit is not published as a redistributed image.

`nyx-recovery.efi` is an unsigned unified kernel image. Place it as `EFI/BOOT/BOOTX64.EFI` on an independently prepared FAT EFI system partition. **No USB/disk writer is supplied or run automatically.** Use your own Secure Boot signing workflow if needed. The kit can also boot using QEMU `-kernel` and `-initrd`; BOOT.txt gives a no-disks-attached example. It does not set up networking, unlock volumes or mount disks automatically. Test it on the intended hardware before relying on it. [Systemd EFI stub reference](https://www.freedesktop.org/software/systemd/man/latest/systemd-stub.html).

## Capture a recovery disk

Boot the kit with the source system shut down. Mount independent local repository and scratch storage yourself. Scratch must have space for the entire disk's logical byte length, including unused space. Do not mount any source partitions or activate their swap/RAID/LVM. The source must be a whole disk and kernel read-only (for example, `blockdev --setro` in the offline environment); capture refuses writable sources. This is not live backup or filesystem freezing.

```sh
nyx-backup-recovery devices --json
# Explicit initialization is only for a new repository path:
nyx-backup-recovery capture --device /dev/SELECTED_SOURCE \
  --repo /mounted-independent-storage/recovery-repository \
  --work /mounted-scratch/new-capture --initialize
# Omit --initialize for later captures into an existing recovery repository.
```

Commands prompt on the controlling terminal. `--password-stdin` accepts a password through a pipe for controlled automation; never put it in argv, environment, ordinary configuration or shell history. A fresh inherited pipe passes it to Restic's password-file descriptor. Passwords are not exported, stored in plans or logged. Keep a recovery password separately before depending on the repository. The offline tool deliberately does not read the desktop keyring, Rclone configuration, SSH settings or user profile configuration. Repository paths are local mounted paths; remote recovery transports require independent offline mounting/setup and are unqualified.

Capture exclusively opens the whole disk, refuses partitions, loop/memory/device-mapper/RAID devices, mounted descendants, active holders and swap, copies the full disk to private staging, hashes it and saves a versioned manifest in a standard encrypted Restic snapshot tagged `nyx-recovery-v1`. Initialization is separate explicit authorization; any failed read/credential/connection never initializes a substitute. Restic locking remains enabled. Successful capture deletes its transient plaintext disk image; the small manifest remains. Failed operations retain private staging for inspection. The encrypted disk image contains **all disk data**, including unallocated regions, deleted remnants, keys and credentials. Capture itself is explicit authorization to include these bytes; defaults for ordinary user profiles do not apply.

## Prepare, review and apply

Disconnect the original disk before eventually booting the copy. Connect an expendable **exact-size** target with the same logical sector size. No automatic repartitioning, shrink/grow, hardware adaptation or bootloader reinstallation is performed. Disk/filesystem UUIDs are cloned, so attaching both copies to a normal boot can be hazardous.

```sh
nyx-backup-recovery plan --repo /mounted-storage/recovery-repository \
  --snapshot EXACT_64_CHARACTER_ID --device /dev/SELECTED_TARGET \
  --work /mounted-scratch/new-reviewed-plan
# Read the returned identity, byte count, plan path and exact confirmation token.
nyx-backup-recovery apply --plan /mounted-scratch/new-reviewed-plan/plan.json \
  --confirm ERASE:EXACT_PLAN_ID:MAJOR:MINOR
```

Preparation validates the snapshot tag and exact ID, downloads the encrypted manifest and image, validates geometry and full SHA-256, and fingerprints the entire unmounted target. It writes no target bytes. Review the physical disk identity yourself; major/minor numbers alone do not establish a disk's purpose. `apply` reopens the disk exclusively, rechecks identity, size, current content and staged image, requires the exact plan token, then durably marks the plan `applying` before the first destructive write. It copies and syncs every byte and rereads the whole target to compare its hash. Only then is the receipt `verified`.

There is **no disk overwrite rollback**. Cancellation, power loss or I/O failure after writing starts can leave a partial disk. The durable state remains `applying`; automatic byte-level resume and reapplying that plan are refused. Prepare a new plan from the unchanged repository to retry. Keep the source/repository until recovery has been tested. Staging contains private plaintext disk bytes and is retained after preparation/application; remove your chosen staging folder yourself when no longer needed. The offline environment's `/run` is volatile; use persistent scratch if the receipt must survive reboot.

The helper requires root, `/etc/nyx-recovery-environment` and the boot parameter `nyx.recovery=1`. This prevents accidental desktop use; it is not a security boundary against someone who already controls root. Linux exclusive block opens and mount/holder/swap checks reduce accidental writes, but cannot guarantee safety against another privileged process or faulty hardware. No repository lock is removed automatically. [Linux exclusive block opens](https://man7.org/linux/man-pages/man2/open.2.html), [Restic backup behavior](https://restic.readthedocs.io/en/stable/040_backup.html).

## Evidence and limits

`tools/recovery-smoke.py` builds disposable kits and regular disk files, attaches only those files to QEMU, creates a GPT/UEFI/ext4 fixture inside the guest, captures the read-only source into an encrypted Restic repository, restores to a separate target and compares all bytes. A second VM attaches only the recovered target and UEFI firmware; it boots its restored EFI image and root filesystem and checks contents, mode and a symlink. Host disks, real user files and live NAS credentials are never attached or selected. See QUALIFICATION.md for recorded results.

An exact-image boot fixture supports only that demonstrated recovery path. Full production-system, physical-machine and cross-PC recovery claims remain unaccepted until their own end-to-end tests pass. The desktop integration continues to report `full_system_recovery: false`.

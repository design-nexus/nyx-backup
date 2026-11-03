//! Boot-kit creation only. No privileged or disk-writing action in GTK/D-Bus.
use super::*;
pub fn show(ui: &Rc<Ui>) {
    let d = dialog(&ui.window, "Offline disk recovery");
    let body = w::vbox(12);
    body.append(&text(
        "Capture and restore an offline whole disk",
        "heading",
    ));
    body.append(&text("The separate recovery environment captures an unmounted, kernel read-only disk into an encrypted local Restic repository. Recovery stages and verifies the full image, reviews a target of matching size and sector geometry, then requires exact confirmation before overwriting every target byte. It preserves partition tables, filesystems and boot files together. There is no undo for a disk overwrite.", ""));
    body.append(&text("Ordinary Backup profiles contain selected user data and cannot be used as whole-disk images. Boot the recovery environment, mount separate repository and scratch storage, and use nyx-backup-recovery. Passwords are entered there; no desktop privilege helper or destructive integration API is installed.", "dim-label"));
    body.append(&text("Qualification is limited to a disposable x86-64 UEFI GPT/ext4 VM. Physical machines, hardware changes, Secure Boot, TPM unlock, RAID/LVM and Omarchy system recovery remain unverified. The kit is unsigned and includes the installed kernel; it is not a universal rescue distribution. It does not configure networking or mount disks automatically.", "dim-label"));
    let u = ui.clone();
    let parent = d.clone();
    body.append(&action("media-floppy-symbolic", "Create a new recovery boot kit in a chosen folder", move || {
        let ui = u.clone();
        gtk::FileDialog::builder().title("Choose parent folder for a new recovery kit").build().select_folder(Some(&parent), gio::Cancellable::NONE, move |result| {
            if let Ok(folder) = result {
                if let Some(parent) = folder.path() {
                    let output = parent.join(format!("nyx-recovery-{}", backup_core::now()));
                    let path = output.clone();
                    work(&ui, "Building offline recovery kit…", move || {
                        let result = std::process::Command::new(nyx_core::paths::data_home().join("nyx-backup/tools/build-recovery.py"))
                            .arg("--output").arg(&path).output()?;
                        anyhow::ensure!(result.status.success(), "Recovery kit could not be built: {}", String::from_utf8_lossy(&result.stderr));
                        Ok(path.display().to_string())
                    }, |ui, path| message(ui, "Recovery kit created", &format!("Created {path}\n\nThe folder contains a UEFI image, kernel/initramfs, checksums and BOOT.txt. No disk was written or mounted. Test the kit before relying on it; Secure Boot needs your own signing workflow.")));
                }
            }
        });
    }));
    d.content_area().append(&body);
    d.connect_response(|d, _| d.close());
    d.present();
}

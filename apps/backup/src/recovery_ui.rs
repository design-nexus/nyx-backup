//! Boot-kit creation only. No privileged or disk-writing action in GTK/D-Bus.
use crate::forms::{self, Sheet};
use crate::ui::*;
use gtk::prelude::*;
use std::rc::Rc;

pub fn show(ui: &Rc<Ui>) {
    let sheet = Sheet::new("Offline disk recovery", 500);
    sheet.body.append(&label(
        "A separate boot environment copies a whole, unmounted disk into an encrypted repository, and can write such a copy back onto a disk of the same size. Writing a disk replaces everything on it and can't be undone.",
        "",
    ));
    sheet.body.append(&label(
        "Ordinary backups hold selected files and can't become disk images later. Boot the kit, mount separate storage for the repository and a scratch copy yourself, then use nyx-backup-recovery. Passwords are entered there.",
        "field-hint",
    ));
    sheet.body.append(&label(
        "Tested only on a disposable x86-64 UEFI virtual machine with GPT and ext4. Physical machines, Secure Boot, TPM unlock, RAID, LVM and networking aren't covered, and the kit is unsigned.",
        "field-hint",
    ));
    sheet.close_button("Close");
    let create = sheet.action("Create boot kit…", true);
    let (s, u) = (sheet.clone(), ui.clone());
    create.connect_clicked(move |_| {
        let (s, u) = (s.clone(), u.clone());
        let parent = s.parent.clone();
        forms::choose_folder(&parent, "Choose where to create the boot kit", move |folder| {
            let output = folder.join(format!("nyx-recovery-{}", backup_core::now()));
            s.submit(
                &u,
                move || {
                    let tool = nyx_core::paths::data_home().join("nyx-backup/tools/build-recovery.py");
                    let result = std::process::Command::new(tool)
                        .arg("--output")
                        .arg(&output)
                        .output()?;
                    anyhow::ensure!(
                        result.status.success(),
                        "The boot kit couldn't be built: {}",
                        String::from_utf8_lossy(&result.stderr).trim()
                    );
                    Ok(output)
                },
                |_, path| {
                    info(
                        "Boot kit created",
                        &format!(
                            "{}\n\nIt holds a UEFI image, the kernel and initramfs, checksums and BOOT.txt. No disk was written or mounted. Try booting it before you rely on it.",
                            path.display()
                        ),
                    )
                },
            );
        });
    });
    sheet.present();
}

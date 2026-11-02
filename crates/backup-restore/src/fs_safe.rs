//! Descriptor-relative writes: no symlink ancestors and no silent replacement.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::{self, File},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub kind: String,
    pub digest: String,
    pub mode: u32,
    pub size: u64,
    pub mtime: i64,
    pub mtime_nsec: i64,
}
fn name(path: &Path) -> Result<CString> {
    Ok(CString::new(path.as_os_str().as_bytes())?)
}
pub fn clean_absolute(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
        "Use an absolute path without traversal components"
    );
    ensure!(!path.as_os_str().as_bytes().contains(&0), "Invalid path");
    Ok(())
}
pub fn resolved(path: &Path) -> Result<PathBuf> {
    clean_absolute(path)?;
    let mut ancestor = path;
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .context("Missing ancestor")?
                        .to_os_string(),
                );
                ancestor = ancestor.parent().context("Missing parent")?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    let mut canonical = fs::canonicalize(ancestor)?;
    for component in suffix.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}
fn open_dir(path: &Path, create: bool, created: &mut Vec<PathBuf>) -> Result<File> {
    clean_absolute(path)?;
    let mut directory = File::open("/")?;
    let mut current = PathBuf::from("/");
    for component in path.components() {
        let Component::Normal(component) = component else {
            continue;
        };
        current.push(component);
        let n = name(Path::new(component))?;
        let mut fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                n.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0
            && create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            let made = unsafe { libc::mkdirat(directory.as_raw_fd(), n.as_ptr(), 0o700) };
            if made == 0 {
                created.push(current.clone());
            } else if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
                return Err(std::io::Error::last_os_error().into());
            }
            fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    n.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
        }
        ensure!(
            fd >= 0,
            "Destination ancestor is unavailable or a symlink: {}",
            current.display()
        );
        directory = unsafe { File::from_raw_fd(fd) };
    }
    Ok(directory)
}
pub struct Parent {
    pub fd: File,
    pub leaf: CString,
}
impl Parent {
    pub fn open(path: &Path, create: bool, created: &mut Vec<PathBuf>) -> Result<Self> {
        clean_absolute(path)?;
        let parent = path
            .parent()
            .context("Cannot replace the filesystem root")?;
        Ok(Self {
            fd: open_dir(parent, create, created)?,
            leaf: name(Path::new(path.file_name().context("Missing file name")?))?,
        })
    }
    fn stat(&self) -> Result<Option<libc::stat>> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                self.fd.as_raw_fd(),
                self.leaf.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error.into());
        }
        Ok(Some(unsafe { stat.assume_init() }))
    }
    pub fn read_file(&self) -> Result<File> {
        let fd = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                self.leaf.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        ensure!(fd >= 0, "File is not safely readable");
        let file = unsafe { File::from_raw_fd(fd) };
        ensure!(file.metadata()?.is_file(), "Select an ordinary file");
        Ok(file)
    }
    pub fn fingerprint(&self) -> Result<Option<Fingerprint>> {
        let Some(stat) = self.stat()? else {
            return Ok(None);
        };
        let mode = stat.st_mode & 0o777;
        let kind = stat.st_mode & libc::S_IFMT;
        if kind == libc::S_IFREG {
            let mut file = self.read_file()?;
            let before = file.metadata()?;
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let len = file.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                digest.update(&buffer[..len]);
            }
            let after = file.metadata()?;
            ensure!(
                before.ino() == after.ino()
                    && before.len() == after.len()
                    && before.mtime() == after.mtime()
                    && before.mtime_nsec() == after.mtime_nsec()
                    && before.ctime() == after.ctime()
                    && before.ctime_nsec() == after.ctime_nsec(),
                "File changed while checking it; review a new restore plan"
            );
            let current = self.stat()?.context("File disappeared while checking it")?;
            ensure!(
                current.st_ino == after.ino() && current.st_dev == after.dev(),
                "File was replaced while checking it"
            );
            return Ok(Some(Fingerprint {
                kind: "file".into(),
                digest: format!("{:x}", digest.finalize()),
                mode,
                size: after.len(),
                mtime: after.mtime(),
                mtime_nsec: after.mtime_nsec(),
            }));
        }
        if kind == libc::S_IFLNK {
            let mut buffer = vec![0u8; 65536];
            let len = unsafe {
                libc::readlinkat(
                    self.fd.as_raw_fd(),
                    self.leaf.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            ensure!(
                len >= 0 && (len as usize) < buffer.len(),
                "Could not read symbolic link"
            );
            buffer.truncate(len as usize);
            return Ok(Some(Fingerprint {
                kind: "symlink".into(),
                digest: format!("{:x}", Sha256::digest(&buffer)),
                mode,
                size: len as u64,
                mtime: stat.st_mtime,
                mtime_nsec: stat.st_mtime_nsec,
            }));
        }
        anyhow::bail!("Restore refuses to replace a directory or special file")
    }
    pub fn rename_to(&self, other: &Self) -> Result<()> {
        let status = unsafe {
            libc::renameat2(
                self.fd.as_raw_fd(),
                self.leaf.as_ptr(),
                other.fd.as_raw_fd(),
                other.leaf.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if status < 0 {
            return Err(std::io::Error::last_os_error())
                .context("Safe rename failed: destination changed, already exists, or lacks no-replace support; no file was overwritten");
        }
        self.fd.sync_all()?;
        other.fd.sync_all()?;
        Ok(())
    }
    pub fn create_from(
        &self,
        source: &Path,
        expected: &Fingerprint,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<()> {
        if expected.kind == "file" {
            let fd = unsafe {
                libc::openat(
                    self.fd.as_raw_fd(),
                    self.leaf.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            ensure!(
                fd >= 0,
                "Restore temporary path already exists or is unavailable"
            );
            let mut target = unsafe { File::from_raw_fd(fd) };
            let source_parent = Parent::open(source, false, &mut vec![])?;
            let mut input = source_parent.read_file()?;
            {
                use std::io::Write;
                let mut buffer = [0u8; 65536];
                loop {
                    check()?;
                    let size = input.read(&mut buffer)?;
                    if size == 0 {
                        break;
                    }
                    target.write_all(&buffer[..size])?;
                }
            }
            target.set_permissions(fs::Permissions::from_mode(expected.mode))?;
            if let Ok(modified) = input.metadata()?.modified() {
                target.set_times(fs::FileTimes::new().set_modified(modified))?;
            }
            target.sync_all()?;
        } else if expected.kind == "symlink" {
            let link = fs::read_link(source)?;
            let link = name(&link)?;
            ensure!(
                unsafe { libc::symlinkat(link.as_ptr(), self.fd.as_raw_fd(), self.leaf.as_ptr()) }
                    == 0,
                "Could not create the staged symbolic link"
            );
        } else {
            anyhow::bail!("Unsupported restore entry")
        }
        if expected.kind == "symlink" {
            let times = [
                libc::timespec {
                    tv_sec: 0,
                    tv_nsec: libc::UTIME_OMIT,
                },
                libc::timespec {
                    tv_sec: expected.mtime,
                    tv_nsec: expected.mtime_nsec,
                },
            ];
            ensure!(
                unsafe {
                    libc::utimensat(
                        self.fd.as_raw_fd(),
                        self.leaf.as_ptr(),
                        times.as_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } == 0,
                "Could not preserve symlink modification time"
            );
        }
        self.fd.sync_all()?;
        ensure!(
            self.fingerprint()?.as_ref() == Some(expected),
            "Staged file changed while copying; restore stopped"
        );
        Ok(())
    }
    pub fn remove_owned(&self, expected: &Fingerprint) -> Result<()> {
        ensure!(
            self.fingerprint()?.as_ref() == Some(expected),
            "Temporary restore data changed; retaining it"
        );
        ensure!(
            unsafe { libc::unlinkat(self.fd.as_raw_fd(), self.leaf.as_ptr(), 0) } == 0,
            "Could not remove restore temporary file"
        );
        self.fd.sync_all()?;
        Ok(())
    }
}
pub fn fingerprint(path: &Path) -> Result<Option<Fingerprint>> {
    // Missing ancestors are safe during a preview; symlink ancestors are never followed.
    let mut ancestor = path.parent().context("No parent")?;
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(m) => {
                ensure!(
                    !m.file_type().is_symlink(),
                    "Destination ancestor is a symlink"
                );
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().context("Missing parent")?
            }
            Err(e) => return Err(e.into()),
        }
    }
    let _ = open_dir(ancestor, false, &mut vec![])?;
    if ancestor != path.parent().unwrap() {
        return Ok(None);
    }
    Parent::open(path, false, &mut vec![])?.fingerprint()
}
pub fn ensure_directory(path: &Path, created: &mut Vec<PathBuf>) -> Result<()> {
    let _ = open_dir(path, true, created)?;
    Ok(())
}
pub fn remove_empty_directory(path: &Path) -> Result<()> {
    let parent = Parent::open(path, false, &mut vec![])?;
    let status = unsafe {
        libc::unlinkat(
            parent.fd.as_raw_fd(),
            parent.leaf.as_ptr(),
            libc::AT_REMOVEDIR,
        )
    };
    if status < 0 {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ENOTEMPTY | libc::ENOENT)) {
            return Ok(());
        }
        return Err(error.into());
    }
    parent.fd.sync_all()?;
    Ok(())
}

//! Offline-only whole-disk recovery. Never linked into the desktop service.
use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[derive(Parser)]
#[command(
    version,
    about = "Offline whole-disk recovery; run only from the Nyx recovery environment"
)]
struct Args {
    #[arg(long, global = true)]
    json: bool,
    /// Read a recovery password from stdin instead of the controlling terminal.
    #[arg(long, global = true)]
    password_stdin: bool,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Read-only inventory; also available outside the recovery environment.
    Devices,
    /// Capture an offline, read-only whole disk into an encrypted local Restic repository.
    Capture {
        #[arg(long)]
        device: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        work: PathBuf,
        #[arg(long)]
        initialize: bool,
    },
    /// Stage and verify the exact recovery image, then fingerprint an unmounted target.
    Plan {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        device: PathBuf,
        #[arg(long)]
        work: PathBuf,
    },
    /// Overwrite exactly the reviewed target disk. Cannot restore a running system.
    Apply {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        confirm: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Device {
    path: PathBuf,
    sys_path: PathBuf,
    major: u32,
    minor: u32,
    bytes: u64,
    sector: u32,
    model: String,
    serial: String,
    readonly: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    bytes: u64,
    sha256: String,
    source: Device,
    coverage: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    id: String,
    snapshot: String,
    device: Device,
    before_sha256: String,
    image_sha256: String,
    bytes: u64,
    state: String,
}
fn environment() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Recovery requires root in the offline boot environment"
    );
    ensure!(
        fs::read_to_string("/etc/nyx-recovery-environment")
            .ok()
            .as_deref()
            == Some("nyx-backup-recovery-v1\n")
            && fs::read_to_string("/proc/cmdline")?
                .split_whitespace()
                .any(|x| x == "nyx.recovery=1"),
        "Boot the Nyx recovery environment; recovery is refused on the installed desktop"
    );
    Ok(())
}
fn clean(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Choose an absolute path without parent traversal"
    );
    Ok(())
}
fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().into()
}
fn identify(path: &Path, file: &File) -> Result<Device> {
    let m = file.metadata()?;
    ensure!(
        m.file_type().is_block_device(),
        "Choose a whole block device, not an image file or ordinary path"
    );
    let major = libc::major(m.rdev());
    let minor = libc::minor(m.rdev());
    let sys = fs::canonicalize(format!("/sys/dev/block/{major}:{minor}"))?;
    ensure!(
        !sys.join("partition").exists(),
        "Choose a whole disk, not a partition"
    );
    let name = sys
        .file_name()
        .context("Missing disk name")?
        .to_string_lossy();
    ensure!(
        !["loop", "dm-", "md", "ram", "zram"]
            .iter()
            .any(|p| name.starts_with(p)),
        "Loop, device-mapper, RAID and memory devices are outside this recovery format"
    );
    ensure!(
        fs::read_dir(sys.join("holders"))?.next().is_none(),
        "Disk has an active holder; deactivate it offline first"
    );
    let mut bytes = 0u64;
    let mut sector = 0u32;
    let mut ro = 0i32;
    ensure!(
        unsafe { libc::ioctl(file.as_raw_fd(), 0x80081272u64, &mut bytes) } == 0
            && unsafe { libc::ioctl(file.as_raw_fd(), 0x1268u64, &mut sector) } == 0
            && unsafe { libc::ioctl(file.as_raw_fd(), 0x125eu64, &mut ro) } == 0,
        "Cannot read block-device geometry"
    );
    ensure!(
        bytes > 0 && [512, 4096].contains(&sector),
        "Unsupported disk geometry"
    );
    // Include every descendant partition's major/minor, not merely the whole disk.
    let mut devices = vec![format!("{major}:{minor}")];
    for entry in fs::read_dir(&sys)? {
        let p = entry?.path();
        if p.join("partition").exists() {
            devices.push(read(p.join("dev")));
            ensure!(
                fs::read_dir(p.join("holders"))?.next().is_none(),
                "Partition has an active holder"
            );
        }
    }
    for line in fs::read_to_string("/proc/self/mountinfo")?.lines() {
        if let Some(dev) = line.split_whitespace().nth(2) {
            ensure!(
                !devices.iter().any(|d| d == dev),
                "Disk or partition is mounted; unmount it before recovery"
            );
        }
    }
    for line in fs::read_to_string("/proc/swaps")?.lines().skip(1) {
        if let Some(p) = line.split_whitespace().next() {
            if let Ok(m) = fs::metadata(p) {
                let dev = if m.file_type().is_block_device() {
                    m.rdev()
                } else {
                    m.dev()
                };
                ensure!(
                    !devices.contains(&format!("{}:{}", libc::major(dev), libc::minor(dev))),
                    "Disk contains active swap"
                );
            }
        }
    }
    Ok(Device {
        path: fs::canonicalize(path)?,
        sys_path: sys.clone(),
        major,
        minor,
        bytes,
        sector,
        model: read(sys.join("device/model")),
        serial: read(sys.join("serial")),
        readonly: ro != 0,
    })
}
fn open_device(path: &Path, write: bool) -> Result<(File, Device)> {
    clean(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .custom_flags(libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .context("Cannot exclusively open disk; check permissions and unmount all partitions")?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "Another recovery worker owns the disk"
    );
    let dev = identify(path, &file)?;
    ensure!(!write || !dev.readonly, "Target disk is read-only");
    Ok((file, dev))
}
fn hash(file: &mut File, bytes: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut h = Sha256::new();
    let mut left = bytes;
    let mut buf = vec![0; 1024 * 1024];
    while left > 0 {
        let n = file.read(&mut buf[..left.min(1024 * 1024) as usize])?;
        ensure!(n > 0, "Short disk/image read");
        h.update(&buf[..n]);
        left -= n as u64;
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(format!("{:x}", h.finalize()))
}
fn private_dir(path: &Path) -> Result<()> {
    clean(path)?;
    fs::create_dir(path).context(
        "Choose a new private staging directory with enough space for a full disk image",
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
fn atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("new");
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    f.sync_all()?;
    fs::rename(tmp, path)?;
    File::open(path.parent().context("Missing parent")?)?.sync_all()?;
    Ok(())
}
fn password(stdin: bool) -> Result<Vec<u8>> {
    let s = if stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        s.trim_end_matches(['\r', '\n']).to_string()
    } else {
        rpassword::prompt_password("Repository recovery password: ")?
    };
    ensure!(
        !s.is_empty() && s.len() <= 4096 && !s.contains(['\n', '\r']),
        "Choose a nonempty recovery password (at most 4096 bytes)"
    );
    Ok(s.into_bytes())
}
// A fresh inherited pipe for every child: no password argv/env/log/file.
fn restic(repo: &Path, secret: &[u8]) -> Result<Command> {
    clean(repo)?;
    let mut fds = [-1; 2];
    ensure!(
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == 0,
        "Cannot create credential pipe"
    );
    let input = unsafe { File::from_raw_fd(fds[0]) };
    let mut output = unsafe { File::from_raw_fd(fds[1]) };
    output.write_all(secret)?;
    output.write_all(b"\n")?;
    drop(output);
    let mut c = Command::new("/usr/bin/restic");
    for (k, _) in std::env::vars() {
        if k.starts_with("RESTIC_") || k.starts_with("RCLONE_") {
            c.env_remove(k);
        }
    }
    c.args(["--repo"])
        .arg(repo)
        .args(["--password-file", "/proc/self/fd/3", "--no-cache"]);
    c.stdin(Stdio::null()).stderr(Stdio::null());
    unsafe {
        c.pre_exec(move || {
            let fd = input.as_raw_fd();
            if fd != 3 && libc::dup2(fd, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(c)
}
fn run(mut c: Command) -> Result<Vec<u8>> {
    let out = c.output()?;
    ensure!(
        out.status.success(),
        "Restic recovery operation failed ({}); check repository, password, free space and locks. No replacement repository is initialized.",
        out.status
    );
    Ok(out.stdout)
}
fn exact(s: &str) -> Result<()> {
    ensure!(
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Choose an exact snapshot ID"
    );
    Ok(())
}
fn snapshot(repo: &Path, secret: &[u8], id: &str) -> Result<()> {
    exact(id)?;
    let mut c = restic(repo, secret)?;
    c.args(["snapshots", "--json", id]);
    let data: serde_json::Value = serde_json::from_slice(&run(c)?)?;
    let a = data.as_array().context("Invalid snapshot listing")?;
    ensure!(
        a.len() == 1
            && a[0]["id"] == id
            && a[0]["tags"]
                .as_array()
                .is_some_and(|t| t.iter().any(|v| v == "nyx-recovery-v1")),
        "Snapshot is not a Nyx whole-disk recovery capture"
    );
    Ok(())
}
fn dump(repo: &Path, secret: &[u8], snapshot: &str, name: &str, path: &Path) -> Result<()> {
    let f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let mut c = restic(repo, secret)?;
    c.args(["dump", snapshot, name]).stdout(Stdio::from(f));
    let status = c.status()?;
    ensure!(
        status.success(),
        "Restic recovery image read failed; target has not been written"
    );
    File::open(path)?.sync_all()?;
    Ok(())
}
fn capture(
    device: &Path,
    repo: &Path,
    work: &Path,
    initialize: bool,
    secret: &[u8],
) -> Result<serde_json::Value> {
    let (mut disk, source) = open_device(device, false)?;
    ensure!(
        source.readonly,
        "Capture requires a kernel read-only source disk; boot offline and set it read-only first"
    );
    // Repository and work must be on mounted storage, never on the source being copied.
    private_dir(work)?;
    let image = work.join("disk.img");
    let mut out = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .mode(0o600)
        .open(&image)?;
    ensure!(
        std::io::copy(&mut (&mut disk).take(source.bytes), &mut out)? == source.bytes,
        "Short source-disk capture"
    );
    out.sync_all()?;
    ensure!(
        identify(device, &disk)? == source,
        "Source device identity changed during capture"
    );
    let manifest=Manifest{format:"nyx-recovery-disk".into(),version:1,bytes:source.bytes,sha256:hash(&mut out,source.bytes)?,source,coverage:"Offline exact whole-disk bytes; target geometry must match. No hardware adaptation or physical-machine recovery qualification.".into()};
    atomic(&work.join("manifest.json"), &manifest)?;
    if initialize {
        ensure!(
            !repo.exists(),
            "Explicit initialization requires a new repository path"
        );
        let mut c = restic(repo, secret)?;
        c.arg("init");
        run(c)?;
    }
    let mut c = restic(repo, secret)?;
    c.current_dir(work).args([
        "backup",
        "--json",
        "--tag",
        "nyx-recovery-v1",
        "--",
        "disk.img",
        "manifest.json",
    ]);
    let data = run(c)?;
    let result = data
        .split(|b| *b == b'\n')
        .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
        .find(|v| v["message_type"] == "summary")
        .context("Restic did not report a completed recovery snapshot")?;
    let id = result["snapshot_id"]
        .as_str()
        .context("Missing recovery snapshot ID")?;
    exact(id)?;
    fs::remove_file(image)?;
    Ok(
        serde_json::json!({"version":1,"snapshot":id,"bytes":manifest.bytes,"sha256":manifest.sha256,"manifest":work.join("manifest.json"),"message":"Encrypted offline disk capture complete. Keep the recovery password separately; it is not exported automatically."}),
    )
}
fn prepare(
    repo: &Path,
    snap: &str,
    device: &Path,
    work: &Path,
    secret: &[u8],
) -> Result<serde_json::Value> {
    snapshot(repo, secret, snap)?;
    let (mut disk, target) = open_device(device, true)?;
    private_dir(work)?;
    dump(
        repo,
        secret,
        snap,
        "/manifest.json",
        &work.join("manifest.json"),
    )?;
    ensure!(
        fs::metadata(work.join("manifest.json"))?.len() < 65536,
        "Invalid recovery manifest size"
    );
    let m: Manifest = serde_json::from_slice(&fs::read(work.join("manifest.json"))?)?;
    ensure!(
        m.format == "nyx-recovery-disk"
            && m.version == 1
            && m.bytes == target.bytes
            && m.source.sector == target.sector,
        "Unsupported recovery image or target geometry differs; use an exact-size target with matching logical sectors"
    );
    dump(repo, secret, snap, "/disk.img", &work.join("disk.img"))?;
    let mut image = File::open(work.join("disk.img"))?;
    ensure!(
        image.metadata()?.len() == m.bytes && hash(&mut image, m.bytes)? == m.sha256,
        "Recovery image length/hash mismatch; target has not been written"
    );
    let plan = Plan {
        version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        snapshot: snap.into(),
        device: target,
        before_sha256: hash(&mut disk, m.bytes)?,
        image_sha256: m.sha256,
        bytes: m.bytes,
        state: "prepared".into(),
    };
    atomic(&work.join("plan.json"), &plan)?;
    Ok(
        serde_json::json!({"version":1,"plan":work.join("plan.json"),"id":plan.id,"device":plan.device,"bytes":plan.bytes,"confirmation":format!("ERASE:{}:{}:{}",plan.id,plan.device.major,plan.device.minor),"message":"Review this disk identity. Apply overwrites every target byte; there is no rollback. No target writes have occurred."}),
    )
}
fn apply(path: &Path, confirm: &str) -> Result<serde_json::Value> {
    clean(path)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_file()
            && meta.uid() == 0
            && meta.mode() & 0o077 == 0
            && meta.len() < 65536,
        "Plan must be a private root-owned regular file"
    );
    let mut plan: Plan = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        plan.version == 1
            && plan.state == "prepared"
            && confirm
                == format!(
                    "ERASE:{}:{}:{}",
                    plan.id, plan.device.major, plan.device.minor
                ),
        "Confirm the exact prepared plan and target identity; interrupted/completed plans require new preparation"
    );
    let dir = path.parent().context("Missing plan directory")?;
    let (mut disk, current) = open_device(&plan.device.path, true)?;
    ensure!(
        current == plan.device && current.bytes == plan.bytes,
        "Target disk identity/geometry changed"
    );
    let mut image = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join("disk.img"))?;
    ensure!(
        image.metadata()?.is_file()
            && image.metadata()?.len() == plan.bytes
            && hash(&mut image, plan.bytes)? == plan.image_sha256,
        "Staged recovery image changed"
    );
    ensure!(
        hash(&mut disk, plan.bytes)? == plan.before_sha256,
        "Target disk changed after review; prepare again"
    );
    plan.state = "applying".into();
    atomic(path, &plan)?;
    ensure!(
        std::io::copy(&mut image, &mut disk)? == plan.bytes,
        "Incomplete target write; disk is partially overwritten"
    );
    disk.sync_all()?;
    ensure!(
        hash(&mut disk, plan.bytes)? == plan.image_sha256,
        "Target verification failed; do not boot it"
    );
    plan.state = "verified".into();
    atomic(path, &plan)?;
    Ok(
        serde_json::json!({"version":1,"state":"verified","id":plan.id,"bytes":plan.bytes,"sha256":plan.image_sha256,"message":"Every target byte matches the captured disk. Power off and disconnect the original before booting the recovered disk; duplicate filesystem/disk IDs are preserved. Boot compatibility still requires testing."}),
    )
}
fn devices() -> Result<serde_json::Value> {
    let mut a = Vec::new();
    for e in fs::read_dir("/sys/class/block")? {
        let p = e?.path();
        if !p.join("partition").exists() {
            a.push(serde_json::json!({"name":p.file_name(),"sectors_512":read(p.join("size")),"readonly":read(p.join("ro")),"model":read(p.join("device/model")),"message":"Inventory only; offline guards are checked by capture/plan/apply"}));
        }
    }
    Ok(serde_json::json!({"version":1,"devices":a}))
}
fn execute(args: &Args) -> Result<serde_json::Value> {
    if matches!(args.command, Action::Devices) {
        return devices();
    }
    environment()?;
    match &args.command {
        Action::Apply { plan, confirm } => apply(plan, confirm),
        Action::Capture {
            device,
            repo,
            work,
            initialize,
        } => capture(
            device,
            repo,
            work,
            *initialize,
            &password(args.password_stdin)?,
        ),
        Action::Plan {
            repo,
            snapshot,
            device,
            work,
        } => prepare(
            repo,
            snapshot,
            device,
            work,
            &password(args.password_stdin)?,
        ),
        _ => unreachable!(),
    }
}
fn main() {
    let args = Args::parse();
    match execute(&args) {
        Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
        Err(e) => {
            if args.json {
                println!("{}", serde_json::json!({"error":e.to_string()}))
            } else {
                eprintln!("{e:#}")
            }
            std::process::exit(1)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_ids_and_paths() {
        assert!(exact(&"a".repeat(64)).is_ok());
        assert!(exact("latest").is_err());
        assert!(clean(Path::new("/dev/../sda")).is_err());
        assert!(clean(Path::new("/dev/vda")).is_ok());
    }
    #[test]
    fn hash_short_read_refused() {
        let path = std::env::temp_dir().join(format!("nyx-recovery-test-{}", uuid::Uuid::new_v4()));
        fs::write(&path, b"abc").unwrap();
        let mut f = File::open(&path).unwrap();
        assert!(hash(&mut f, 4).is_err());
        assert_eq!(
            hash(&mut f, 3).unwrap(),
            format!("{:x}", Sha256::digest(b"abc"))
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn ordinary_file_is_never_disk() {
        let file = File::open("/etc/os-release").unwrap();
        assert!(identify(Path::new("/etc/os-release"), &file).is_err());
    }
}

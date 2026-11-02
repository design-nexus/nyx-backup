# Backup

Rust 2024 workspace. GUI: apps/backup; headless domain crates: crates/backup-*, including backup-restore.
Use pinned public Nyx shared crates and canonical nyx-design/STYLE.md.
Preserve unrelated work. Keep secrets out of argv, environment values, config and logs.
Never initialize a repository as a connectivity repair, remove uncertain Restic locks,
back up real user data in tests, enable a schedule without user action, or imply full
system recovery without VM evidence. Integration methods must enforce UID and profile
approval and expose no restore, credential retrieval or deletion.

Checks: cargo fmt --all --check; cargo test --locked -p backup-engine -p backup-restore;
cargo build --release --locked --workspace. tools/smoke.py uses a private session bus,
keyring and disposable paths. Install with ./install.sh --skip-build. Do not open or
restart the GUI after installation. Keep updates and verification focused.

Phase 2 restoration stays on the private local channel. Preserve exact snapshot/profile
ownership, explicit plan confirmation, no-follow/no-replace filesystem guards and
rollback originals. Never put diff/file contents in job journals or diagnostics.
Use tools/restore-smoke.py for disposable restore qualification; do not select real
user files. Deliver each approved phase coherently before reporting completion.

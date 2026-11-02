# Backup

Rust 2024 workspace. GUI: apps/backup; headless domain crates: crates/backup-*.
Use pinned public Nyx shared crates and canonical nyx-design/STYLE.md.
Preserve unrelated work. Keep secrets out of argv, environment values, config and logs.
Never initialize a repository as a connectivity repair, remove uncertain Restic locks,
back up real user data in tests, enable a schedule without user action, or imply full
system recovery without VM evidence. Integration methods must enforce UID and profile
approval and expose no restore, credential retrieval or deletion.

Checks: cargo fmt --all --check; cargo test --locked -p backup-engine;
cargo build --release --locked --workspace. tools/smoke.py uses a private session bus,
keyring and disposable paths. Install with ./install.sh --skip-build. Do not open or
restart the GUI after installation. Keep updates and verification focused.

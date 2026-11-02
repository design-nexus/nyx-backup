#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ ${1:-} != --skip-build ]]; then cargo build --release --locked --workspace; fi
bin="${NYX_BIN_DIR:-$HOME/.local/bin}"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/scalable/apps" "$share/dbus-1/services" "$config/systemd/user"
for name in nyx-backup nyx-backupctl nyx-backupd; do install -m755 "${CARGO_TARGET_DIR:-target}/release/$name" "$bin/$name"; done
install -m644 data/io.github.design_nexus.nyx.Backup.desktop "$share/applications/"
install -m644 data/io.github.design_nexus.nyx.Backup.svg "$share/icons/hicolor/scalable/apps/"
# Avoid interpreting installation paths as sed replacement expressions.
python3 - "$bin" "$share" "$config" <<'PY'
import pathlib,sys
binary,share,config=map(pathlib.Path,sys.argv[1:])
if any(c in str(binary) for c in '\n\r%"\\'):
 raise SystemExit('Unsupported installation path')
for source,target in [('nyx-backup.service',config/'systemd/user/nyx-backup.service'),('io.github.design_nexus.nyx.Backup1.service',share/'dbus-1/services/io.github.design_nexus.nyx.Backup1.service')]:
 text=(pathlib.Path('data')/source).read_text().replace('@BIN@',str(binary))
 if source=='nyx-backup.service':
  text=text.replace('ExecStart='+str(binary)+'/nyx-backupd','ExecStart="'+str(binary)+'/nyx-backupd"')
 target.write_text(text)
PY
systemctl --user daemon-reload
update-desktop-database "$share/applications" 2>/dev/null || true
gtk-update-icon-cache -f -t -q "$share/icons/hicolor" 2>/dev/null || true
printf 'Installed Nyx Backup, CLI, icon and on-demand service. No schedules enabled.\n'

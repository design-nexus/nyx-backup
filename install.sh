#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ ${1:-} != --skip-build ]]; then cargo build --release --locked --workspace; fi
bin="${NYX_BIN_DIR:-$HOME/.local/bin}"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
mkdir -p "$bin" "$share/applications" "$share/icons/hicolor/scalable/apps" "$share/dbus-1/services" "$share/nyx-backup" "$config/systemd/user"
for name in nyx-backup nyx-backupctl nyx-backupd nyx-backup-recovery; do install -m755 "${CARGO_TARGET_DIR:-target}/release/$name" "$bin/$name"; done
install -m755 data/nyx-backup-setup "$bin/nyx-backup-setup"
install -m755 data/nyx-backup-recovery-kit "$bin/nyx-backup-recovery-kit"
mkdir -p "$share/nyx-backup/tools" "$share/nyx-backup/recovery" "$share/nyx-backup/docs"
install -m755 tools/build-recovery.py "$share/nyx-backup/tools/"
install -m755 recovery/init "$share/nyx-backup/recovery/"
install -m644 docs/RECOVERY.md "$share/nyx-backup/docs/"
install -m644 data/setup-contract-v1.schema.json data/nyx-backup-control.json "$share/nyx-backup/"
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
 text=text.replace('ExecStart='+str(binary)+'/nyx-backupd','ExecStart="'+str(binary)+'/nyx-backupd"')
 text=text.replace('Exec='+str(binary)+'/nyx-backupd','Exec="'+str(binary)+'/nyx-backupd"')
 target.write_text(text)
PY
systemctl --user daemon-reload
# A running service keeps the old binary. Restart it only while no job runs;
# a busy one picks up the new version the next time it starts.
if systemctl --user is-active --quiet nyx-backup.service; then
  if "$bin/nyx-backupctl" status 2>/dev/null | python3 -c 'import json,sys; sys.exit(0 if json.load(sys.stdin).get("active") is None else 1)'; then
    systemctl --user restart nyx-backup.service
  else
    printf 'A backup job is running; the background service updates when it next starts.\n'
  fi
fi
# Backup opens as a small floating window. Its rule lives in its own file so the
# suite installer's rewrite of hypr/nyx.lua never drops it.
hypr="$config/hypr"
if [[ ${NYX_SKIP_INTEGRATION:-0} != 1 && -f $hypr/hyprland.lua ]] && grep -q 'o\.window' "$hypr/hyprland.lua"; then
  printf -- '-- Written by the Nyx Backup installer: Backup opens as a floating window.\no.window("^io\\\\.github\\\\.design_nexus\\\\.nyx\\\\.Backup$", { float = true, center = true, size = { 760, 560 } })\n' >"$hypr/nyx_backup.lua"
  if ! grep -q 'require("hypr.nyx_backup")' "$hypr/hyprland.lua"; then
    cp "$hypr/hyprland.lua" "$hypr/hyprland.lua.bak.$(date +%s)"
    if grep -q '^require("hypr.nyx")' "$hypr/hyprland.lua"; then
      sed -i 's/^require("hypr.nyx").*$/&\nrequire("hypr.nyx_backup") -- Nyx Backup (floating window)/' "$hypr/hyprland.lua"
    else
      printf '\nrequire("hypr.nyx_backup") -- Nyx Backup (floating window)\n' >>"$hypr/hyprland.lua"
    fi
  fi
  if command -v hyprctl >/dev/null && hyprctl reload >/dev/null 2>&1; then
    errors=$(hyprctl configerrors 2>/dev/null | grep -v '^$' || true)
    [[ -z $errors || $errors == *"no errors"* ]] || printf 'Hyprland reports: %s\n' "$errors" >&2
  fi
fi
update-desktop-database "$share/applications" 2>/dev/null || true
gtk-update-icon-cache -f -t -q "$share/icons/hicolor" 2>/dev/null || true
printf 'Installed Nyx Backup, CLI, icon and on-demand service. No schedules enabled.\n'

# VM fixture only: every device below is a disposable QEMU disk.
BEFORE=$(sha256sum /dev/vdc)
printf X | dd of=/repository/plan/disk.img bs=1 count=1 conv=notrunc
if nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN"; then exit 1; fi
[ "$BEFORE" = "$(sha256sum /dev/vdc)" ]
dd if=/dev/vda of=/repository/plan/disk.img bs=1 count=1 conv=notrunc
# Throttled virtual writes give a deterministic opportunity to stop the worker
# after the durable journal reaches applying, before the whole image is written.
nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN" > /repository/interrupted.json &
WORKER=$!
while kill -0 "$WORKER" 2>/dev/null; do
 if grep -q '"state": "applying"' /repository/plan/plan.json; then kill -STOP "$WORKER"; break; fi
done
kill -KILL "$WORKER"
wait "$WORKER" || true
grep -q '"state": "applying"' /repository/plan/plan.json
if nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN"; then exit 1; fi
printf 'fixture-recovery-password\n' | nyx-backup-recovery --password-stdin --json plan --repo /repository/restic --snapshot "$SNAP" --device /dev/vdc --work /repository/retry > /repository/retry.json
TOKEN2=$(sed -n 's/.*"confirmation": "\([^"]*\)".*/\1/p' /repository/retry.json)
nyx-backup-recovery --json apply --plan /repository/retry/plan.json --confirm "$TOKEN2" > /repository/applied.json
printf 'NYX_INTERRUPTED_RECOVERY_RETRY_OK\n'

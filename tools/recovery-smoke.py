#!/usr/bin/env python3
"""Destructive qualification entirely in QEMU guests, using disposable regular disk files."""
import hashlib,importlib.util,json,os,pathlib,shutil,subprocess,tempfile,time,sys
extended="--extended" in sys.argv
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('builder',ROOT/'tools/build-recovery.py');builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)
work=pathlib.Path(tempfile.mkdtemp(prefix='nyx-recovery-vm-',dir=ROOT/'target'));work.chmod(0o700)
base=(ROOT/'recovery/init').read_text().split("printf '\\nNyx Backup offline recovery")[0]
# Boot payload lives on the captured disk. It mounts its restored root partition
# and verifies a fixture executable, permissions and a symlink, then switches root.
boot=work/'boot-init';boot.write_text(base+'''set -eu
mkdir -p /recovered
mount -o ro /dev/vda2 /recovered
[ "$(cat /recovered/etc/nyx-fixture)" = "recovered-system-fixture" ]
[ "$(stat -c %a /recovered/etc/nyx-fixture)" = "640" ]
[ "$(readlink /recovered/fixture-link)" = "etc/nyx-fixture" ]
exec /usr/bin/switch_root /recovered /sbin/init
''')
# switch_root is supplied by util-linux rather than the reduced mkinitcpio BusyBox.
boot.chmod(0o700)
# Boot system root contains a static shell? Copy the known BusyBox and dependencies
# into its ext4 root from the recovery environment, which has those exact libraries.
root_init=work/'root-init';root_init.write_text('''#!/bin/sh
printf 'NYX_RECOVERED_SYSTEM_BOOT_OK\\n'
/usr/lib/initcpio/busybox poweroff -f
''');root_init.chmod(0o700)
bootkit=builder.build(work/'boot-kit','7.2.11-arch1-1',boot,console='ttyS0')
script=work/'recovery-init'
script.write_text(base+'''set -eu
trap 'echo NYX_RECOVERY_TEST_FAILED; poweroff -f' EXIT
mkdir -p /repository /source-root /source-efi
# All three devices here are disposable QEMU virtio disks. No host block devices.
printf 'label: gpt\\n,160M,U\\n,,L\\n' | sfdisk /dev/vda
partprobe /dev/vda
mkfs.fat -F 32 /dev/vda1
mke2fs -q -F -t ext4 /dev/vda2
mke2fs -q -F -t ext4 /dev/vdb
mount /dev/vdb /repository
mount /dev/vda1 /source-efi
mkdir -p /source-efi/EFI/BOOT
cp /fixture/boot.efi /source-efi/EFI/BOOT/BOOTX64.EFI
mount /dev/vda2 /source-root
mkdir -p /source-root/etc /source-root/sbin /source-root/usr/lib/initcpio /source-root/usr/bin /source-root/dev /source-root/proc /source-root/sys
printf 'recovered-system-fixture\\n' > /source-root/etc/nyx-fixture
chmod 640 /source-root/etc/nyx-fixture
ln -s etc/nyx-fixture /source-root/fixture-link
cp /fixture/root-init /source-root/sbin/init
cp /usr/lib/initcpio/busybox /source-root/usr/lib/initcpio/busybox
cp -a /usr/lib/*.so* /source-root/usr/lib/
mkdir -p /source-root/usr/lib64
cp -a /usr/lib64/* /source-root/usr/lib64/
ln -s usr/lib /source-root/lib
ln -s usr/lib64 /source-root/lib64
ln -s usr/bin /source-root/bin
ln -s /usr/lib/initcpio/busybox /source-root/usr/bin/sh
ln -s /usr/lib/initcpio/busybox /source-root/usr/bin/echo
chmod 755 /source-root/sbin/init
# Mounted source is rejected, and failed capture must not create a repository.
if printf 'fixture-recovery-password\\n' | nyx-backup-recovery --password-stdin --json capture --device /dev/vda --repo /repository/bad --work /repository/bad-stage --initialize; then exit 1; fi
[ ! -e /repository/bad ]
umount /source-root
umount /source-efi
sync
blockdev --setro /dev/vda
printf 'fixture-recovery-password\\n' | nyx-backup-recovery --password-stdin --json capture --device /dev/vda --repo /repository/restic --work /repository/capture --initialize > /repository/capture.json
SNAP=$(sed -n 's/.*"snapshot": "\\([a-f0-9]*\\)".*/\\1/p' /repository/capture.json)
[ "${#SNAP}" -eq 64 ]
# Wrong password and ordinary snapshots cannot touch targets or initialize replacements.
if printf 'wrong-password\\n' | nyx-backup-recovery --password-stdin --json plan --repo /repository/restic --snapshot "$SNAP" --device /dev/vdc --work /repository/wrong; then exit 1; fi
printf 'fixture-recovery-password\\n' | nyx-backup-recovery --password-stdin --json plan --repo /repository/restic --snapshot "$SNAP" --device /dev/vdc --work /repository/plan > /repository/preview.json
TOKEN=$(sed -n 's/.*"confirmation": "\\([^"]*\\)".*/\\1/p' /repository/preview.json)
[ -n "$TOKEN" ]
BEFORE=$(sha256sum /dev/vdc)
if nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm wrong; then exit 1; fi
[ "$BEFORE" = "$(sha256sum /dev/vdc)" ]
# Change-after-preview is rejected before any recovery write.
printf X | dd of=/dev/vdc bs=1 count=1 conv=notrunc
if nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN"; then exit 1; fi
printf '\\000' | dd of=/dev/vdc bs=1 count=1 conv=notrunc
nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN" > /repository/applied.json
[ "$(sha256sum /dev/vda | cut -d ' ' -f1)" = "$(sha256sum /dev/vdc | cut -d ' ' -f1)" ]
if nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN"; then exit 1; fi
printf 'NYX_RECOVERY_DISK_MATCH_OK\\n'
sync
umount /repository
trap - EXIT
poweroff -f
''');script.chmod(0o700)
if extended:
 text=script.read_text()
 marker='# Wrong password and ordinary snapshots cannot touch targets or initialize replacements.'
 text=text.replace(marker,"if printf 'fixture-recovery-password\\n' | nyx-backup-recovery --password-stdin --json plan --repo /repository/restic --snapshot \"$SNAP\" --device /dev/vdd --work /repository/mismatched; then exit 1; fi\n"+marker)
 needle='nyx-backup-recovery --json apply --plan /repository/plan/plan.json --confirm "$TOKEN" > /repository/applied.json'
 text=text.replace(needle,(ROOT/'tools/recovery-interruption.sh').read_text())
 script.write_text(text)
kit=builder.build(work/'recovery-kit','7.2.11-arch1-1',script,{'fixture/boot.efi':bootkit/'nyx-recovery.efi','fixture/root-init':root_init},fixture=True,console='ttyS0')
for name,size in [('source.raw',384),('repository.raw',1536),('target.raw',384)]+([('geometry.raw',64)] if extended else []):
 with (work/name).open('wb') as f:f.truncate(size*1024*1024)
def qemu(args,log,marker):
 command=['qemu-system-x86_64','-machine','q35','-accel','kvm','-cpu','host','-m','2048','-nographic','-monitor','none','-nic','none','-no-reboot']+args
 with log.open('wb') as f:
  p=subprocess.Popen(command,stdout=f,stderr=subprocess.STDOUT)
  try:code=p.wait(timeout=240)
  except subprocess.TimeoutExpired:p.kill();p.wait();raise AssertionError(f'VM timeout; inspect {log}')
 text=log.read_text(errors='replace');assert code==0 and marker in text,(code,str(log),text[-5000:])
qemu(['-kernel',kit/'vmlinuz','-initrd',kit/'initramfs.cpio.gz','-append','console=ttyS0 nyx.recovery=1 panic=-1']+sum((['-drive',f'file={work/name},format=raw,if=virtio'+(',throttling.bps-write=16777216' if extended and name=='target.raw' else '')] for name in ['source.raw','repository.raw','target.raw']+(['geometry.raw'] if extended else [])),[]),work/'recovery.log','NYX_RECOVERY_DISK_MATCH_OK')
if extended:assert 'NYX_INTERRUPTED_RECOVERY_RETRY_OK' in (work/'recovery.log').read_text(errors='replace')
assert hashlib.sha256((work/'source.raw').read_bytes()).digest()==hashlib.sha256((work/'target.raw').read_bytes()).digest()
varsfile=work/'OVMF_VARS.fd';shutil.copy2('/usr/share/edk2/x64/OVMF_VARS.4m.fd',varsfile)
# Real firmware boot: no -kernel/-initrd shortcut and no original/source disk attached.
qemu(['-drive','if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd','-drive',f'if=pflash,format=raw,file={varsfile}','-drive',f'file={work}/target.raw,format=raw,if=virtio'],work/'boot.log','NYX_RECOVERED_SYSTEM_BOOT_OK')
print(json.dumps({'fixture':str(work),'passed':['offline read-only encrypted capture','mounted-source refusal','wrong credential refusal','verified plan without target writes','exact confirmation refusal','changed-target refusal','whole-disk restore and SHA256 match','completed-plan reapply refusal','UEFI firmware boot of recovered disk with source absent','switch_root into restored ext4 system','contents modes and symlink preserved'],'extended':extended,'limits':['single x86-64 QEMU UEFI GPT/ext4 fixture','no real host disk or user data','no physical/hardware-change recovery','no Secure Boot/TPM/RAID/LVM qualification']},indent=2))

#!/usr/bin/env python3
"""Build a local x86-64 offline recovery kit without root or touching host boot files."""
import argparse,gzip,hashlib,json,os,pathlib,re,shutil,stat,subprocess,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
def run(*args,**kwargs):return subprocess.run(list(map(str,args)),check=True,**kwargs)
def copy_elf(tree,source):
 source=pathlib.Path(source);target=tree/source.as_posix().lstrip('/');target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,target)
 result=subprocess.run(['ldd',str(source)],text=True,capture_output=True)
 if result.returncode and 'not a dynamic executable' not in result.stderr+result.stdout:raise RuntimeError('Cannot enumerate ELF dependencies: '+str(source))
 for path in set(re.findall(r'(/[^\s()]+)',result.stdout)):
  if pathlib.Path(path).is_file():
   dest=tree/path.lstrip('/');dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,dest)
def archive(tree,dest):
 # newc supports regular files/directories/symlinks; never include host device nodes.
 with gzip.open(dest,'wb',compresslevel=3) as out:
  def entry(name,mode,data,ino):
   n=name.encode()+b'\0';fields=[ino,mode,0,0,1,0,len(data),0,0,0,0,len(n),0]
   h=b'070701'+b''.join(f'{v:08x}'.encode() for v in fields);out.write(h+n);out.write(b'\0'*((-len(h+n))%4));out.write(data);out.write(b'\0'*((-len(data))%4))
  for ino,p in enumerate(sorted(tree.rglob('*')),1):
   m=p.lstat();assert not stat.S_ISBLK(m.st_mode) and not stat.S_ISCHR(m.st_mode)
   data=os.readlink(p).encode() if p.is_symlink() else (p.read_bytes() if p.is_file() else b'')
   entry(str(p.relative_to(tree)),m.st_mode,data,ino)
  entry('TRAILER!!!',0,b'',0)
def uki(kernel,initrd,out,commandline):
 with tempfile.TemporaryDirectory(prefix='nyx-uki-') as tmp:
  tmp=pathlib.Path(tmp);(tmp/'cmdline').write_text(commandline+'\0');(tmp/'os-release').write_text('ID=nyx-recovery\nPRETTY_NAME="Nyx Backup Recovery"\n')
  # Page-aligned VMAs after the stub and previous variable-sized sections.
  stub='/usr/lib/systemd/boot/efi/linuxx64.efi.stub'
  image_base=int(re.search(r'ImageBase\s+([0-9a-fA-F]+)',subprocess.check_output(['objdump','-p',stub],text=True)).group(1),16)
  args=['objcopy'];offset=image_base+0x200000
  for name,path in [('osrel',tmp/'os-release'),('cmdline',tmp/'cmdline'),('linux',kernel),('initrd',initrd)]:
   args+=['--add-section',f'.{name}={path}','--change-section-vma',f'.{name}={offset:#x}'];offset=(offset+path.stat().st_size+0xffff)&~0xffff
  run(*args,'/usr/lib/systemd/boot/efi/linuxx64.efi.stub',out)
def build(output,kernel_version=None,init=None,extras=None,fixture=False,console="tty0"):
 output=pathlib.Path(output).resolve();output.mkdir(mode=0o700,parents=False,exist_ok=False)
 if kernel_version is None:
  candidates=sorted(p for p in pathlib.Path('/usr/lib/modules').iterdir() if (p/'vmlinuz').is_file());assert candidates,'No readable installed kernel';kernel_version=os.uname().release if (pathlib.Path('/usr/lib/modules')/os.uname().release/'vmlinuz').is_file() else candidates[-1].name
 kernel=pathlib.Path('/usr/lib/modules')/kernel_version/'vmlinuz';assert kernel.is_file()
 with tempfile.TemporaryDirectory(prefix='nyx-recovery-build-') as tmp:
  tree=pathlib.Path(tmp)
  for d in ['usr/bin','usr/lib64','proc','sys','dev','run','tmp','mnt','etc','root']: (tree/d).mkdir(parents=True,exist_ok=True)
  (tree/'bin').symlink_to('usr/bin');(tree/'lib').symlink_to('usr/lib');(tree/'lib64').symlink_to('usr/lib64')
  copy_elf(tree,'/usr/lib/initcpio/busybox')
  for applet in subprocess.check_output(['/usr/lib/initcpio/busybox','--list'],text=True).splitlines():
   (tree/'usr/bin'/applet).symlink_to('/usr/lib/initcpio/busybox')
  for binary in ['mount','modprobe','setsid','blockdev','switch_root']+(['sfdisk','mkfs.fat','mke2fs'] if fixture else []):
   p=tree/'usr/bin'/binary
   if p.is_symlink():p.unlink()
   resolved=pathlib.Path(shutil.which(binary)).resolve();copy_elf(tree,resolved)
   if not p.exists():p.symlink_to(str(resolved))
  # Helpers always execute our fixed Restic binary; no host paths are bind-mounted.
  copy_elf(tree,'/usr/bin/restic');helper=ROOT/'target/release/nyx-backup-recovery'
  # Launched from the desktop, PATH may not include the user's install folder.
  if not helper.is_file():helper=pathlib.Path(os.environ.get('NYX_BIN_DIR',pathlib.Path.home()/'.local/bin'))/'nyx-backup-recovery'
  if not helper.is_file():helper=pathlib.Path(shutil.which('nyx-backup-recovery') or '/missing/nyx-backup-recovery')
  copy_elf(tree,helper)
  original=tree/str(helper).lstrip('/');shutil.move(original,tree/'usr/bin/nyx-backup-recovery')
  modules=['virtio_pci','virtio_blk','ext4','vfat','nls_cp437','nls_iso8859_1']
  for module in modules:
   lines=subprocess.check_output(['modprobe','--show-depends','-S',kernel_version,module],text=True).splitlines()
   for line in lines:
    if line.startswith('insmod '):
     src=pathlib.Path(line.split()[1]);dst=tree/str(src).lstrip('/');dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,dst)
  src=pathlib.Path('/usr/lib/modules')/kernel_version;dest=tree/'usr/lib/modules'/kernel_version;dest.mkdir(parents=True,exist_ok=True)
  for p in src.glob('modules.*'):
   if p.is_file():shutil.copy2(p,dest/p.name)
  (tree/'etc/nyx-recovery-environment').write_text('nyx-backup-recovery-v1\n');(tree/'etc/os-release').write_text('ID=nyx-recovery\n')
  shutil.copy2(init or ROOT/'recovery/init',tree/'init');(tree/'init').chmod(0o755)
  if (ROOT/'docs/RECOVERY.md').exists():shutil.copy2(ROOT/'docs/RECOVERY.md',tree/'RECOVERY.md')
  for name,path in (extras or {}).items():
   assert not name.startswith('/') and '..' not in pathlib.Path(name).parts
   dst=tree/name;dst.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,dst)
  archive(tree,output/'initramfs.cpio.gz')
 shutil.copy2(kernel,output/'vmlinuz');cmdline=f'console={console} nyx.recovery=1 panic=-1'
 uki(output/'vmlinuz',output/'initramfs.cpio.gz',output/'nyx-recovery.efi',cmdline)
 files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in output.iterdir() if p.is_file()}
 (output/'kit.json').write_text(json.dumps({'version':1,'architecture':'x86_64','kernel':kernel_version,'secure_boot_signed':False,'sha256':files},indent=2)+'\n')
 (output/'BOOT.txt').write_text('UEFI: place nyx-recovery.efi as EFI/BOOT/BOOTX64.EFI on a FAT EFI system partition. This file is unsigned; Secure Boot requires your own signing workflow.\nQEMU: qemu-system-x86_64 -m 2048 -kernel vmlinuz -initrd initramfs.cpio.gz -append "console=ttyS0 nyx.recovery=1" -nographic -nic none\nNo disks are attached by this command. Add only explicitly selected offline disks.\n')
 return output
if __name__=='__main__':
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',required=True,type=pathlib.Path);p.add_argument('--kernel-version');p.add_argument('--console',choices=['tty0','ttyS0'],default='tty0');a=p.parse_args();print(build(a.output,a.kernel_version,console=a.console))

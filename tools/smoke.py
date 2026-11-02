#!/usr/bin/env python3
"""Disposable integration proof. Run under dbus-run-session; never reads user files."""
import json,os,pathlib,subprocess,sys,tempfile,time,stat,fcntl
repo=pathlib.Path(__file__).resolve().parents[1]
# Only fixture source paths enter profiles. Secret Service runs on this private bus.
root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-backup-smoke-'))
for name in ['config','state','cache','data','home','runtime','source/.cache','source/.ssh','source/build','source/repository']:(root/name).mkdir(parents=True,exist_ok=True)
os.chmod(root/'runtime',0o700)
os.environ.update(HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_STATE_HOME=str(root/'state'),XDG_CACHE_HOME=str(root/'cache'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(root/'runtime'))
for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK']:os.environ.pop(key,None)
ctl=repo/'target/release/nyx-backupctl'; daemon=repo/'target/release/nyx-backupd'
def cmd(*args,input=None,ok=True):
 p=subprocess.run([str(x) for x in args],input=input,text=True,capture_output=True,timeout=60)
 if ok and p.returncode:raise AssertionError(f'{args}: {p.stderr} {p.stdout}')
 return p

def call(*args,ok=True,input=None):
 p=cmd(ctl,*args,'--json',input=input,ok=ok)
 try:v=json.loads(p.stdout)
 except Exception:raise AssertionError(p.stdout+p.stderr)
 return v

def wait(job):
 for _ in range(200):
  j=call('job',job,ok=False)
  if j.get('state')!='running':return j
  time.sleep(.1)
 raise AssertionError('Job did not finish')

config=f'''version = 1
notifications = false
[[destinations]]
id = "fixture"
name = "Fixture"
type = "local"
path = "{root}/source/repository"
[[profiles]]
id = "fixture"
name = "Fixture"
destination = "fixture"
sources = ["{root}/source"]
exclusions = ["*.skip"]
approved = true
keep_last = 1
schedule = "daily"
'''
# Select a path containing .. so the native backup also proves normalized exclusions.
config=config.replace(f'sources = ["{root}/source"]', f'sources = ["{root}/source/build/.."]')
(root/'fixture.toml').write_text(config)
(root/'source/keep.txt').write_text('initial\n');os.chmod(root/'source/keep.txt',0o640)
(root/'source/build/valuable').write_text('build folders are not disposable')
(root/'source/.cache/skip').write_text('cache');(root/'source/.ssh/id_ed25519').write_text('dummy-key-not-real')
(root/'source/ignored.skip').write_text('excluded')
keyring=cmd('gnome-keyring-daemon','--unlock','--components=secrets',input='disposable-fixture-unlock\n')
call('config','import',root/'fixture.toml')
call('password','fixture','--stdin',input='fixture-only-recovery-password\n')
# Verify encrypted Rclone config using Rclone's own decoder, without a live NAS.
smb_config=config+'''
[[destinations]]
id = "smb-fixture"
name = "SMB fixture (no endpoint)"
type = "smb"
server = "127.0.0.1"
share = "fixture"
path = "restic"
username = "fixture"
domain = ""
'''
(root/'fixture.toml').write_text(smb_config)
call('config','import',root/'fixture.toml')
call('smb-password','smb-fixture','--stdin',input='fixture-only-nas-password\n')
encrypted=root/'config/nyx-backup/rclone/smb-fixture.conf'
assert encrypted.read_text().startswith('RCLONE_ENCRYPT_V0:') and 'fixture-only-nas-password' not in encrypted.read_text()
decoded=cmd('rclone','--config',encrypted,'--password-command','secret-tool lookup application nyx-backup account smb-fixture kind rclone','--ask-password=false','config','dump')
assert json.loads(decoded.stdout)['nas']['type']=='smb'
# Failed SMB credential updates must preserve the existing encrypted config/key.
original_ciphertext=encrypted.read_bytes()
original_unlock=cmd('secret-tool','lookup','application','nyx-backup','account','smb-fixture','kind','rclone').stdout
fault_bin=root/'fault-bin';fault_bin.mkdir()
(fault_bin/'rclone').write_text('#!/bin/sh\nexit 1\n');(fault_bin/'rclone').chmod(0o700)
original_path=os.environ['PATH'];os.environ['PATH']=str(fault_bin)+os.pathsep+original_path
failed_update=call('smb-password','smb-fixture','--stdin',input='replacement-fixture-password\n',ok=False)
os.environ['PATH']=original_path
assert 'error' in failed_update and encrypted.read_bytes()==original_ciphertext
assert cmd('secret-tool','lookup','application','nyx-backup','account','smb-fixture','kind','rclone').stdout==original_unlock
assert cmd('rclone','--config',encrypted,'--password-command','secret-tool lookup application nyx-backup account smb-fixture kind rclone','--ask-password=false','config','dump').stdout==decoded.stdout


# The D-Bus activation file supplies an isolated service without touching user units.
activation=root/'data/dbus-1/services';activation.mkdir(parents=True)
(activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={daemon}\n')
# A bus may cache activation directories before XDG_DATA_HOME changes. Spawn here if so.
worker=subprocess.Popen([str(daemon)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
try:
 for _ in range(100):
  p=cmd(ctl,'capabilities',ok=False)
  p=cmd('gdbus','call','--session','--dest','io.github.design_nexus.nyx.Backup1','--object-path','/io/github/design_nexus/nyx/Backup1','--method','io.github.design_nexus.nyx.Backup1.Capabilities',ok=False)
  if p.returncode==0:break
  if worker.poll() is not None:raise AssertionError(worker.stderr.read())
  time.sleep(.05)
 else:raise AssertionError('Service failed to register')
 preview=call('preview','fixture');assert preview['files']==2,preview
 init=call('destination','fixture','init','fixture','--confirm')
 assert init['state']=='success',init
 first=call('backup','fixture','--wait');assert first['state']=='success',first
 (root/'source/keep.txt').write_text('incremental\n');(root/'source/new.txt').write_text('new\n')
 second=call('backup','fixture','--wait');assert second['state']=='success',second
 history=call('history','fixture');assert len(history['result'])==2,history
 verified=call('verify','fixture','--full','--wait');assert verified['state']=='success',verified
 call('stats','fixture')
 password_command='secret-tool lookup application nyx-backup account fixture kind repository'
 native=['restic','--repo',root/'source/repository','--password-command',password_command]
 cmd(*native,'restore','latest','--target',root/'restored')
 restored=root/'restored'/str(root/'source').lstrip('/')
 assert (restored/'keep.txt').read_text()=='incremental\n'
 assert stat.S_IMODE((restored/'keep.txt').stat().st_mode)==0o640
 assert (restored/'build/valuable').exists()
 assert not (restored/'.cache').exists() and not (restored/'.ssh').exists() and not (restored/'ignored.skip').exists() and not (restored/'repository').exists()
 retention=call('retention','fixture');assert retention['state']=='success'
 # Two tagged snapshots still exist after a retention preview.
 assert len(call('history','fixture')['result'])==2
 call('retention','fixture','--apply','--confirm-profile','fixture')
 assert len(call('history','fixture')['result'])==1
 # A second caller cannot enter while the application lock is held.
 lockpath=root/'state/nyx-backup/worker.lock'
 with lockpath.open('w') as lock:
  fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
  refused=call('backup','fixture',ok=False);assert 'error' in refused,refused
 # Real permission failure is distinct from success.
 (root/'source/unreadable').write_text('permission fixture');os.chmod(root/'source/unreadable',0)
 partial=call('backup','fixture','--wait',ok=False);assert partial['state']=='incomplete',partial
 os.chmod(root/'source/unreadable',0o600)
 # Real Restic cancellation, including its process group.
 with (root/'source/large').open('wb') as f:
  for _ in range(8):f.write(os.urandom(4*1024*1024))
 job=call('backup','fixture')['job'];call('cancel',job);cancelled=wait(job);assert cancelled['state']=='cancelled',cancelled
 # API cannot initialize, delete or execute arbitrary commands.
 rejected=cmd('gdbus','call','--session','--dest','io.github.design_nexus.nyx.Backup1','--object-path','/io/github/design_nexus/nyx/Backup1','--method','io.github.design_nexus.nyx.Backup1.Start','fixture','retention-apply',ok=False)
 assert rejected.returncode!=0
 call('approve','fixture','--revoke');assert 'error' in call('backup','fixture',ok=False)
 call('approve','fixture')
 # Persisted jobs are reconciled after the worker disappears.
 job=call('backup','fixture')['job'];time.sleep(.1)
 worker.terminate();worker.wait(timeout=10)
 # Restic is its own process group; emulate systemd's KillMode=control-group for the private-bus harness.
 for proc in pathlib.Path('/proc').iterdir():
  if proc.name.isdigit():
   try:
    command=(proc/'cmdline').read_bytes()
    if b'restic\0' in command and str(root).encode() in command:os.killpg(int(proc.name),15)
   except (OSError,ProcessLookupError):pass
 worker=subprocess.Popen([str(daemon)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
 for _ in range(100):
  p=cmd(ctl,'job',job,'--json',ok=False)
  if p.stdout and json.loads(p.stdout).get('state')=='interrupted':break
  time.sleep(.1)
 else:raise AssertionError('Interrupted worker was not reconciled')
 # Lock the fixture Secret Service collection. No user's keyring is involved.
 cmd('gdbus','call','--session','--dest','org.freedesktop.secrets','--object-path','/org/freedesktop/secrets','--method','org.freedesktop.Secret.Service.Lock',"[objectpath '/org/freedesktop/secrets/collection/login']",ok=False)
 lock_check_started=time.monotonic()
 locked=call('destination','fixture','validate','fixture',ok=False)
 assert locked['state']=='failed' and 'locked' in locked['message'].lower(),locked
 assert time.monotonic()-lock_check_started<5, 'Locked credential lookup must not wait for an unlock prompt'
 state=''.join(p.read_text() for p in (root/'state/nyx-backup/jobs').glob('*.json'))+(root/'config/nyx-backup/config.toml').read_text()
 assert 'fixture-only-recovery-password' not in state and 'disposable-fixture-unlock' not in state
 print(json.dumps({'fixture':str(root),'passed':['initial and incremental snapshots','shared selection rules','native restore contents and permissions','metadata/full verification','retention preview and scoped deletion','worker concurrency','partial read result','cancellation','API restrictions','approval enforcement','interrupted-job reconciliation','locked credentials','no secrets in state','normalized source exclusions','failed SMB credential update preserves access']},indent=2))
finally:
 worker.terminate()
 try:worker.wait(timeout=10)
 except subprocess.TimeoutExpired:worker.kill()

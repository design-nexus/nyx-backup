#!/usr/bin/env python3
"""Qualify SFTP-only accounts using private keys, loopback sshd and disposable data."""
import json,os,pathlib,pwd,shlex,socket,stat,subprocess,sys,tempfile,time
repo=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-backup-sftp-'))
 env=os.environ.copy();env['NYX_TEST_ROOT']=str(root)
 for key,name in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache')]:
  (root/name).mkdir();env[key]=str(root/name)
 for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK']:env.pop(key,None)
 raise SystemExit(subprocess.call(['dbus-run-session','--',sys.executable,__file__,'inner'],env=env))
root=pathlib.Path(os.environ['NYX_TEST_ROOT']);ctl=repo/'target/release/nyx-backupctl';daemon=repo/'target/release/nyx-backupd'
def run(*args,input=None,check=True):
 p=subprocess.run(list(map(str,args)),text=True,input=input,capture_output=True,timeout=45)
 if check and p.returncode:raise AssertionError(f'{args}: {p.stdout} {p.stderr}')
 return p

def call(*args,input=None,check=True):return json.loads(run(ctl,*args,'--json',input=input,check=check).stdout)
for name in ['source','remote','bin']:(root/name).mkdir()
for name in ['host','client']:run('ssh-keygen','-q','-t','ed25519','-N','','-f',root/name)
(root/'authorized_keys').write_text((root/'client.pub').read_text())
with socket.socket() as sock:
 sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
(root/'sshd.conf').write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root}/host
PidFile {root}/sshd.pid
AuthorizedKeysFile {root}/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
ForceCommand internal-sftp
Subsystem sftp internal-sftp
AllowUsers {pwd.getpwuid(os.getuid()).pw_name}
''')
host_public=(root/'host.pub').read_text().split()
(root/'known_hosts').write_text(f'[127.0.0.1]:{port} {host_public[0]} {host_public[1]}\n')
(root/'ssh.conf').write_text(f'''Host nyx-backup-fixture
 HostName 127.0.0.1
 Port {port}
 User {pwd.getpwuid(os.getuid()).pw_name}
 IdentityFile {root}/client
 IdentitiesOnly yes
 IdentityAgent none
 UserKnownHostsFile {root}/known_hosts
 StrictHostKeyChecking yes
''')
# OpenSSH reads ~/.ssh using the passwd home, not $HOME. These fixture wrappers
# inject a private existing config without changing the user's SSH configuration.
for name in ['ssh','sftp']:
 path=root/'bin'/name
 path.write_text(f'#!/bin/sh\nexec /usr/bin/{name} -F {shlex.quote(str(root/"ssh.conf"))} "$@"\n');path.chmod(0o700)
os.environ['PATH']=str(root/'bin')+os.pathsep+os.environ['PATH']
server_log=(root/'sshd.log').open('w')
server=subprocess.Popen(['/usr/bin/sshd','-D','-e','-f',str(root/'sshd.conf')],stdout=subprocess.DEVNULL,stderr=server_log)
worker=None
try:
 for _ in range(50):
  if server.poll() is not None:raise AssertionError((root/'sshd.log').read_text())
  try:
   with socket.create_connection(('127.0.0.1',port),timeout=.2):break
  except OSError:time.sleep(.05)
 (root/'source/file').write_text('SFTP-only fixture\n');os.chmod(root/'source/file',0o640)
 configuration=f'''version = 1
notifications = false
[[destinations]]
id = "sftp-fixture"
name = "SFTP-only fixture"
type = "sftp"
host = "nyx-backup-fixture"
path = "{root}/remote/repository"
[[profiles]]
id = "sftp-fixture"
name = "SFTP-only fixture"
destination = "sftp-fixture"
sources = ["{root}/source"]
approved = true
keep_last = 2
'''
 (root/'profile.toml').write_text(configuration)
 run('gnome-keyring-daemon','--unlock','--components=secrets',input='ephemeral-sftp-keyring\n')
 call('config','import',root/'profile.toml');call('password','sftp-fixture','--stdin',input='ephemeral-sftp-repository-password\n')
 # The previous shell probe fails for this correctly configured SFTP-only server.
 shell=run('ssh','-oBatchMode=yes','nyx-backup-fixture','true',input='',check=False)
 assert shell.returncode!=0
 assert call('destination','sftp-fixture','test')['connected']
 assert not (root/'remote/repository').exists()
 initialized=call('destination','sftp-fixture','init','sftp-fixture','--confirm');assert initialized['state']=='success',initialized
 worker=subprocess.Popen([str(daemon)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,text=True)
 for _ in range(50):
  status=run(ctl,'status','--json',check=False)
  if status.returncode==0:break
  if worker.poll() is not None:raise AssertionError(worker.stderr.read())
  time.sleep(.05)
 backup=call('backup','sftp-fixture','--wait');assert backup['state']=='success',backup
 assert call('verify','sftp-fixture','--full','--wait')['state']=='success'
 assert len(call('history','sftp-fixture')['result'])==1
 password_command='secret-tool lookup application nyx-backup account sftp-fixture kind repository'
 run('restic','--repo',f'sftp:nyx-backup-fixture:{root}/remote/repository','--password-command',password_command,'restore','latest','--target',root/'restored')
 restored=root/'restored'/str(root/'source/file').lstrip('/')
 assert restored.read_text()=='SFTP-only fixture\n' and stat.S_IMODE(restored.stat().st_mode)==0o640
 # Validation of a missing path must not create a replacement repository.
 (root/'profile.toml').write_text(configuration.replace('/remote/repository','/remote/absent'))
 call('config','import',root/'profile.toml')
 failed=call('destination','sftp-fixture','validate','sftp-fixture',check=False)
 assert failed['state']=='failed' and not (root/'remote/absent').exists(),failed
 server.terminate();server.wait(timeout=5)
 disconnected=call('destination','sftp-fixture','test',check=False)
 assert 'error' in disconnected and not (root/'remote/absent').exists()
 print(json.dumps({'fixture':str(root),'passed':['SFTP-only connection probe without shell access','existing SSH configuration and host-key verification','explicit repository initialization','encrypted backup','full-data verification','snapshot history','native Restic restore contents and mode 0640','missing repository validation never initializes','disconnected transport reports failure']},indent=2))
finally:
 if worker is not None:
  worker.terminate()
  try:worker.wait(timeout=5)
  except subprocess.TimeoutExpired:worker.kill()
 if server.poll() is None:server.terminate();server.wait(timeout=5)
 server_log.close()

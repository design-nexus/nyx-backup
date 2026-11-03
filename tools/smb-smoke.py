#!/usr/bin/env python3
"""SMB2 protocol qualification against a disposable loopback endpoint.
Uses an extracted official Samba package in a private user/mount namespace; only loopback is selected.
"""
import json,os,pathlib,signal,socket,subprocess,sys,tempfile,time,shutil
REPO=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-smb-smoke-'));env=os.environ.copy();env['NYX_SMB_ROOT']=str(root)
 for k,n in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
  (root/n).mkdir(mode=0o700);env[k]=str(root/n)
 for k in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK','DBUS_SESSION_BUS_ADDRESS']:env.pop(k,None)
 # Only a loopback endpoint is selected; no real NAS configuration is used.
 command=['dbus-run-session','--','/usr/bin/python3',str(pathlib.Path(__file__).resolve()),'inner']
 raise SystemExit(subprocess.call(command,env=env))
root=pathlib.Path(os.environ['NYX_SMB_ROOT']);ctl=REPO/'target/release/nyx-backupctl'
if sys.argv[1]=='server':
 # Root exists only in a new user/mount namespace. Bind mounts are private;
 # the host passwd, group, module directories and mount table are unchanged.
 stage=REPO/'target/qualification-tools/samba/usr'
 for source,target in [(root/'modules','/usr/lib/samba'),(root/'passwd','/etc/passwd'),(root/'group','/etc/group')]:subprocess.run(['mount','--bind',str(source),target],check=True)
 env=os.environ.copy();env['LD_LIBRARY_PATH']=str(root/'modules')+os.pathsep+str(stage/'lib')
 subprocess.run([str(stage/'bin/pdbedit'),'--configfile',str(root/'smb.conf'),'-t','-a','-u','fixture'],input='fixture-smb-password\nfixture-smb-password\n',text=True,env=env,stdout=subprocess.DEVNULL,check=True)
 os.execvpe(str(stage/'bin/smbd'),[str(stage/'bin/smbd'),'--foreground','--no-process-group','--log-basename',str(root),'--configfile',str(root/'smb.conf')],env)

def cmd(*a,input=None,ok=True):
 p=subprocess.run(list(map(str,a)),input=input,text=True,capture_output=True,timeout=60)
 if ok and p.returncode:
  extra=''.join(p.read_text() for p in [root/'native-errors.log',root/'native-stderr.log',root/'native-restic.log'] if p.exists())
  for secret in ['fixture-smb-password','fixture-recovery-password']:extra=extra.replace(secret,'[fixture secret redacted]')
  raise AssertionError((a,p.stdout,p.stderr,extra))
 return p

def cli(*a,input=None,ok=True):return json.loads(cmd(ctl,*a,'--json',input=input,ok=ok).stdout)
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
# Test-only port injection, like the existing SFTP fixture's SSH-config wrapper.
# No credentials or alternate server is passed through environment values.
(root/'bin').mkdir();wrapper=root/'bin/rclone'
wrapper.write_text(chr(10).join(['#!/usr/bin/python3','import os,sys,json,pathlib',f'root=pathlib.Path({str(root)!r})',"with (root/'rclone-calls.jsonl').open('a') as f:f.write(json.dumps(sys.argv[1:])+chr(10))",f"os.execv('/usr/bin/rclone',['rclone','--smb-port','{port}','--log-level','ERROR','--log-file',str(root/'native-errors.log')]+sys.argv[1:])",'']));wrapper.chmod(0o700);os.environ['PATH']=str(root/'bin')+os.pathsep+os.environ['PATH']
restic_wrapper=root/'bin/restic';restic_wrapper.write_text(chr(10).join(['#!/bin/sh',f'exec /usr/bin/restic "$@" 2>> {root}/native-restic.log','']));restic_wrapper.chmod(0o700)
share=root/'share';(share/'restic').mkdir(parents=True);source=root/'source';source.mkdir();(source/'example.txt').write_text('initial SMB fixture\n');(source/'example.txt').chmod(0o640)
# Use Samba's real server with a private single-user passdb; no host config is read.
smbd=REPO/'target/qualification-tools/samba/usr/bin/smbd'
assert smbd.is_file(),'Extract the official Samba package into target/qualification-tools/samba first'
shutil.copytree('/usr/lib/samba',root/'modules',symlinks=True)
shutil.copytree(smbd.parents[1]/'lib/samba',root/'modules',symlinks=True,dirs_exist_ok=True)
username='fixture'
(root/'passwd').write_text('root:x:0:0:root:/root:/bin/sh\nfixture:x:0:0:fixture:/root:/bin/sh\nnobody:x:65534:65534:nobody:/:/bin/false\n')
(root/'group').write_text('root:x:0:fixture\nnogroup:x:65534:\n')
passdb=root/'passdb.tdb'
for d in ['private','lock','state-samba','cache-samba','pid','ncalrpc']:(root/d).mkdir(mode=0o700)
samba_config=root/'smb.conf';samba_config.write_text(f"""[global]
workgroup = WORKGROUP
server role = standalone server
security = user
passdb backend = tdbsam:{passdb}
private dir = {root}/private
lock directory = {root}/lock
state directory = {root}/state-samba
cache directory = {root}/cache-samba
pid directory = {root}/pid
ncalrpc dir = {root}/ncalrpc
log file = {root}/samba.log
interfaces = 127.0.0.1
bind interfaces only = yes
smb ports = {port}
disable netbios = yes
load printers = no
printing = bsd
printcap name = /dev/null
map to guest = Never
server min protocol = SMB2
[fixture]
path = {share}
read only = no
valid users = {username}
""")
log=(root/'server.log').open('w')
server=subprocess.Popen(['unshare','-Ur','--map-auto','--setgroups=allow','-m','--propagation','private','/usr/bin/python3',str(pathlib.Path(__file__).resolve()),'server'],stdout=log,stderr=log,start_new_session=True)
for _ in range(100):
 assert server.poll() is None,(root/'server.log').read_text()
 try:
  with socket.create_connection(('127.0.0.1',port),.1):break
 except OSError:time.sleep(.05)
activation=root/'data/dbus-1/services';activation.mkdir(parents=True);(activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={REPO}/target/release/nyx-backupd\n')
config=f'''version=1
notifications=false
[[destinations]]
id="smb"
name="Disposable SMB2 endpoint"
type="smb"
server="127.0.0.1"
share="fixture"
path="restic"
username="{username}"
domain=""
[[profiles]]
id="fixture"
name="SMB fixture"
destination="smb"
sources=["{source}"]
approved=true
keep_last=2
'''
file=root/'config.toml';file.write_text(config)
daemon=subprocess.Popen([str(REPO/'target/release/nyx-backupd')],stdout=log,stderr=log);pid=daemon.pid
for _ in range(100):
 state=cmd('gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.NameHasOwner','io.github.design_nexus.nyx.Backup1').stdout
 if 'true' in state:break
 time.sleep(.05)
try:
 cmd('gnome-keyring-daemon','--unlock','--components=secrets',input='fixture-keyring\n');cli('config','import',file);cli('password','smb','--stdin',input='fixture-recovery-password\n');cli('smb-password','smb','--stdin',input='fixture-smb-password\n')
 cli('destination','smb','test');failed=cli('destination','smb','validate','fixture',ok=False);assert failed['state']=='failed' and not (share/'restic/config').exists(),failed
 cmd('rclone','--config',root/'config/nyx-backup/rclone/smb.conf','--password-command','secret-tool lookup application nyx-backup account smb kind rclone','--ask-password=false','rcat','nas:fixture/restic/protocol-probe',input='fixture write test')
 cmd('rclone','--config',root/'config/nyx-backup/rclone/smb.conf','--password-command','secret-tool lookup application nyx-backup account smb kind rclone','--ask-password=false','deletefile','nas:fixture/restic/protocol-probe')
 assert cli('destination','smb','init','fixture','--confirm')['state']=='success'
 raw=cmd('gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.GetConnectionUnixProcessID','io.github.design_nexus.nyx.Backup1').stdout;pid=int(raw.split('uint32 ')[1].split(',')[0].split(')')[0])
 initial=cli('backup','fixture','--wait');assert initial['state']=='success',initial
 (source/'example.txt').write_text('incremental SMB fixture\n');incremental=cli('backup','fixture','--wait');assert incremental['state']=='success',incremental
 assert cli('verify','fixture','--full','--wait')['state']=='success'
 snapshot=incremental['result']['snapshot_id'];out=root/'restore';planned=cli('restore','plan','fixture',snapshot,'--path',source/'example.txt','--target',out,'--wait');assert planned['state']=='success',planned
 plan=planned['result']['plan'];assert cli('restore','apply','fixture',plan,'--confirm',plan,'--wait')['state']=='success'
 restored=out/str(source/'example.txt').lstrip('/');assert restored.read_bytes()==(source/'example.txt').read_bytes() and restored.stat().st_mode&0o777==0o640
 cli('smb-password','smb','--stdin',input='wrong-fixture-password\n');assert 'error' in cli('destination','smb','test',ok=False)
 cli('smb-password','smb','--stdin',input='fixture-smb-password\n');cli('destination','smb','test')
 server.terminate();server.wait();assert 'error' in cli('destination','smb','test',ok=False)
 for folder in [root/'config/nyx-backup',root/'state/nyx-backup/jobs']:
  for p in folder.rglob('*'):
   if p.is_file():
    data=p.read_bytes();assert b'fixture-smb-password' not in data and b'fixture-recovery-password' not in data,p
 print(json.dumps({'fixture':str(root),'passed':['loopback endpoint and fixture-only port wrapper','Samba credentialed connection','missing repository never initializes','explicit repository initialization','initial and incremental encrypted backups','full-data verification','selective restore contents and mode 0640','wrong password refusal','disconnected server refusal','no credential values in persistent app config or journals'],'limits':['Samba 4.25.0 loopback endpoint; physical NAS vendor firmware unverified','no host root or system mount definition changes']},indent=2))
finally:
 if pid:
  try:os.kill(pid,signal.SIGTERM)
  except ProcessLookupError:pass
 if server.poll() is None:server.terminate();server.wait()

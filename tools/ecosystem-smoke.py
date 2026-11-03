#!/usr/bin/env python3
"""Private-bus, signed loopback S3, grant boundary, GitHub-stub and timing qualification."""
import base64,copy,json,os,pathlib,socket,subprocess,sys,tempfile,time
REPO=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-ecosystem-'));env=os.environ.copy();env['NYX_ECOSYSTEM_ROOT']=str(root)
 for key,name in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
  (root/name).mkdir(mode=0o700);env[key]=str(root/name)
 for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK','GH_TOKEN','GITHUB_TOKEN']:env.pop(key,None)
 (root/'bin').mkdir();env['PATH']=str(root/'bin')+os.pathsep+env['PATH']
 env['GH_TOKEN']='inherited-token-must-be-stripped' # Synthetic adversarial inherited override.
 # Only this script's existing ephemeral fake account is reachable through this gh stub.
 (root/'bin/gh').write_text(f'''#!{sys.executable}
import base64,json,os,pathlib,sys
root=pathlib.Path({str(root)!r});a=sys.argv[1:]
assert 'GH_TOKEN' not in os.environ and '--hostname' in a and a[a.index('--hostname')+1]=='github.com'
method=a[a.index('--method')+1];endpoint=a[a.index('--method')+2]
state=json.loads((root/'gh-state.json').read_text()) if (root/'gh-state.json').exists() else {{'private':True,'sha':None,'content':None}}
status=200
if endpoint=='repos/fixture/settings':data={{'id':123,'private':state['private'],'archived':False,'permissions':{{'push':True}}}}
elif '/branches/' in endpoint:data={{'name':'main'}}
elif '/contents/' in endpoint and method=='GET':
 if state['sha'] is None:status=404;data={{'message':'not found'}}
 else:data={{'type':'file','size':len(base64.b64decode(state['content'])),'sha':state['sha'],'content':state['content']}}
elif '/contents/' in endpoint and method=='PUT':
 body=json.load(sys.stdin);assert body.get('sha')==state['sha'];state.update(sha='new-commit-blob',content=body['content']);(root/'gh-state.json').write_text(json.dumps(state));data={{'commit':{{'sha':'fixture-commit'}}}}
else:raise SystemExit('Unexpected GitHub operation')
print('HTTP/2.0 '+str(status)+' Status\\nContent-Type: application/json\\n\\n'+json.dumps(data));raise SystemExit(0 if status<400 else 1)
''');(root/'bin/gh').chmod(0o700)
 (root/'bin/restic').write_text(f'''#!{sys.executable}
import os,pathlib,sys,time
if pathlib.Path({str(root/'stall')!r}).exists() and 'backup' in sys.argv[1:]:time.sleep(60)
os.execv('/usr/bin/restic',['restic']+sys.argv[1:])
''');(root/'bin/restic').chmod(0o700)
 activation=root/'data/dbus-1/services';activation.mkdir(parents=True);(activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={REPO}/target/release/nyx-backupd\n')
 raise SystemExit(subprocess.call(['dbus-run-session','--','/usr/bin/python3',__file__,'inner'],env=env))
import gi
gi.require_version('Gio','2.0');from gi.repository import Gio,GLib
root=pathlib.Path(os.environ['NYX_ECOSYSTEM_ROOT']);ctl=REPO/'target/release/nyx-backupctl'
def cmd(*a,input=None,ok=True):
 p=subprocess.run(list(map(str,a)),input=input,text=True,capture_output=True,timeout=90)
 if ok and p.returncode:raise AssertionError((a,p.stdout,p.stderr))
 return p

def cli(*a,input=None,ok=True):return json.loads(cmd(ctl,*a,'--json',input=input,ok=ok).stdout)
def wait(job):
 for _ in range(500):
  j=cli('job',job,ok=False)
  if j['state']!='running':return j
  time.sleep(.05)
 raise AssertionError('Job did not finish')
source=root/'source';source.mkdir();(source/'keep.txt').write_text('original fixture\n');(source/'keep.txt').chmod(0o640)
(source/'payload.bin').write_bytes(os.urandom(32*1024*1024))
for i in range(200):(source/f'item-{i}.txt').write_text('small-file fixture\n'*10)
objects=root/'objects';objects.mkdir();(objects/'fixture-bucket').mkdir()
with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
proxy=root/'auth-proxy';proxy.write_text(f'''#!{sys.executable}
import json,sys
v=json.load(sys.stdin)
if v.get('user')!='fixture-access':raise SystemExit(1)
json.dump({{'type':'local','_root':{str(objects)!r},'_secret_access_key':'fixture-secret-key'}},sys.stdout)
''');proxy.chmod(0o700)
log=(root/'s3-server.log').open('w');server=subprocess.Popen(['rclone','serve','s3','--addr',f'127.0.0.1:{port}','--auth-proxy',str(proxy),'--config','/dev/null'],stdout=log,stderr=log)
for _ in range(100):
 assert server.poll() is None,(root/'s3-server.log').read_text()
 try:
  with socket.create_connection(('127.0.0.1',port),.1):break
 except OSError:time.sleep(.05)
config=f'''version=1
notifications=false
[[destinations]]
id="cloud"
name="Fixture cloud"
type="s3"
endpoint="http://127.0.0.1:{port}"
region="us-east-1"
bucket="fixture-bucket"
path="restic"
[[profiles]]
id="fixture"
name="Fixture"
destination="cloud"
sources=["{source}"]
approved=true
include_secrets=true
keep_last=2
'''
file=root/'config.toml';file.write_text(config)
cmd('gnome-keyring-daemon','--unlock','--components=secrets',input='ephemeral-ecosystem-keyring\n');cli('config','import',file);cli('password','cloud','--stdin',input='fixture-recovery-password\n')
cli('s3-credentials','cloud','--stdin',input='fixture-access\nfixture-secret-key\n')
bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','RequestName',GLib.Variant('(su)',('io.github.design_nexus.nyx.ControlCenter',4)),None,Gio.DBusCallFlags.NONE,5000,None)
other=Gio.DBusConnection.new_for_address_sync(os.environ['DBUS_SESSION_BUS_ADDRESS'],Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT|Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,None,None)
def control(name,payload,connection=bus):
 v=connection.call_sync('io.github.design_nexus.nyx.Backup1','/io/github/design_nexus/nyx/Backup1','io.github.design_nexus.nyx.Control','Call',GLib.Variant('(ss)',(name,json.dumps(payload))),GLib.VariantType('(s)'),Gio.DBusCallFlags.NONE,10000,None);return json.loads(v.unpack()[0])
pid=None
try:
 cli('destination','cloud','test')
 missing=cli('destination','cloud','validate','fixture',ok=False);assert missing['state']=='failed' and not (objects/'fixture-bucket/restic/config').exists(),missing
 assert cli('destination','cloud','init','fixture','--confirm')['state']=='success'
 pid=bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('io.github.design_nexus.nyx.Backup1',)),None,Gio.DBusCallFlags.NONE,5000,None).unpack()[0]
 readonly=cli('integrations','grant','io.github.design_nexus.nyx.ControlCenter','fixture')
 assert 'error' in control('backup.start',{'grant':readonly['id']})
 cli('integrations','revoke',readonly['id'])
 g=cli('integrations','grant','io.github.design_nexus.nyx.ControlCenter','fixture','--actions','status,backup,verify,cancel')
 assert 'ok' in control('backup.status',{'grant':g['id']})
 assert 'error' in control('backup.status',{'grant':g['id']},other)
 assert 'error' in control('restore.apply',{'grant':g['id']})
 assert 'error' in control('backup.start',{'grant':g['id'],'command':'arbitrary'})
 # Closing the legacy mutation path prevents bypassing grants through Start.
 legacy=cmd('gdbus','call','--session','--dest','io.github.design_nexus.nyx.Backup1','--object-path','/io/github/design_nexus/nyx/Backup1','--method','io.github.design_nexus.nyx.Backup1.Start','fixture','backup',ok=False);assert legacy.returncode!=0
 start=time.monotonic();initial=wait(control('backup.start',{'grant':g['id']})['ok']['job']);initial_seconds=time.monotonic()-start;assert initial['state']=='success',initial
 (source/'keep.txt').write_text('incremental fixture\n');start=time.monotonic();incremental=cli('backup','fixture','--wait');incremental_seconds=time.monotonic()-start;assert incremental['state']=='success',incremental
 assert incremental['result']['data_added']<initial['result']['data_added']
 start=time.monotonic();verified=wait(control('backup.verify',{'grant':g['id']})['ok']['job']);metadata_verify_seconds=time.monotonic()-start;assert verified['state']=='success'
 assert cli('verify','fixture','--full','--wait')['state']=='success'
 snap=incremental['result']['snapshot_id'];out=root/'restored'
 planned=cli('restore','plan','fixture',snap,'--path',source/'keep.txt','--target',out,'--wait');assert planned['state']=='success',planned
 planid=planned['result']['plan'];assert cli('restore','apply','fixture',planid,'--confirm',planid,'--wait')['state']=='success'
 restored=out/str(source/'keep.txt').lstrip('/');assert restored.read_bytes()==(source/'keep.txt').read_bytes() and restored.stat().st_mode&0o777==0o640
 (root/'stall').touch();j=control('backup.start',{'grant':g['id']})['ok']['job']
 for _ in range(100):
  if cli('job',j,ok=False).get('progress',{}).get('stage')=='running':break
  time.sleep(.05)
 time.sleep(.1)
 assert 'ok' in control('backup.cancel',{'grant':g['id'],'job':j});assert wait(j)['state']=='cancelled';(root/'stall').unlink()
 cli('approve','fixture','--revoke');assert 'error' in control('backup.status',{'grant':g['id']});cli('approve','fixture')
 gp=root/'state/nyx-backup/grants'/f"{g['id']}.json";v=json.loads(gp.read_text());v['expires']=0;gp.write_text(json.dumps(v));assert 'error' in control('backup.status',{'grant':g['id']});v['expires']=int(time.time())+3600;gp.write_text(json.dumps(v))
 bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','ReleaseName',GLib.Variant('(s)',('io.github.design_nexus.nyx.ControlCenter',)),None,Gio.DBusCallFlags.NONE,5000,None)
 assert 'error' in control('backup.status',{'grant':g['id']});cli('integrations','revoke',g['id']);assert 'error' in control('backup.status',{'grant':g['id']})
 probe=REPO.parent/'nyx-control-center/target/debug/examples/backup_probe'
 if probe.is_file():assert json.loads(cmd(probe,ctl,'fixture').stdout)['unsubscribed_provider_stopped']
 # GitHub API is a local stub: no account or remote repository is contacted.
 preview=cli('github','plan','fixture/settings','--profile','fixture');assert 'approved = false' in preview['configuration'] and 'include_secrets = false' in preview['configuration']
 assert 'error' in cli('github','push',preview['plan'],'--confirm','wrong',ok=False)
 assert cli('github','push',preview['plan'],'--confirm',preview['plan'])['state']=='uploaded'
 fetched=root/'fetched.toml';cli('github','fetch','fixture/settings','--output',fetched);assert fetched.stat().st_mode&0o777==0o600
 assert cli('config','show')['profiles'][0]['approved'] is True,'Fetch must not import or revoke current config'
 second=cli('github','plan','fixture/settings','--profile','fixture');state=json.loads((root/'gh-state.json').read_text());state['sha']='changed-remote';(root/'gh-state.json').write_text(json.dumps(state));assert 'error' in cli('github','push',second['plan'],'--confirm',second['plan'],ok=False)
 state['private']=False;(root/'gh-state.json').write_text(json.dumps(state));assert 'error' in cli('github','plan','fixture/settings','--profile','fixture',ok=False)
 # A changed endpoint is rejected before credential use.
 file.write_text(config.replace(f':{port}',f':{port+1}'));cli('config','import',file);assert 'error' in cli('destination','cloud','test',ok=False);file.write_text(config);cli('config','import',file)
 cli('s3-credentials','cloud','--stdin',input='fixture-access\nwrong-key\n');assert 'error' in cli('destination','cloud','test',ok=False)
 cli('s3-credentials','cloud','--stdin',input='fixture-access\nfixture-secret-key\n');cli('destination','cloud','test')
 server.terminate();server.wait();assert 'error' in cli('destination','cloud','test',ok=False)
 # Credentials never enter config, ciphertext, job journals, grants or sync payloads.
 for directory in [root/'config/nyx-backup',root/'state/nyx-backup']:
  for p in directory.rglob('*'):
   if p.is_file():
    data=p.read_bytes()
    for secret in [b'fixture-secret-key',b'fixture-recovery-password',b'inherited-token-must-be-stripped']:assert secret not in data,(p,secret)
 result={'fixture':str(root),'passed':['signed S3 connectivity and failed validation without initialization','encrypted S3 initial and incremental backup','metadata and full-data verification','S3 selective restore and mode','grant owner/action/profile/expiry/revocation/name ownership checks','legacy start bypass closed','integration cancellation and forbidden action rejection','GitHub settings plan/push/fetch against stub','approval stripping private-repository requirement remote-SHA race refusal','endpoint binding wrong credentials and disconnection refusal','no credential values in persistent app state'],'benchmark':{'source_bytes':32*1024*1024,'small_files':200,'initial_seconds':round(initial_seconds,3),'incremental_seconds':round(incremental_seconds,3),'metadata_verify_seconds':round(metadata_verify_seconds,3),'initial_data_added':initial['result']['data_added'],'incremental_data_added':incremental['result']['data_added']},'limits':['S3 protocol qualified against a signed loopback endpoint; real cloud providers unverified','GitHub network/account behavior unverified; local API stub only','no real user sources or live credentials']}
 (root/'result.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
finally:
 if pid:
  try:os.kill(pid,15)
  except ProcessLookupError:pass
 if server.poll() is None:server.terminate();server.wait()

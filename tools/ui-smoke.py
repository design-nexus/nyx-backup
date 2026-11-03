#!/usr/bin/env python3
"""Render and exercise fixture-only GTK views through an isolated Broadway display.
Build first: cargo build --locked -p nyx-backup --features ui-review
No existing desktop window, account, keyring or configuration is used.
"""
import argparse,json,os,pathlib,shutil,signal,socket,subprocess,tempfile,time
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--theme",choices=["nyx-dark","nyx-light"],default="nyx-dark");args=parser.parse_args()
repo=pathlib.Path(__file__).resolve().parents[1]
root=pathlib.Path(tempfile.mkdtemp(prefix='ui-review-',dir=repo/'target'));root.chmod(0o700)
env=os.environ.copy()
for key,name in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
 (root/name).mkdir(mode=0o700);env[key]=str(root/name)
for key in ['DBUS_SESSION_BUS_ADDRESS','WAYLAND_DISPLAY','DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK']:env.pop(key,None)
with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
(root/'source').mkdir();(root/'source/example.txt').write_text('Disposable GUI review fixture\n')
activation=root/'data/dbus-1/services';activation.mkdir(parents=True);(activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={repo}/target/release/nyx-backupd\n')
icons=root/'data/icons/hicolor';(icons/'scalable/apps').mkdir(parents=True);shutil.copy2(repo/'data/io.github.design_nexus.nyx.Backup.svg',icons/'scalable/apps');shutil.copy2('/usr/share/icons/hicolor/index.theme',icons/'index.theme')
(root/'config/nyx-backup').mkdir(mode=0o700);(root/'config/nyx-backup/config.toml').write_text(f'''version=1
notifications=false
[[destinations]]
id="fixture"
name="Disposable local repository"
type="local"
path="{root}/repository"
[[profiles]]
id="fixture"
name="Review fixture"
destination="fixture"
sources=["{root}/source"]
approved=false
keep_last=3
''')
(root/'config/nyx-backup/appearance.json').write_text(json.dumps({'mode':'theme','theme':args.theme,'reduce_motion':True}))
(root/'bus.conf').write_text(f'''<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir={root}/runtime</listen><auth>EXTERNAL</auth><servicedir>{activation}</servicedir><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>
''')
processes=[];worker=None
log=(root/'review.log').open('w')
def launch(args,**kw):
 p=subprocess.Popen(list(map(str,args)),env=env,stderr=log,start_new_session=True,**kw);processes.append(p);return p
try:
 bus=launch(['dbus-daemon','--nofork','--print-address',f'--config-file={root}/bus.conf'],stdout=subprocess.PIPE);env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().decode().strip();assert env['DBUS_SESSION_BUS_ADDRESS'].startswith('unix:')
 env.update(GDK_BACKEND='broadway',BROADWAY_DISPLAY=':49',GSK_RENDERER='cairo',GTK_A11Y='none',GIO_USE_VFS='local',GTK_USE_PORTAL='0',NYX_BACKUP_UI_REVIEW=str(root/'renders'))
 launch(['gtk4-broadwayd','--address','127.0.0.1','--port',str(port),':49'],stdout=log)
 launch(['chromium','--headless','--disable-gpu','--disable-dev-shm-usage','--disable-background-networking','--disable-component-update','--disable-sync','--no-first-run','--user-data-dir='+str(root/'browser-profile'),'--window-size=1800,1100',f'http://127.0.0.1:{port}'],stdout=log)
 time.sleep(1)
 app=launch([repo/'target/debug/nyx-backup'],stdout=log);assert app.wait(timeout=35)==0,(root/'review.log').read_text()[-4000:]
 result=json.loads((root/'renders/result.json').read_text());assert result['pages_rendered']==8 and result['transport_fields']==6 and result['invalid_save_guard'] and result['ecosystem_escape']
 query=subprocess.run(['gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.GetConnectionUnixProcessID','io.github.design_nexus.nyx.Backup1'],env=env,text=True,capture_output=True)
 if query.returncode==0:worker=int(query.stdout.split('uint32 ')[1].split(',')[0].split(')')[0])
 print(json.dumps({'fixture':str(root),'renders':str(root/'renders'),'theme':args.theme,**result,'limits':['Scripted GTK fixture and software-rendered review; physical desktop input is not injected','No real user files or credentials selected']},indent=2))
finally:
 if worker:
  try:os.kill(worker,signal.SIGTERM)
  except ProcessLookupError:pass
 for p in reversed(processes):
  if p.poll() is None:
   try:os.killpg(p.pid,signal.SIGTERM)
   except ProcessLookupError:pass
   try:p.wait(timeout=3)
   except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
 log.close()

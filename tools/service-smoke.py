#!/usr/bin/env python3
"""Private-bus activation and transient user-timer proof; disposable sources only."""
import os,pathlib,subprocess,sys,tempfile,json,time,uuid,signal
repo=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-backup-service-'))
 env=os.environ.copy();env['NYX_TEST_ROOT']=str(root);env['NYX_TEST_MANAGER_BUS']=env.get('DBUS_SESSION_BUS_ADDRESS',f'unix:path=/run/user/{os.getuid()}/bus')
 for key,name in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache')]:
  (root/name).mkdir();env[key]=str(root/name)
 for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL']:env.pop(key,None)
 activation=root/'data/dbus-1/services';activation.mkdir(parents=True)
 (activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={repo}/target/release/nyx-backupd\n')
 raise SystemExit(subprocess.call(['dbus-run-session','--',sys.executable,__file__,'inner'],env=env))
root=pathlib.Path(os.environ['NYX_TEST_ROOT']);ctl=repo/'target/release/nyx-backupctl'
def run(*args,input=None,env=None,check=True):
 p=subprocess.run(list(map(str,args)),text=True,input=input,capture_output=True,timeout=45,env=env)
 if check and p.returncode:raise AssertionError(f'{args}: {p.stdout} {p.stderr}')
 return p

def call(*args,input=None):return json.loads(run(ctl,*args,'--json',input=input).stdout)
(root/'source').mkdir();(root/'source/file').write_text('scheduled fixture\n')
(root/'profile.toml').write_text(f'''version = 1
notifications = false
[[destinations]]
id = "timer-fixture"
name = "Timer fixture"
type = "local"
path = "{root}/repository"
[[profiles]]
id = "timer-fixture"
name = "Timer fixture"
destination = "timer-fixture"
sources = ["{root}/source"]
approved = true
keep_last = 2
schedule = "daily"
''')
run('gnome-keyring-daemon','--unlock','--components=secrets',input='ephemeral-keyring\n')
call('config','import',root/'profile.toml');call('password','timer-fixture','--stdin',input='ephemeral-repository-secret\n')
call('destination','timer-fixture','init','timer-fixture','--confirm')
# There is no manually launched worker: this requests D-Bus activation.
health=call('status');assert health['active'] is None
pid_text=run('gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.GetConnectionUnixProcessID','io.github.design_nexus.nyx.Backup1').stdout
pid=int(pid_text.split('uint32 ')[1].split(',')[0].split(')')[0])
manager=os.environ.copy();manager['DBUS_SESSION_BUS_ADDRESS']=os.environ['NYX_TEST_MANAGER_BUS'];manager['XDG_RUNTIME_DIR']=f'/run/user/{os.getuid()}'
unit='nyx-backup-qualification-'+uuid.uuid4().hex[:10]
try:
 # Generate the application's real schedule resources in isolated XDG config.
 # The real user manager does not search this directory, so enable is expected to fail.
 run(ctl,'schedule','timer-fixture','--enable','--json',check=False)
 units=root/'config/systemd/user'
 service=units/'nyx-backup-timer-fixture.service';timer=units/'nyx-backup-timer-fixture.timer'
 assert service.exists() and timer.exists()
 run('systemd-analyze','--user','verify',service,timer)
 # A transient timer exercises the same CLI -> private daemon -> Restic path.
 args=['systemd-run','--user','--quiet','--collect',f'--unit={unit}','--on-active=1s']
 for key in ['DBUS_SESSION_BUS_ADDRESS','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME','XDG_DATA_HOME']:
  args.append(f'--setenv={key}={os.environ[key]}')
 args.extend([str(ctl),'backup','timer-fixture','--wait'])
 run(*args,env=manager)
 for _ in range(150):
  jobs=call('jobs')
  if any(j['operation']=='backup' and j['state']=='success' for j in jobs):break
  if any(j['operation']=='backup' and j['state']=='failed' for j in jobs):raise AssertionError(jobs)
  time.sleep(.2)
 else:raise AssertionError('Transient scheduled backup did not complete')
 # A short-lived client completed; the worker and persisted result remain available.
 assert call('status')['last_success']['profile']=='timer-fixture'
 print(json.dumps({'fixture':str(root),'passed':['private D-Bus on-demand activation','generated service/timer syntax','transient systemd user timer -> CLI -> approved profile -> Restic','worker remains after client exit'],'limits':['persistent enable/disable not exercised in the real user configuration']},indent=2))
finally:
 run('systemctl','--user','stop',unit+'.timer',unit+'.service',env=manager,check=False)
 try:os.kill(pid,signal.SIGTERM)
 except ProcessLookupError:pass

#!/usr/bin/env python3
"""Phase 3: synthetic different-home/hardware migration, with no live package install."""
import ast,copy,hashlib,json,os,pathlib,shlex,signal,subprocess,sys,tempfile,time
repo=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-migration-smoke-'));env=os.environ.copy();env['NYX_MIGRATION_ROOT']=str(root)
 for key,name in [('HOME','home'),('XDG_CONFIG_HOME','home/.config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
  (root/name).mkdir(parents=True,exist_ok=True,mode=0o700);env[key]=str(root/name)
 for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK']:env.pop(key,None)
 (root/'bin').mkdir();env['PATH']=str(root/'bin')+os.pathsep+env['PATH']
 # Query-only inventory; the consumer's installer is a separate recording stub.
 script=f'''#!{sys.executable}
import json,pathlib,sys
root=pathlib.Path({str(root)!r})
with (root/'package-query.log').open('a') as f:f.write(json.dumps(sys.argv[1:])+'\\n')
argument=sys.argv[1:]
if argument==['-Q']:print('base 1.0\\ngtk4 4.18')
elif argument==['-Qmq']:raise SystemExit(1)
elif argument==['-Qeq']:print('base\\ngtk4')
elif argument==['-Slq']:print('base\\ngtk4\\ngit\\nvidia-open\\nmesa')
else:raise SystemExit('Unexpected non-read-only package operation')
'''
 (root/'bin/pacman').write_text(script);(root/'bin/pacman').chmod(0o700)
 (root/'bin/flatpak').write_text('#!/bin/sh\nprintf "org.example.App\\tstable\\tx86_64\\tflathub\\n"\n');(root/'bin/flatpak').chmod(0o700)
 sudo=f'''#!{sys.executable}
import json,pathlib,sys
pathlib.Path({str(root/'installer.log')!r}).write_text(json.dumps(sys.argv[1:]))
'''
 (root/'bin/sudo').write_text(sudo);(root/'bin/sudo').chmod(0o700)
 # The installed consumer's exact script, next to the same versioned CLI.
 (root/'bin/nyx-backup-setup').write_bytes((repo/'data/nyx-backup-setup').read_bytes());(root/'bin/nyx-backup-setup').chmod(0o700)
 os.symlink(repo/'target/release/nyx-backupctl',root/'bin/nyx-backupctl')
 activation=root/'data/dbus-1/services';activation.mkdir(parents=True)
 (activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={repo}/target/release/nyx-backupd\n')
 raise SystemExit(subprocess.call(['dbus-run-session','--',sys.executable,__file__,'inner'],env=env))
root=pathlib.Path(os.environ['NYX_MIGRATION_ROOT']);ctl=repo/'target/release/nyx-backupctl';helper=root/'bin/nyx-backup-setup'
def cmd(*args,input=None,ok=True):
 p=subprocess.run(list(map(str,args)),input=input,text=True,capture_output=True,timeout=60)
 if ok and p.returncode:raise AssertionError(f'{args}: {p.stderr} {p.stdout}')
 return p

def call(*args,ok=True,input=None):return json.loads(cmd(ctl,*args,'--json',ok=ok,input=input).stdout)
def setup(*args,ok=True):return json.loads(cmd(helper,*args,'--json',ok=ok).stdout)
def bundle_digest(bundle):
 bundle['digest']='';bundle['digest']=hashlib.sha256(json.dumps(bundle,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
current=root/'home/.config';(current/'demo').mkdir();(current/'hypr').mkdir();(current/'hypr/hyprland.lua').write_text('-- Target main configuration stays intact\n')
(current/'hypr/monitors.lua').write_text('-- target monitor configuration\n');(current/'hypr/envs.conf').write_text('target hardware environment\n');(current/'demo/settings.json').write_text('{"current":true}\n')
old=root/'old-home';oldconfig=old/'.config';(oldconfig/'demo').mkdir(parents=True);(oldconfig/'hypr').mkdir();(oldconfig/'nyx-backup').mkdir()
(oldconfig/'demo/settings.json').write_text(json.dumps({'cache':str(old/'Documents'),'url':'https://server'+str(old/'Documents')})+'\n')
(oldconfig/'hypr/monitors.conf').write_text('monitor = SOURCE-DP-9,1920x1080@60,0x0,1\n')
(oldconfig/'hypr/envs.conf').write_text('env = LIBVA_DRIVER_NAME,nvidia\n')
(oldconfig/'nyx-backup/config.toml').write_text('do not overwrite the receiving app\n')
(root/'profile.toml').write_text(f'''version = 1
notifications = false
[[destinations]]
id = "fixture"
name = "Fixture"
type = "local"
path = "{root}/repository"
[[profiles]]
id = "fixture"
name = "Fixture"
destination = "fixture"
sources = ["{oldconfig}","{current}/demo"]
approved = true
keep_last = 3
''')
cmd('gnome-keyring-daemon','--unlock','--components=secrets',input='ephemeral-migration-keyring\n');call('config','import',root/'profile.toml');call('password','fixture','--stdin',input='ephemeral-migration-password\n')
pid=None
try:
 assert call('destination','fixture','init','fixture','--confirm')['state']=='success'
 assert call('backup','fixture','--wait')['state']=='success'
 snapshot=call('restore','snapshots','fixture')[0]['id']
 pid_reply=cmd('gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.GetConnectionUnixProcessID','io.github.design_nexus.nyx.Backup1').stdout;pid=int(pid_reply.split('uint32 ')[1].split(',')[0].split(')')[0])
 source=call('migration','source','fixture',snapshot);assert source['version']==1 and any(p['name']=='base' for p in source['packages']),source
 assert not any('package inventory unavailable' in x for x in source['missing']), 'Empty foreign package query is legitimate'
 # A new snapshot contains an attested package/hardware inventory.
 attested=call('migration','prepare','fixture',snapshot,'--wait');assert attested['state']=='success',attested
 assert call('migration','inspect','fixture',attested['result']['bundle'])['source_attested']
 external=copy.deepcopy(source);external['home']=str(old);external['config']=str(oldconfig);external['hardware']['cpu_vendor']='synthetic-old-vendor';external['hardware']['product']='synthetic old model';external['hardware']['gpus']=[{'vendor':'0xffff','device':'0xffff'}];external['hardware']['monitor_format']='conf'
 external['packages']=[{'name':name,'version':'1.0','explicit':explicit,'origin':origin} for name,origin,explicit in [('base','native',True),('git','native',True),('nvidia-open','native',True),('foreign-app','foreign',True),('lib-dependency','native',False),('not-in-sync','native',True)]]
 manifest=root/'source-manifest.json';manifest.write_text(json.dumps(external))
 planned=call('migration','prepare','fixture',snapshot,'--path',oldconfig,'--source-manifest',manifest,'--policy','replace','--wait');assert planned['state']=='success',planned
 bundle=call('migration','inspect','fixture',planned['result']['bundle']);assert not bundle['source_attested']
 held={p['path'] for p in bundle['held_files']};assert str(oldconfig/'hypr/envs.conf') in held and str(oldconfig/'nyx-backup/config.toml') in held,bundle
 decisions={p['name']:p['action'] for p in bundle['packages']};assert decisions['git']=='install' and decisions['nvidia-open']=='review_hardware' and decisions['foreign-app']=='review_foreign' and decisions['lib-dependency']=='dependency_only' and decisions['not-in-sync']=='review_unavailable',decisions
 assert len(bundle['hardware_changes'])>=3
 plan=call('restore','inspect','fixture',bundle['restore_plan']);assert (current/'demo/settings.json').read_text()=='{"current":true}\n','Planning must not change target config'
 entries={p['source']:p for p in plan['entries']};assert entries[str(oldconfig/'demo/settings.json')]['target']==str(current/'demo/settings.json')
 assert entries[str(oldconfig/'hypr/monitors.conf')]['target']==str(current/'hypr/monitors.lua')
 diff=call('restore','diff','fixture',plan['id'],oldconfig/'demo/settings.json');assert str(root/'home/Documents') in diff['diff'] and 'prepared' in diff['diff']
 stage=root/'state/nyx-backup/restore'/plan['id']/'stage'/str(oldconfig/'hypr/monitors.conf').lstrip('/')
 lua=stage.read_text();assert 'hl.monitor' in lua and 'SOURCE-DP-9' not in lua and '\\n' not in lua
 cmd('luac','-p',stage)
 exported=root/'setup.json';call('migration','export','fixture',bundle['id'],exported);assert (exported.stat().st_mode&0o777)==0o600
 preview=setup('inspect',exported);assert preview['install']==['git'],preview
 assert not (root/'installer.log').exists()
 wrong=setup('packages',exported,'--apply','--confirm','wrong',ok=False);assert 'error' in wrong and not (root/'installer.log').exists()
 assert setup('packages',exported,'--apply','--confirm',bundle['id'])['ok']
 assert json.loads((root/'installer.log').read_text())==['pacman','-S','--needed','--','git']
 # Native consumer uses only current eligible packages, not a forged action list.
 forged=copy.deepcopy(bundle);next(p for p in forged['packages'] if p['name']=='foreign-app')['action']='install';bundle_digest(forged);(root/'forged.json').write_text(json.dumps(forged));denied=setup('packages',root/'forged.json','--apply','--confirm',bundle['id'],ok=False);assert 'error' in denied,denied
 changed=copy.deepcopy(bundle);changed['warnings'].append('tamper');(root/'damaged.json').write_text(json.dumps(changed));assert 'error' in setup('inspect',root/'damaged.json',ok=False)
 # Explicit Setup restore applies the reviewed mappings; rollback restores target originals.
 restored=setup('restore',exported,'--confirm',bundle['id'],'--wait');assert restored['state']=='success',restored
 result=json.loads((current/'demo/settings.json').read_text());assert result['cache']==str(root/'home/Documents') and result['url']=='https://server'+str(old/'Documents')
 assert (current/'hypr/envs.conf').read_text()=='target hardware environment\n'
 assert (current/'hypr/hyprland.lua').read_text()=='-- Target main configuration stays intact\n'
 assert (current/'hypr/monitors.lua').read_text()==lua
 assert call('restore','rollback','fixture',plan['id'],'--confirm',plan['id'],'--wait')['state']=='success'
 assert (current/'demo/settings.json').read_text()=='{"current":true}\n' and (current/'hypr/monitors.lua').read_text()=='-- target monitor configuration\n'
 assert call('migration','bundles','fixture')['bundles']
 # Setup prepare is a real export consumer, not only a documentation example.
 handoff=root/'second-setup.json';assert setup('prepare','fixture',snapshot,'--source-manifest',manifest,'--bundle',handoff)['exported']==str(handoff)
 assert handoff.exists()
 # An unsafe package argument is rejected before any process handoff.
 invalid=copy.deepcopy(external);invalid['packages'][0]['name']='--overwrite';(root/'unsafe.json').write_text(json.dumps(invalid));assert 'error' in call('migration','prepare','fixture',snapshot,'--source-manifest',root/'unsafe.json','--wait',ok=False)
 calls=[json.loads(line) for line in (root/'package-query.log').read_text().splitlines()];assert all(call in [['-Q'],['-Qmq'],['-Qeq'],['-Slq'],['-Q','omarchy'],['-Q','omarchy-dev']] for call in calls)
 denied=cmd('gdbus','call','--session','--dest','io.github.design_nexus.nyx.Backup1','--object-path','/io/github/design_nexus/nyx/Backup1','--method','io.github.design_nexus.nyx.Backup1.Start','fixture','migration-plan',ok=False);assert denied.returncode!=0
 journals=''.join(p.read_text() for p in (root/'state/nyx-backup/jobs').glob('*.json'));assert 'ephemeral-migration-password' not in journals
 print(json.dumps({'fixture':str(root),'passed':['snapshot-attested inventory and missing-coverage fields','empty foreign package set','explicit external-manifest provenance','different-home configuration mapping','target-format automatic monitor generation and Lua syntax','bounded literal path-prefix rewriting','held hardware/app-private files','native/foreign/hardware/dependency/unavailable package decisions','private Setup export and installed-consumer script','dry-run package preview','confirmed package installer boundary with recording stub','forged/damaged bundle and unsafe package refusal','explicit Setup restore and existing rollback','Setup prepare/export consumer','no repository refresh or live package operation','integration API rejects migration']},indent=2))
finally:
 if pid:
  try:os.kill(pid,signal.SIGTERM)
  except ProcessLookupError:pass

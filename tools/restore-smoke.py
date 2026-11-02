#!/usr/bin/env python3
"""Phase 2 native Restic qualification with a private bus, keyring and home."""
import json,os,pathlib,subprocess,sys,tempfile,time,stat,signal,shlex
repo=pathlib.Path(__file__).resolve().parents[1]
if len(sys.argv)==1:
 root=pathlib.Path(tempfile.mkdtemp(prefix='nyx-restore-smoke-'));env=os.environ.copy();env['NYX_RESTORE_ROOT']=str(root)
 for key,name in [('HOME','home'),('XDG_CONFIG_HOME','home/.config'),('XDG_DATA_HOME','data'),('XDG_STATE_HOME','state'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
  (root/name).mkdir(parents=True,exist_ok=True,mode=0o700);env[key]=str(root/name)
 for key in ['DISPLAY','WAYLAND_DISPLAY','GNOME_KEYRING_CONTROL','SSH_AUTH_SOCK']:env.pop(key,None)
 (root/'bin').mkdir();env['PATH']=str(root/'bin')+os.pathsep+env['PATH']
 # A deterministic stalled restore child checks cancellation of the whole process group.
 wrapper=f'''#!/bin/sh
for argument in "$@"; do
 if [ "$argument" = restore ] && [ -e {shlex.quote(str(root/'pause-restore'))} ]; then
  sleep 30 &
  child=$!
  echo "$child" > {shlex.quote(str(root/'restore-child-running'))}
  wait "$child"
 fi
done
exec /usr/bin/restic "$@"
'''
 (root/'bin/restic').write_text(wrapper);(root/'bin/restic').chmod(0o700)
 activation=root/'data/dbus-1/services';activation.mkdir(parents=True)
 (activation/'io.github.design_nexus.nyx.Backup1.service').write_text(f'[D-BUS Service]\nName=io.github.design_nexus.nyx.Backup1\nExec={repo}/target/release/nyx-backupd\n')
 raise SystemExit(subprocess.call(['dbus-run-session','--',sys.executable,__file__,'inner'],env=env))
root=pathlib.Path(os.environ['NYX_RESTORE_ROOT']);ctl=repo/'target/release/nyx-backupctl'
def cmd(*args,input=None,ok=True):
 p=subprocess.run(list(map(str,args)),text=True,input=input,capture_output=True,timeout=60)
 if ok and p.returncode:raise AssertionError(f'{args}: {p.stderr} {p.stdout}')
 return p

def call(*args,input=None,ok=True):return json.loads(cmd(ctl,*args,'--json',input=input,ok=ok).stdout)
def restore(*args,ok=True):return call('restore',*args,ok=ok)
def prepared(path,target=None,policy='skip',original=False,secrets=False):
 args=['plan','fixture',snapshot,'--path',str(path),'--policy',policy,'--wait']
 if original:args.append('--original-dotfiles')
 else:args.extend(['--target',str(target)])
 if secrets:args.append('--include-secrets')
 result=restore(*args);assert result['state']=='success',result
 return restore('inspect','fixture',result['result']['plan'])
def apply(plan):return restore('apply','fixture',plan['id'],'--confirm',plan['id'],'--wait')
def undo(plan,ok=True):return restore('rollback','fixture',plan['id'],'--confirm',plan['id'],'--wait',ok=ok)
def wait(job):
 for _ in range(200):
  value=call('job',job,ok=False)
  if value.get('state')!='running':return value
  time.sleep(.05)
 raise AssertionError('Restore job did not finish')
source=root/'source';source.mkdir();config=root/'home/.config/demo';config.mkdir(parents=True)
(source/'personal.txt').write_text('snapshot personal\n');os.chmod(source/'personal.txt',0o640)
(source/'literal[1]*.txt').write_text('literal filename\n');(source/'literal11.txt').write_text('must not be selected\n')
(source/'empty').mkdir();os.symlink('/absent-fixture-target',source/'link')
(config/'app.conf').write_text('snapshot-phase2-config-unique\n');(config/'.env').write_text('dummy-fixture-credential-material')
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
sources = ["{source}","{config}"]
include_secrets = true
approved = true
keep_last = 3
''')
cmd('gnome-keyring-daemon','--unlock','--components=secrets',input='ephemeral-restore-keyring\n')
call('config','import',root/'profile.toml');call('password','fixture','--stdin',input='ephemeral-restore-password\n')
pid=None
try:
 initialized=call('destination','fixture','init','fixture','--confirm');assert initialized['state']=='success',initialized
 result=call('backup','fixture','--wait');assert result['state']=='success',result
 snapshots=restore('snapshots','fixture');snapshot=snapshots[0]['id'];assert len(snapshot)==64
 pid_reply=cmd('gdbus','call','--session','--dest','org.freedesktop.DBus','--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.GetConnectionUnixProcessID','io.github.design_nexus.nyx.Backup1').stdout
 pid=int(pid_reply.split('uint32 ')[1].split(',')[0].split(')')[0])
 listing=restore('browse','fixture',snapshot,'--directory',str(source));assert any(n['path']==str(source/'personal.txt') for n in listing['entries']),listing
 output=root/'separate'
 plan=prepared(source/'personal.txt',output);assert not output.exists(),'Preview must not write to the selected target'
 assert apply(plan)['state']=='success'
 target=pathlib.Path(plan['entries'][0]['target']);assert target.read_text()=='snapshot personal\n' and stat.S_IMODE(target.stat().st_mode)==0o640
 assert undo(plan)['state']=='success' and not target.exists()
 special=prepared(source/'literal[1]*.txt',root/'literal-output');assert len(special['entries'])==1,special
 assert apply(special)['state']=='success';assert pathlib.Path(special['entries'][0]['target']).read_text()=='literal filename\n'
 link=prepared(source/'link',root/'links');assert apply(link)['state']=='success';assert os.readlink(link['entries'][0]['target'])=='/absent-fixture-target';assert undo(link)['state']=='success'
 empty=prepared(source/'empty',root/'empty-output');assert apply(empty)['state']=='success';assert pathlib.Path(empty['empty_directories'][0]).is_dir();assert undo(empty)['state']=='success'
 # Default conflicts preserve the current file; keep-both creates a distinct version.
 conflicts=root/'conflicts';expected=conflicts/str(source/'personal.txt').lstrip('/');expected.parent.mkdir(parents=True);expected.write_text('existing file\n')
 skip=prepared(source/'personal.txt',conflicts);assert skip['entries'][0]['action']=='skip';assert apply(skip)['state']=='success';assert expected.read_text()=='existing file\n'
 both=prepared(source/'personal.txt',conflicts,policy='keep-both');assert apply(both)['state']=='success';assert expected.read_text()=='existing file\n';assert pathlib.Path(both['entries'][0]['target']).read_text()=='snapshot personal\n';assert undo(both)['state']=='success'
 # A target changed after preview cannot be overwritten.
 stale=prepared(source/'personal.txt',conflicts,policy='replace');expected.write_text('changed after preview\n');result=restore('apply','fixture',stale['id'],'--confirm',stale['id'],'--wait',ok=False);assert result['state']=='failed',result;assert expected.read_text()=='changed after preview\n'
 # Original dotfiles: view a diff, replace with a saved original, and roll back.
 (config/'app.conf').write_text('current user configuration\n');os.chmod(config/'app.conf',0o640)
 quick=prepared(config/'app.conf',policy='replace',original=True)
 diff=restore('diff','fixture',quick['id'],str(config/'app.conf'));assert '-current user configuration' in diff['diff'] and '+snapshot-phase2-config-unique' in diff['diff'],diff
 assert apply(quick)['state']=='success';assert (config/'app.conf').read_text()=='snapshot-phase2-config-unique\n';assert pathlib.Path(quick['entries'][0]['previous']).read_text()=='current user configuration\n'
 preview=restore('rollback-preview','fixture',quick['id']);assert preview['entries'][0]['status']['action']=='undo'
 assert undo(quick)['state']=='success';assert (config/'app.conf').read_text()=='current user configuration\n';assert stat.S_IMODE((config/'app.conf').stat().st_mode)==0o640
 # Edits after restoration survive rollback; both the edit and saved original remain.
 changed=prepared(config/'app.conf',policy='replace',original=True);assert apply(changed)['state']=='success';(config/'app.conf').write_text('new edit after restore\n');result=undo(changed,ok=False);assert result['state']=='incomplete',result;assert (config/'app.conf').read_text()=='new edit after restore\n';assert pathlib.Path(changed['entries'][0]['previous']).read_text()=='current user configuration\n'
 # Profile boundaries apply to both snapshot data and saved restore receipts.
 original_configuration=(root/'profile.toml').read_text()
 (root/'profile.toml').write_text(original_configuration+f'\n[[profiles]]\nid = "other"\nname = "Other"\ndestination = "fixture"\nsources = ["{source}"]\n')
 call('config','import',root/'profile.toml')
 foreign=restore('browse','other',snapshot,'--directory',str(source),ok=False)
 assert 'error' in foreign and 'does not belong' in foreign['error'],foreign
 foreign_receipt=restore('inspect','other',quick['id'],ok=False)
 assert 'error' in foreign_receipt and 'does not belong' in foreign_receipt['error'],foreign_receipt
 (root/'profile.toml').write_text(original_configuration);call('config','import',root/'profile.toml')
 root_refusal=restore('plan','fixture',snapshot,'--path',str(source/'personal.txt'),'--target','/','--wait',ok=False)
 assert root_refusal['state']=='failed' and 'filesystem root' in root_refusal['message'],root_refusal
 # Credentials require explicit authorization before staging.
 rejected=restore('plan','fixture',snapshot,'--path',str(config/'.env'),'--target',root/'credential-output','--wait',ok=False);assert rejected['state']=='failed' and not (root/'credential-output').exists(),rejected
 allowed=prepared(config/'.env',root/'credential-output',secrets=True);assert apply(allowed)['state']=='success'
 # Snapshot ownership and traversal checks precede target writes.
 traversal=restore('plan','fixture',snapshot,'--path','/tmp/../escape','--target',root/'escape','--wait',ok=False);assert 'error' in traversal
 wrong_confirmation=restore('apply','fixture',stale['id'],'--confirm','wrong','--wait',ok=False);assert 'error' in wrong_confirmation
 outside=root/'outside';outside.mkdir();os.symlink(outside,root/'conflicts-link')
 unsafe=restore('plan','fixture',snapshot,'--path',str(source/'personal.txt'),'--target',conflicts,'--policy','replace','--wait');unsafe_plan=restore('inspect','fixture',unsafe['result']['plan']);parent=pathlib.Path(unsafe_plan['entries'][0]['target']).parent;parent.rename(parent.with_name(parent.name+'-moved'));os.symlink(outside,parent)
 result=restore('apply','fixture',unsafe_plan['id'],'--confirm',unsafe_plan['id'],'--wait',ok=False);assert result['state']=='failed',result;assert not list(outside.iterdir())
 # A deterministic stalled child is cancelled, with no writes to the chosen target.
 (root/'pause-restore').touch();pending=restore('plan','fixture',snapshot,'--path',str(source/'personal.txt'),'--target',root/'cancelled-output')['job']
 for _ in range(200):
  if (root/'restore-child-running').exists():break
  time.sleep(.05)
 else:raise AssertionError('Cancellation fixture did not reach the child process')
 child=int((root/'restore-child-running').read_text());call('cancel',pending);cancelled=wait(pending);assert cancelled['state']=='cancelled',cancelled;assert not (root/'cancelled-output').exists()
 proc=pathlib.Path(f'/proc/{child}/stat');assert not proc.exists() or proc.read_text().split()[2]=='Z'
 (root/'pause-restore').unlink()
 # Read-only/diff data must never be embedded in the ordinary job journal.
 jobs=''.join(p.read_text() for p in (root/'state/nyx-backup/jobs').glob('*.json'))
 assert 'snapshot-phase2-config-unique' not in jobs and 'ephemeral-restore-password' not in jobs and 'dummy-fixture-credential-material' not in jobs
 receipts=restore('receipts','fixture');assert len(receipts['receipts'])>=8
 denied=cmd('gdbus','call','--session','--dest','io.github.design_nexus.nyx.Backup1','--object-path','/io/github/design_nexus/nyx/Backup1','--method','io.github.design_nexus.nyx.Backup1.Start','fixture','restore-apply',ok=False);assert denied.returncode!=0
 print(json.dumps({'fixture':str(root),'passed':['snapshot browsing','staged preview without target writes','selected restore contents and permissions','literal glob filenames','symlink and empty-directory restore','skip/keep-both/replace policies','post-preview conflict refusal','Quick Dotfiles Restore','local text diff without journal content','retained originals and rollback','rollback conflict preservation','credential restore authorization','traversal/symlink/confirmation guards','stalled-child process-group cancellation','restore receipts','integration API rejects restoration','snapshot and receipt profile ownership','filesystem-root restore refusal']},indent=2))
finally:
 if pid:
  try:os.kill(pid,signal.SIGTERM)
  except ProcessLookupError:pass

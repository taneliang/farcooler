#!/usr/bin/python3
"""Actual CLI subprocess -> existing socket transport -> RpcFactory -> owned runtime."""
import json, os, signal, subprocess, tempfile, time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
target=Path('/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/worktrees/overnight/ov-923-workflow-runtime/target/debug')
fixture=Path(tempfile.mkdtemp(prefix='ov925-',dir='/tmp'))/'fresh'
env={k:v for k,v in os.environ.items() if not k.startswith('FARCOOLER_')}
env['FARCOOLER_HOME']=str(fixture/'home')
records=[]
log=open(root/'reports/ov-925/admission-trace-server.log','w')
server=subprocess.Popen([str(target/'examples/workflow_command_server'),str(fixture)],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
try:
 deadline=time.monotonic()+30
 while not (fixture/'ready.json').exists():
  if server.poll() is not None or time.monotonic()>deadline: raise RuntimeError('fixture failed to become ready')
  time.sleep(.05)
 ready=json.loads((fixture/'ready.json').read_text());ws=ready['workspace'];pin=ready['pin']
 def cli(*args):
  result=subprocess.run([str(target/'farcooler'),'--json',*args],env=env,capture_output=True,text=True,timeout=30)
  records.append({'args':list(args),'exit':result.returncode,'stdout':result.stdout,'stderr':result.stderr})
  if result.returncode: raise RuntimeError(f'CLI refused {args}: {result.stderr} {result.stdout}')
  return json.loads(result.stdout)
 def identity(id,revision,seq):return ['--command-id',id,'--revision',str(revision),'--epoch','1','--sequence',str(seq)]
 publish=['workspace','set-workflow','Main','--file',str(fixture/'definition.json'),*identity('publish',0,1)]
 p=cli(*publish);assert 'Ok' in p['receipt']['result']
 enqueue=['workflow','enqueue',ws,'a','--pin',pin,*identity('enqueue',0,1)]
 admitted=cli(*enqueue);assert cli(*enqueue)==admitted
 s=cli('workflow','get',ws);assert s['state']['items']['a']['phase']=='WaitingApproval'
 item=s['state']['items']['a']
 approve=['workflow','approve',ws,'a','--run',item['run'],'--pin',pin,*identity('approve',s['state']['revision'],2)]
 approval=cli(*approve)
 deadline=time.monotonic()+30
 while True:
  s=cli('workflow','get',ws)
  if s['state']['items']['a']['phase']=='Done':break
  if time.monotonic()>deadline:raise RuntimeError('automatic runtime did not finish')
  time.sleep(.05)
 assert cli(*approve)==approval
 second=['workflow','enqueue',ws,'b','--pin',pin,*identity('second',s['state']['revision'],3)]
 cli(*second);s=cli('workflow','get',ws);item=s['state']['items']['b']
 cancel=['workflow','cancel',ws,'b','--run',item['run'],'--pin',pin,*identity('cancel',s['state']['revision'],4)]
 cancelled=cli(*cancel);assert cli(*cancel)==cancelled
 final=cli('workflow','get',ws);assert final['state']['items']['b']['phase']=='Cancelled'
 records.append({'summary':'CLI publication/enqueue/read/approve/automatic progression/cancel and distinct-connection identity retries passed','fixture':str(fixture),'final_revision':final['state']['revision'],'owner_app_data_or_provider_used':False,'auth_boundary':'fixture server supplies known owner peer; not a real SSH/enrollment proof; held-queue unit tests exercise device access/session changes and real isolated tmux caller proof'})
finally:
 server.send_signal(signal.SIGINT)
 try:server.wait(timeout=10)
 except subprocess.TimeoutExpired:os.killpg(server.pid,signal.SIGKILL);server.wait()
 log.close()
 (root/'reports/ov-925/admission-trace.json').write_text(json.dumps(records,indent=2)+'\n')
print('actual CLI trace passed')

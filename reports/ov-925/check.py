#!/usr/bin/python3
"""Bounded, reserve-monitored checks under the machine heavy lock; no provider/app calls."""
import json, os, shutil, signal, subprocess, sys, time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
target=Path('/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/worktrees/overnight/ov-923-workflow-runtime/target')
env={**os.environ,'CARGO_TARGET_DIR':str(target),'CARGO_BUILD_JOBS':'6','CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0'}
cargo='/Users/e-liang/.cargo/bin/cargo'
commands={
 'mutation':['script',str(root/'reports/ov-925/mutation.py')],
 'compile':['check','--offline','-p','farcooler-cli','-p','farcooler-daemon','-p','farcooler-store','--all-targets'],
 'store':['test','--offline','-p','farcooler-store','--lib','workflow'],
 'runtime':['test','--offline','-p','farcooler-daemon','--lib','workflow_'],
 'protocol':['test','--offline','-p','farcooler-protocol','workflow_wire'],
 'cli':['test','--offline','-p','farcooler-cli','workflow_'],
 'socket':['test','--offline','-p','farcooler-daemon','--test','workflow_commands_over_socket'],
 'lint':['clippy','--offline','-p','farcooler-cli','-p','farcooler-daemon','-p','farcooler-store','--all-targets','--','-D','warnings'],
 'build':['build','--offline','-p','farcooler-cli','-p','farcooler-daemon','--example','workflow_command_server','--bin','farcooler'],
}
minimum=shutil.disk_usage(root).free
for name in sys.argv[1:]:
 args=commands[name]; start=time.monotonic(); stopped=None
 with open(root/f'reports/ov-925/{name}.log','w') as log:
  command=['/usr/bin/python3',*args[1:]] if args[0]=='script' else [cargo,*args]
  child=subprocess.Popen(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
  try:
   while child.poll() is None:
    minimum=min(minimum,shutil.disk_usage(root).free)
    if minimum<10*1024**3 or time.monotonic()-start>1800:
     stopped='reserve/deadline';break
    time.sleep(1)
  finally:
   if child.poll() is None:
    os.killpg(child.pid,signal.SIGTERM)
    try: child.wait(timeout=10)
    except subprocess.TimeoutExpired: os.killpg(child.pid,signal.SIGKILL);child.wait()
   code=child.wait(); log.write(f'\nexit={code}, stopped={stopped}\n')
 record={'command':[cargo,*args],'exit':code,'stopped':stopped,'seconds':time.monotonic()-start,'minimum_free':minimum,'target':str(target),'profile':'debug0/test0/incremental0/jobs6/offline','source':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()}
 (root/f'reports/ov-925/{name}.json').write_text(json.dumps(record,indent=2)+'\n')
 print(f'{name}: exit={code} stopped={stopped}',flush=True)
 if code or stopped: sys.exit(code or 1)

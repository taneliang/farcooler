#!/usr/bin/python3
"""One meaningful red/green: replay identity must bind the full admitted action."""
from pathlib import Path
import hashlib, os, signal, subprocess, time
root=Path(__file__).resolve().parents[2]
path=root/'crates/daemon/src/workflow_runtime/wire_commands.rs'
original=path.read_bytes()
needle=b'let payload = serde_json::to_string(&(principal, &action))?;'
assert original.count(needle)==1
# The surrounding heavy supervisor supplies the same profile/target and deadline.
env={**os.environ,'CARGO_TARGET_DIR':'/Users/e-liang/Library/Application Support/com.farcooler.FarCoolerCanary/worktrees/overnight/ov-923-workflow-runtime/target','CARGO_BUILD_JOBS':'6','CARGO_PROFILE_DEV_DEBUG':'0','CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0'}
def run(name,expected):
 with open(root/f'reports/ov-925/{name}.log','w') as log:
  child=subprocess.Popen(['/Users/e-liang/.cargo/bin/cargo','test','--offline','-p','farcooler-daemon','--lib','workflow_wire_pins_policy_and_replays_after_restart_without_reexecution'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=False)
  try: code=child.wait(timeout=300)
  finally:
   if child.poll() is None:
    child.terminate()
    try:child.wait(timeout=10)
    except subprocess.TimeoutExpired:child.kill();child.wait()
  log.write(f'\nexit={code}, expected={expected}\n')
 assert code==expected,(name,code)
 print(name,code,flush=True)
def interrupted(signum,frame):raise KeyboardInterrupt(f'signal {signum}')
signal.signal(signal.SIGTERM,interrupted)
try:
 path.write_bytes(original.replace(needle,b'let payload = serde_json::to_string(principal)?;'))
 run('identity-mutation',101)
finally:
 path.write_bytes(original)
 assert hashlib.sha256(path.read_bytes()).digest()==hashlib.sha256(original).digest()
run('identity-restored',0)

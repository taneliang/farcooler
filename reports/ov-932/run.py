#!/usr/bin/python3
"""Record one owned, finite FIFO check; never retry or infer a pass."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
REPORT = ROOT / 'reports/ov-932'
label, *command = sys.argv[1:]
assert label and command and '/' not in label
env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
           CARGO_BUILD_JOBS='6', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
           CARGO_INCREMENTAL='0', CARGO_NET_OFFLINE='true', MACOSX_DEPLOYMENT_TARGET='26.0')
target = ROOT.parent / 'ov-923-workflow-runtime/target'
env['CARGO_TARGET_DIR'] = str(target)
args = ['/usr/bin/python3', str(ROOT/'scripts/heavy'), '--load-limit', '20', '--',
        '/usr/bin/python3', str(ROOT/'reports/ov-930/entry.py'), str(target), *command]
tracked = subprocess.check_output(['git', 'ls-files', 'apps', 'crates', 'Cargo.lock', 'Cargo.toml', 'services/web'], cwd=ROOT, text=True).splitlines()
def hashes():
    return {p: hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in tracked if (ROOT/p).is_file()}
before = hashes()
start = time.monotonic()
minimum = shutil.disk_usage(ROOT).free
stopped = None
with (REPORT/(label+'.log')).open('w') as log:
    log.write('COMMAND '+repr(args)+'\n'); log.flush()
    child = subprocess.Popen(args, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    def stop():
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try: child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    def interrupted(sig, _):
        stop()
        raise SystemExit(128+sig)
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT): signal.signal(sig, interrupted)
    try:
        while child.poll() is None:
            minimum = min(minimum, shutil.disk_usage(ROOT).free)
            if minimum < 10*1024**3 or time.monotonic()-start > 600:
                stopped = 'deadline/reserve'
                break
            time.sleep(.25)
    finally: stop()
    log.write('\nEXIT '+str(child.returncode)+'\n')
after = hashes()
result = dict(command=args, source=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
              exit=child.returncode, stopped=stopped, seconds=time.monotonic()-start,
              minimum_free=minimum, source_unchanged=before==after, source_sha256=before)
(REPORT/(label+'.json')).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({k:v for k,v in result.items() if k!='source_sha256'}))
sys.exit(child.returncode if not stopped and before==after else 1)

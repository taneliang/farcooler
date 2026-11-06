#!/usr/bin/python3
"""One normal FIFO route, deadline, reserve monitoring, owned process group/reap."""
import os
from pathlib import Path
import signal
import subprocess
import time
import sys
import shutil
import uuid
root = Path(__file__).resolve().parents[2]
env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
           CARGO_TARGET_DIR=str(root.parent / 'ov-923-workflow-runtime/target'), CARGO_BUILD_JOBS='6', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0', CARGO_INCREMENTAL='0')
mode = sys.argv[1] if len(sys.argv) > 1 else 'focused'
publication = mode == 'publication'
if publication: mode = 'focused'
validate = mode == 'validate'
if validate: mode = 'focused'
fixture_root = Path('/private/tmp') / f'ov-923-runtime-{uuid.uuid4()}'
args = {'trace': ['run', '--offline', '-p', 'farcooler-daemon', '--example', 'workflow_fixture', '--', str(fixture_root)],
        'focused': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'workflow_runtime', '--', '--nocapture'],
        'clippy': ['clippy', '--offline', '-p', 'farcooler-daemon', '-p', 'farcooler-store', '--all-targets', '--', '-D', 'warnings'],
        'store': ['test', '--offline', '-p', 'farcooler-store', 'workflow', '--', '--nocapture'],
        'gate': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'landing_job', '--', '--nocapture']}[mode]
command = [str(root / 'scripts/heavy'), '--load-limit', '20', '--', '/Users/e-liang/.cargo/bin/cargo', *args]
if validate:
    mode = 'validation'
    command = [str(root / 'scripts/heavy'), '--load-limit', '20', '--', '/usr/bin/python3', str(root / 'reports/ov-923/check-delta.py')]
if publication:
    mode = 'publication-validation'
    command = [str(root / 'scripts/heavy'), '--load-limit', '20', '--', '/usr/bin/python3', str(root / 'reports/ov-923/check-publication.py')]
minimum = None
start = time.monotonic()
with open(root / f'reports/ov-923/delta-{mode}.log', 'a') as log:
    log.write(f'\ncommand={command!r}\n')
    log.flush()
    child = subprocess.Popen(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    cancelled = False
    while child.poll() is None:
        stats = os.statvfs(root)
        free = stats.f_bavail * stats.f_frsize
        minimum = free if minimum is None else min(minimum, free)
        if free < 10 * 1024**3 or time.monotonic() - start > 1800:
            os.killpg(child.pid, signal.SIGTERM)
            cancelled = True
            break
        time.sleep(1)
    try:
        code = child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        code = child.wait()
    log.write(f'\nwrapper_exit={code} cancelled={cancelled} minimum_free_bytes={minimum}\n')
if mode == 'trace' and code == 0:
    destination = root / 'reports/ov-923/trace'
    shutil.copytree(fixture_root / 'receipts', destination / 'receipts')
    shutil.copy2(fixture_root / 'trace.json', destination / 'trace.json')
    shutil.copy2(fixture_root / 'fixture.json', destination / 'fixture.json')
    (destination / 'origin.txt').write_text(str(fixture_root) + '\n')
print(f'exit={code} cancelled={cancelled} minimum_free_bytes={minimum}')
raise SystemExit(code if code >= 0 else 128 - code)

#!/usr/bin/python3
"""Own and reap a single heavy foreground test route with a 10 GiB disk reserve."""
import os
from pathlib import Path
import signal
import subprocess
import time

root = Path(__file__).resolve().parents[2]
env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin:/Users/e-liang/.cargo/bin',
           CARGO_BUILD_JOBS='6', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
           CARGO_INCREMENTAL='0')
import sys
checks = len(sys.argv) > 1 and sys.argv[1] == "checks"
command = [str(root / 'scripts/heavy'), '--', '/Users/e-liang/.cargo/bin/cargo',
           'test', '--offline', '-p', 'farcooler-workflow', '-p', 'farcooler-store',
           'workflow', '--', '--nocapture']
if checks:
    command = [str(root / "scripts/heavy"), "--", "/usr/bin/python3", str(root / "reports/ov-922/checks.py")]
minimum = None
with open(root / ('reports/ov-922/additional-checks.log' if checks else 'reports/ov-922/final-focused-tests.log'), 'w') as log:
    child = subprocess.Popen(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT,
                             start_new_session=True)
    cancelled = False
    while child.poll() is None:
        stats = os.statvfs(root)
        free = stats.f_bavail * stats.f_frsize
        minimum = free if minimum is None else min(minimum, free)
        if free < 10 * 1024**3:
            os.killpg(child.pid, signal.SIGTERM)
            cancelled = True
            break
        time.sleep(1)
    try:
        code = child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        code = child.wait()
    log.write(f'\nwrapper_exit={code} reserve_cancelled={cancelled} minimum_free_bytes={minimum}\n')
print(f'exit={code} reserve_cancelled={cancelled} minimum_free_bytes={minimum}')
raise SystemExit(code if code >= 0 else 128 - code)

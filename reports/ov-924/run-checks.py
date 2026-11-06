#!/usr/bin/python3
"""Bounded owned FIFO/offline checks with reserve monitoring and raw exit capture."""
import os, sys, time, signal, subprocess, json
from pathlib import Path
root = Path(__file__).resolve().parents[2]
mode = sys.argv[1]
commands = {
    'mutations': [],
    'golden': ['test', '--offline', '-p', 'farcooler-client', '--lib', 'workflow_presentation_golden', '--', '--nocapture'],
    'focused': ['test', '--offline', '-p', 'farcooler-client', '--lib', 'workflow_presentation', '--', '--nocapture'],
    'jni': ['check', '--offline', '-p', 'farcooler-jni'],
    'web': ['test', '--offline', '--manifest-path', 'crates/web/Cargo.toml', '--test', 'workflow_presentation'],
    'wasm': ['check', '--offline', '--manifest-path', 'crates/web/Cargo.toml', '--target', 'wasm32-unknown-unknown'],
    'clippy': ['clippy', '--offline', '-p', 'farcooler-client', '--lib', '--', '-D', 'warnings'],
}
env = dict(os.environ, PATH='/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
    CARGO_TARGET_DIR=str(root / 'target'), CARGO_BUILD_JOBS='6', CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0', CARGO_INCREMENTAL='0')
if mode == 'golden': env['UPDATE_WORKFLOW_GOLDEN'] = '1'
args = commands[mode]
command = [str(root / 'scripts/heavy'), '--load-limit', '20', '--', '/Users/e-liang/.cargo/bin/cargo', *args]
if mode == 'mutations':
    command = [str(root / 'scripts/heavy'), '--load-limit', '20', '--', '/usr/bin/python3', str(root / 'reports/ov-924/mutations.py')]
minimum = None
started = time.monotonic()
with open(root / f'reports/ov-924/{mode}.log', 'a') as log:
    log.write(f'command={command!r}\n'); log.flush()
    child = subprocess.Popen(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    cancelled = False
    while child.poll() is None:
        st = os.statvfs(root); free = st.f_bavail * st.f_frsize
        minimum = free if minimum is None else min(minimum, free)
        if free < 10 * 1024**3 or time.monotonic() - started > 1800:
            os.killpg(child.pid, signal.SIGTERM); cancelled = True; break
        time.sleep(1)
    try: code = child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL); code = child.wait()
    log.write(f'\nraw_exit={code} cancelled={cancelled} minimum_free_bytes={minimum}\n')
print(json.dumps(dict(mode=mode, raw_exit=code, cancelled=cancelled, minimum_free_bytes=minimum)))
sys.exit(code if code >= 0 else 128-code)

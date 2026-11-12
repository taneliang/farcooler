#!/usr/bin/env python3
"""ov-1110 research capture: serialize measurements and retain load blockers."""
import datetime
import json
import os
import pathlib
import signal
import subprocess
import sys
import time
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = pathlib.Path(__file__).resolve().parent / 'resume-local'
OUT.mkdir(exist_ok=True)
sys.path.insert(0, str(ROOT / 'scripts'))
import heavy_lock

target = ROOT / 'target' / ('ov1110-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S'))
env = {**os.environ, 'PATH': '/Users/e-liang/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:' + os.environ['PATH'],
       'CARGO_BUILD_JOBS': '3', 'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_TARGET_DIR': str(target)}
receipt = {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
           'limit': 20, 'env': {k: env[k] for k in ['PATH', 'CARGO_BUILD_JOBS', 'CARGO_PROFILE_DEV_DEBUG', 'CARGO_TARGET_DIR']},
           'cold_target_existed': target.exists(), 'runs': []}

def save():
    (OUT / 'results.json').write_text(json.dumps(receipt, indent=2) + '\n')

def sample():
    return {'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
            'load_1_5_15': list(os.getloadavg())}

commands = [
    ('cold', ['cargo', 'build', '--workspace', '--locked', '--offline', '--timings']),
    ('warm', ['cargo', 'build', '--workspace', '--locked', '--offline', '--timings']),
    ('nextest', ['/usr/bin/python3', 'scripts/test-leak-check.py', '--', '/usr/bin/python3',
                 'scripts/tmux-leak-check.py', '--', 'cargo', 'nextest', 'run', '--workspace',
                 '--locked', '--offline', '--profile', 'ci', '--test-threads', '3', '--status-level', 'pass']),
    ('landing-gate', ['scripts/landing-gate', '--base', 'd986f3083730db7cda3b91caf5e536ba321b7c1e', '--keep-target']),
]
save()
with heavy_lock.machine_lock(str(ROOT), 'ov-1110 controlled baseline'):
    for label, cmd in commands:
        run = {'label': label, 'command': cmd, 'preflight': [], 'samples': []}
        receipt['runs'].append(run)
        while True:
            reading = sample()
            run['preflight'].append(reading)
            save()
            print(label, 'preflight', reading, flush=True)
            if max(reading['load_1_5_15']) < 20:
                break
            time.sleep(30)
        start = time.monotonic()
        with (OUT / (label + '.log')).open('w') as log:
            child = subprocess.Popen(cmd, cwd=ROOT, env={**env, heavy_lock.HELD_ENV: '1'},
                                     stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            while child.poll() is None:
                reading = sample()
                run['samples'].append(reading)
                if max(reading['load_1_5_15']) >= 20:
                    run['blocker'] = reading
                    # Only this newly created session/process group is signaled.
                    os.killpg(child.pid, signal.SIGINT)
                    try:
                        child.wait(timeout=20)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGTERM)
                        try:
                            child.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            os.killpg(child.pid, signal.SIGKILL)
                            child.wait()
                    break
                save()
                time.sleep(2)
        run.update(seconds=time.monotonic()-start, exit=child.wait(), end=sample())
        run['controlled'] = 'blocker' not in run and max(run['end']['load_1_5_15']) < 20
        if label in ('cold', 'warm'):
            timing = target / 'cargo-timings/cargo-timing.html'
            if timing.exists():
                with zipfile.ZipFile(OUT / (label + '-timing.zip'), 'w', zipfile.ZIP_DEFLATED) as archive:
                    archive.write(timing, label + '-timing.html')
        save()
        print(label, 'finished', run['seconds'], run['exit'], 'controlled', run['controlled'], flush=True)
        if 'blocker' in run:
            break

#!/usr/bin/python3
"""Bounded configured-adapter checks under the normal shared-target FIFO."""
import hashlib, json, os, shutil, signal, subprocess, sys, time
from pathlib import Path
root = Path(__file__).resolve().parents[2]
target = root.parent / 'ov-923-workflow-runtime/target'
mode = sys.argv[1]
args = {
    'binding-red': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'workflow_coding_foreign_provider_binding', '--', '--nocapture'],
    'binding-green': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'workflow_coding_foreign_provider_binding', '--', '--nocapture'],
    'focused': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'workflow_coding', '--', '--nocapture'],
    'runtime': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'workflow_', '--', '--nocapture'],
    'gate': ['test', '--offline', '-p', 'farcooler-daemon', '--lib', 'landing_job', '--', '--nocapture'],
    'lint': ['clippy', '--offline', '-p', 'farcooler-daemon', '--all-targets', '--', '-D', 'warnings'],
}[mode]
env = {**os.environ, 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
       'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '6', 'CARGO_PROFILE_DEV_DEBUG': '0',
       'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0'}
paths = ['crates/daemon/src/effect_runner.rs', 'crates/daemon/src/workflow_runtime.rs',
         'crates/daemon/src/workflow_runtime/adapters.rs', 'crates/daemon/src/workflow_runtime/wire_commands.rs',
         'crates/daemon/src/workflow_runtime/coding.rs', 'crates/daemon/src/workflow_runtime/coding_execute.rs',
         'crates/daemon/src/workflow_runtime/coding_tests.rs', 'crates/daemon/src/workflow_runtime/tests.rs']
def hashes():
    return {p: hashlib.sha256((root/p).read_bytes()).hexdigest() for p in paths if (root/p).exists()}
before = hashes()
command = ['/usr/bin/python3', str(root/'scripts/heavy'), '--load-limit', '20', '--', '/Users/e-liang/.cargo/bin/cargo', *args]

start = time.monotonic()
minimum = shutil.disk_usage(root).free
stopped = None
with (root/f'reports/ov-928/{mode}.log').open('w') as log:
    child = subprocess.Popen(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        while child.poll() is None:
            minimum = min(minimum, shutil.disk_usage(root).free)
            if minimum < 10*1024**3 or time.monotonic()-start > 1800:
                stopped = 'reserve/deadline'
                break
            time.sleep(1)
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try: child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        code = child.wait()
    log.write(f'\nexit={code}, stopped={stopped}\n')
after = hashes()
record = dict(command=command, target=str(target), profile='debug0/test0/incremental0/jobs6/offline',
              exit=code, stopped=stopped, seconds=time.monotonic()-start, minimum_free=minimum,
              head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
              before=before, after=after, source_unchanged=before == after)
(root/f"reports/ov-928/{mode}.json").write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps({k: record[k] for k in ['exit', 'stopped', 'seconds', 'source_unchanged']}), flush=True)
sys.exit(code if not stopped and before == after else 1)

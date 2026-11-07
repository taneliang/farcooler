#!/usr/bin/python3
"""Bounded configured-adapter checks under the normal shared-target FIFO."""
import hashlib, json, os, shutil, signal, subprocess, sys, time
from pathlib import Path
root = Path(__file__).resolve().parents[2]
target = root.parent / 'ov-923-workflow-runtime/target'
mode = sys.argv[1]
label = sys.argv[2] if len(sys.argv) > 2 else mode
args = {
    'compile': ['check','--offline','-p','farcooler-daemon','-p','farcooler-cli','--all-targets'],
    'selected': [],
    'red-binding': ['test','--offline','-p','farcooler-daemon','--lib','runtime_selection_normal_explicit_start_keeps_private_paths','--','--nocapture'],
    'red-selector': ['test','--offline','-p','farcooler-daemon','--lib','runtime_selection_explicit_default_and_pre_effect_refusal','--','--nocapture'],
    'lint': ['clippy','--offline','-p','farcooler-daemon','-p','farcooler-cli','--all-targets','--','-D','warnings'],
}[mode]
env = {**os.environ, 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
       'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '6', 'CARGO_PROFILE_DEV_DEBUG': '0',
       'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0', 'RUST_TEST_THREADS': '1'}
fixture = Path('/private/tmp') / f'ov-937-check-{os.getpid()}'
fixture.mkdir(mode=0o700)
env.update(FARCOOLER_CONFIG=str(fixture/'config.toml'), CLAUDE_CONFIG_DIR=str(fixture/'claude'), CODEX_HOME=str(fixture/'codex'))
tracked=subprocess.check_output(['git','ls-files','-z','crates','Cargo.toml','Cargo.lock','scripts/heavy','scripts/heavy_lock.py'],cwd=root).decode().split('\0')
paths=sorted(set(p for p in tracked if p and (root/p).is_file()) | {str(p.relative_to(root)) for p in (root/'crates').rglob('*.rs') if 'target' not in p.parts})

def hashes():
    return {p: hashlib.sha256((root/p).read_bytes()).hexdigest() for p in paths if (root/p).exists()}
before = hashes()
executable = ['/usr/bin/python3', str(root/f'reports/ov-937/{mode}.py')] if mode == 'selected' else ['/Users/e-liang/.cargo/bin/cargo', *args]
command = ['/usr/bin/python3', str(root/'scripts/heavy'), '--load-limit', '20', '--', '/usr/bin/python3', str(root/'reports/ov-937/entry.py'), str(target), *executable]

start = time.monotonic()
minimum = shutil.disk_usage(root).free
stopped = None
assert minimum >= 10*1024**3
with (root/f'reports/ov-937/{label}.log').open('w') as log:
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
(root/f"reports/ov-937/{label}.json").write_text(json.dumps(record, indent=2)+'\n')
shutil.rmtree(fixture)
print(json.dumps({k: record[k] for k in ['exit', 'stopped', 'seconds', 'source_unchanged']}), flush=True)
sys.exit(code if not stopped and before == after else 1)

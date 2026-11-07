#!/usr/bin/python3
"""Bounded configured-adapter checks under the normal shared-target FIFO."""
import hashlib, json, os, shutil, signal, subprocess, sys, time, tempfile
from pathlib import Path
root = Path(__file__).resolve().parents[2]
target = root.parent / 'ov-923-workflow-runtime/target'
mode = sys.argv[1]
label = sys.argv[2] if len(sys.argv) > 2 else mode
args = {
    'focused': ['test','--offline','-p','farcooler-store','--lib','engine_cutover_','--','--nocapture'],
    'verified': [],
    'controls': [],
    'notices': ['test','--offline','-p','farcooler-daemon','--lib','workflow_notice','--','--nocapture'],
    'lint-both': ['clippy','--offline','-p','farcooler-store','-p','farcooler-daemon','--all-targets','--','-D','warnings'],
    'old-reader': [],
    'cli': [],
    'final': [],
    'negatives': [],
    'verification': [],
    'restored': [],
    'seal': [],
    'finish': [],
    'catalog': ['test','--offline','-p','farcooler-store','--lib','engine_cutover_schema_catalog','--','--nocapture'],
    'store': ['test','--offline','-p','farcooler-store','--lib','--','--nocapture'],
    'groups': ['test','--offline','-p','farcooler-daemon','--lib','workflow_runtime_','--','--nocapture'],
    'compile': ['check','--offline','-p','farcooler-store','--all-targets'],
    'affected': ['test','--offline','-p','farcooler-store','-p','farcooler-daemon','-p','farcooler-cli','workflow_','--','--nocapture'],
    'gate': ['test','--offline','-p','farcooler-store','--lib','--','startup_gate_','transfer_','owner_join_','--nocapture'],
    'lint': ['clippy','--offline','-p','farcooler-store','--all-targets','--','-D','warnings'],
}[mode]
env = {**os.environ, 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
       'CARGO_TARGET_DIR': str(target), 'CARGO_BUILD_JOBS': '6', 'CARGO_PROFILE_DEV_DEBUG': '0',
       'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0', 'RUST_TEST_THREADS': '1', 'OV936_RUN_LABEL': label, 'OV936_EVIDENCE_OUTPUT': str(root/'reports/ov-940/fixtures'/label), 'OV936_SCHEMA_OUTPUT': str(root/'reports/ov-940/schema65.json'), 'OV936_OLD_READER_FIXTURE': os.environ.get('OV936_OLD_READER_FIXTURE','')}
private=Path(tempfile.mkdtemp(prefix='ov940-check-',dir='/private/tmp'))
for part in ['runtime','claude','tmux']:(private/part).mkdir()
(private/'config.toml').write_text('')
fixture_env={'FARCOOLER_HOME':str(private/'runtime'),'CLAUDE_CONFIG_DIR':str(private/'claude'),'FARCOOLER_CONFIG':str(private/'config.toml'),'TMUX_TMPDIR':str(private/'tmux')}
env.update(fixture_env)
tracked=subprocess.check_output(['git','ls-files','-z','crates','Cargo.toml','Cargo.lock','scripts/heavy','scripts/heavy_lock.py'],cwd=root).decode().split('\0')
paths=sorted(set(p for p in tracked if p and (root/p).is_file()) | {str(p.relative_to(root)) for p in (root/'crates').rglob('*') if p.is_file() and p.suffix in ['.rs','.sql','.json','.toml'] and 'target' not in p.parts})

def hashes():
    return {p: hashlib.sha256((root/p).read_bytes()).hexdigest() for p in paths if (root/p).exists()}
before = hashes()
executable = ['/usr/bin/python3', str(root/f'reports/ov-940/{mode}.py')] if mode in ['verified','controls','old-reader','cli','final','negatives','verification','restored','seal','finish'] else ['/Users/e-liang/.cargo/bin/cargo', *args]
command = ['/usr/bin/python3', str(root/'scripts/heavy'), '--load-limit', '20', '--', '/usr/bin/python3', str(root/'reports/ov-940/entry.py'), str(target), *executable]

def interrupted(signum, frame):
    raise KeyboardInterrupt(f'supervisor received signal {signum}')
for signum in [signal.SIGTERM, signal.SIGHUP, signal.SIGINT]: signal.signal(signum,interrupted)
start = time.monotonic()
minimum = shutil.disk_usage(root).free
stopped = None
assert minimum >= 10*1024**3
with (root/f'reports/ov-940/{label}.log').open('w') as log:
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
              fixture_env=fixture_env, exit=code, stopped=stopped, seconds=time.monotonic()-start, minimum_free=minimum,
              head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(),
              before=before, after=after, source_unchanged=before == after)
(root/f"reports/ov-940/{label}.json").write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps({k: record[k] for k in ['exit', 'stopped', 'seconds', 'source_unchanged']}), flush=True)
sys.exit(code if not stopped and before == after else 1)

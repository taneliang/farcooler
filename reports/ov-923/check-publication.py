#!/usr/bin/python3
"""One controlled publication regression, restoration, affected runtime and lint."""
from pathlib import Path
import hashlib
import signal
import subprocess
root = Path(__file__).resolve().parents[2]
cargo = '/Users/e-liang/.cargo/bin/cargo'
def run(name, args, expected=0):
    with open(root / f'reports/ov-923/{name}.log', 'w') as log:
        code = subprocess.run([cargo, *args], cwd=root, stdout=log, stderr=subprocess.STDOUT).returncode
        log.write(f'\nexit={code}\n')
    print(f'{name}: exit={code}, expected={expected}', flush=True)
    if code != expected: raise RuntimeError(f'{name} returned {code}')
def test(name, pattern, expected=0):
    run(name, ['test', '--offline', '-p', 'farcooler-daemon', '--lib', pattern, '--', '--nocapture'], expected)
def interrupted(signum, frame):
    raise KeyboardInterrupt(f'signal {signum}')
signal.signal(signal.SIGTERM, interrupted)
path = root / 'crates/daemon/src/workflow_runtime/adapters.rs'
original = path.read_bytes()
assert original.count(b'if selected.is_err() {') == 1
try:
    path.write_bytes(original.replace(b'if selected.is_err() {', b'if !exists {'))
    test('publication-mutation', 'workflow_runtime_interrupted_base_publication_recovers_before_admission', 101)
finally:
    path.write_bytes(original)
    assert hashlib.sha256(path.read_bytes()).digest() == hashlib.sha256(original).digest()
test('publication-restored', 'workflow_runtime_interrupted_base_publication_recovers_before_admission')
test('publication-focused', 'workflow_runtime')
run('publication-clippy', ['clippy', '--offline', '-p', 'farcooler-daemon', '-p', 'farcooler-store', '--all-targets', '--', '-D', 'warnings'])

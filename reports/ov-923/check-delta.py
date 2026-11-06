#!/usr/bin/python3
"""Focused negative controls, exact restoration, retained primitive regressions and lint."""
from pathlib import Path
import hashlib
import signal
import subprocess
import sys
root = Path(__file__).resolve().parents[2]
cargo = '/Users/e-liang/.cargo/bin/cargo'
def run(name, args, expected=0):
    with open(root / f'reports/ov-923/delta-{name}.log', 'w') as log:
        code = subprocess.run([cargo, *args], cwd=root, stdout=log, stderr=subprocess.STDOUT).returncode
        log.write(f'\nexit={code}\n')
    print(f'{name}: exit={code}, expected={expected}', flush=True)
    if code != expected:
        raise RuntimeError(f'{name} returned {code}')
def daemon_test(name, test, expected=0):
    run(name, ['test', '--offline', '-p', 'farcooler-daemon', '--lib', test, '--', '--nocapture'], expected)
def interrupted(signum, frame):
    raise KeyboardInterrupt(f'signal {signum}')
signal.signal(signal.SIGTERM, interrupted)
mutations = [
    ('inbox', 'crates/daemon/src/workflow_runtime.rs',
     'self.decide(&DecisionSession { instance, principal }, request, time)',
     'self.decide(&DecisionSession { instance, principal: "intruder".into() }, request, time)',
     'workflow_runtime_owned_inbox_approval_cancel_and_idle'),
    ('idle', 'crates/daemon/src/workflow_runtime.rs',
     'None => std::future::pending::<()>().await',
     'None => tokio::time::sleep(Duration::from_millis(25)).await',
     'workflow_runtime_owned_inbox_approval_cancel_and_idle'),
    ('base', 'crates/daemon/src/workflow_runtime/adapters.rs',
     'git(&self.root.join("repo"), &["rev-parse", "refs/heads/landed"]).await?',
     'self.base.clone()', 'workflow_runtime_second_coding_item_pins_new_base_and_recovers'),
]
for name, relative, before, after, test in mutations:
    path = root / relative
    original = path.read_bytes()
    assert original.count(before.encode()) == 1
    try:
        path.write_bytes(original.replace(before.encode(), after.encode()))
        daemon_test(f'mutation-{name}', test, 101)
    finally:
        path.write_bytes(original)
        assert hashlib.sha256(path.read_bytes()).digest() == hashlib.sha256(original).digest()
    daemon_test(f'restored-{name}', test)
daemon_test('final-focused', 'workflow_runtime')
run('store', ['test', '--offline', '-p', 'farcooler-store', 'workflow', '--', '--nocapture'])
run('clippy', ['clippy', '--offline', '-p', 'farcooler-daemon', '-p', 'farcooler-store', '--all-targets', '--', '-D', 'warnings'])

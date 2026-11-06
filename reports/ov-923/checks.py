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
    with open(root / f'reports/ov-923/{name}.log', 'w') as log:
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
    ('binding', 'crates/daemon/src/workflow_runtime/adapters.rs',
     'journal.binding != *binding', 'false', 'workflow_runtime_generated_crash_receipt_and_replay_properties'),
    ('advance', 'crates/daemon/src/workflow_runtime.rs',
     'self.last_occupancy.as_ref() != Some(&fingerprint)', 'false',
     'workflow_runtime_release_and_recovery_advance_blocked_work_without_churn'),
    ('decision', 'crates/daemon/src/workflow_runtime.rs',
     'Grant::Decision { principal: session.principal.clone() }', 'Grant::Decision { principal: "owner".into() }',
     'workflow_runtime_approval_policies_and_authentication'),
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
daemon_test('retained-gate', 'landing_job')
run('store', ['test', '--offline', '-p', 'farcooler-store', 'workflow', '--', '--nocapture'])
run('clippy', ['clippy', '--offline', '-p', 'farcooler-daemon', '-p', 'farcooler-store', '--all-targets', '--', '-D', 'warnings'])

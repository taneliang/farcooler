#!/usr/bin/python3
"""Two scoped fault insertions; exact source restored even on failed checks."""
from pathlib import Path
import hashlib, subprocess
root = Path(__file__).resolve().parents[3]
source = root / 'crates/client/src/workflow_presentation.rs'
cargo = '/Users/e-liang/.cargo/bin/cargo'
original = source.read_bytes()

def run(label, test, expected):
    with open(root / f'reports/ov-924/labels/{label}.log', 'w') as log:
        args = [cargo, 'test', '--offline', '-p', 'farcooler-client', '--lib', test, '--', '--nocapture']
        log.write(f'command={args!r}\n'); log.flush()
        code = subprocess.call(args, cwd=root, stdout=log, stderr=subprocess.STDOUT)
        log.write(f'\nraw_exit={code} expected={expected}\n')
    print(label, code, expected, flush=True)
    if code != expected: raise SystemExit(1)

cases = [
    ('display-copy', b'"Evidence Invalidated", "The latest evidence no longer matches the current candidate or policy."',
        b'"", ""', 'workflow_presentation_generic_display_slots'),
    ('stale', b'if requested.guard != digest(&(digest(&("workflow-presentation/v2", state, auth)), item))',
        b'if false', 'workflow_presentation_stale_and_auth_boundaries'),
    ('evidence', b'item.candidate.as_ref() == Some(&e.candidate) && item.policy_pin == e.policy_pin',
        b'item.policy_pin == e.policy_pin', 'workflow_presentation_invalidated_inspection_and_rework'),
]
for label, old, new, test in cases:
    source = root / ('crates/client/src/workflow_presentation/display.rs' if label == 'display-copy'
        else 'crates/client/src/workflow_presentation.rs')
    original = source.read_bytes()
    assert original.count(old) == 1
    try:
        source.write_bytes(original.replace(old, new))
        run('mutation-' + label, test, 101)
    finally:
        source.write_bytes(original)
    assert hashlib.sha256(source.read_bytes()).digest() == hashlib.sha256(original).digest()
    run('restored-' + label, test, 0)
run('final-focused', 'workflow_presentation', 0)

#!/usr/bin/python3
"""Finite original-source controls, under the outer owned FIFO supervisor."""
import hashlib, json, subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[2]
cargo = '/Users/e-liang/.cargo/bin/cargo'
cases = [
    ('plan-original', 'crates/store/src/plan_tests.rs', 'farcooler-store', 'plan::tests::no_existing_table_gained_a_column'),
    ('notice-original', 'crates/store/src/workflow_notices.rs', 'farcooler-store', 'workflow_notice_migration_never_backfills_historical_events_or_rewrites_bytes'),
    ('group-reaped', 'crates/daemon/src/effect_runner.rs', 'farcooler-daemon', 'workflow_runtime_exited_leader_keeps_group_owned_until_descendant_stops'),
]
receipts = []
for label, name, crate, test in cases:
    path = root/name
    green = path.read_bytes()
    mutant = (green.replace(b'tokio::time::timeout(limit, exited(pid))',
        b'tokio::time::timeout(limit, async { child.wait().await.map(|_| ()) })')
        if label == 'group-reaped' else subprocess.check_output(['git','show','HEAD:'+name],cwd=root))
    assert mutant != green
    command = [cargo,'test','--offline','-p',crate,'--lib',test,'--','--nocapture']
    try:
        path.write_bytes(mutant)
        with (root/f'reports/ov-940/{label}.log').open('w') as log:
            result = subprocess.run(command,cwd=root,stdout=log,stderr=subprocess.STDOUT)
    finally:
        path.write_bytes(green)
    output = (root/f'reports/ov-940/{label}.log').read_text()
    semantic = ('test result: FAILED. 0 passed; 1 failed;' in output and test.split('::')[-1] in output)
    receipts.append(dict(label=label,command=command,exit=result.returncode,semantic=semantic,
        before=hashlib.sha256(green).hexdigest(),mutant=hashlib.sha256(mutant).hexdigest(),
        restored=hashlib.sha256(path.read_bytes()).hexdigest()))
    print(json.dumps(receipts[-1]),flush=True)
    (root/'reports/ov-940/control-results.json').write_text(json.dumps(receipts,indent=2)+'\n')
    assert result.returncode != 0 and semantic, 'control did not execute exactly one behavioral failure'

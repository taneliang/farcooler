#!/usr/bin/python3
"""Sequential focused checks under the parent heavy route; mutations always restored."""
import hashlib
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[2]
cargo = '/Users/e-liang/.cargo/bin/cargo'
report = root / 'reports/ov-922'

def run(label, args, expected=0):
    with open(report / (label + '.log'), 'w') as log:
        result = subprocess.run([cargo] + args, cwd=root, stdout=log, stderr=subprocess.STDOUT)
        log.write(f'\nexit={result.returncode} expected={expected}\n')
    print(f'{label}: exit={result.returncode}, expected={expected}', flush=True)
    if result.returncode != expected:
        raise SystemExit(1)

def mutation(label, relative, old, new, test, package="farcooler-workflow"):
    source = root / relative
    original = source.read_bytes()
    assert original.count(old) == 1
    try:
        source.write_bytes(original.replace(old, new))
        run('mutation-' + label, ['test', '--offline', '-p', package,
            test, '--', '--nocapture'], 101)
    finally:
        source.write_bytes(original)
    assert hashlib.sha256(source.read_bytes()).digest() == hashlib.sha256(original).digest()
    run('restored-' + label, ['test', '--offline', '-p', 'farcooler-workflow',
        test, '--', '--nocapture'])

mutation('stale-revision', 'crates/workflow/src/reducer.rs',
    b'        || input.expected_revision != state.revision\n', b'',
    'workflow_stale_revision_epoch_source_and_duplicate_refused')
mutation('mandatory-review', 'crates/workflow/src/definition.rs',
    b'        // Sequence order is definition policy, not a global coding/reviewer policy.',
    b'        if !self.stages.iter().any(|s| matches!(s.kind, StageKind::Review(_))) { return Err(Refusal::Definition); }',
    'workflow_definition_driven_no_review_multiple_checks_and_approvals')
mutation('blocked-reservation', 'crates/workflow/src/reducer.rs',
    b'Phase::Blocked | Phase::Done | Phase::Failed | Phase::Cancelled)\n}',
    b'Phase::Done | Phase::Failed | Phase::Cancelled)\n}',
    'workflow_blocked_shared_resource_does_not_reserve_and_releases_one_waiter')
mutation('fixed-rework-index', 'crates/workflow/src/reducer.rs',
    b'item.stage = item.definition.stages.iter().position(|stage|\n                Some(&stage.id) == item.definition.rework_target.as_ref()).ok_or(Refusal::Definition)?;',
    b'item.stage = 1;', 'workflow_noncoding_named_actions_and_stable_rework_target')
mutation('candidate-producer', 'crates/workflow/src/definition.rs',
    b'                    | StageKind::Tracker | StageKind::Evaluate(_) if !candidate_available => {',
    b'                    | StageKind::Tracker | StageKind::Evaluate(_) if false => {',
    'workflow_definition_rejects_missing_candidate_paths')
mutation('cross-workspace-occupancy', 'crates/store/src/workflow.rs',
    b'} else { step(&state, &admitted) };', b'} else { step(&state, input) };',
    'workflow_cross_workspace_dependents_cannot_block_completion_and_replay', 'farcooler-store')
run('store-regressions', ['test', '--offline', '-p', 'farcooler-store', '--lib', '--', '--nocapture'])
run('clippy', ['clippy', '--offline', '-p', 'farcooler-workflow', '-p', 'farcooler-store',
    '--all-targets', '--', '-D', 'warnings'])

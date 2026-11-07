#!/usr/bin/python3
"""Each mutant puts one reviewed defect back. Its named test must go red, then
the exact source is restored. Runs under check.py's heavy admission."""
import json, os, subprocess, sys
from pathlib import Path
root = Path(__file__).resolve().parents[2]
T = 'crates/store/src/engine_cutover/transfer.rs'
O = 'crates/store/src/engine_cutover/owners.rs'
E = 'crates/store/src/engine_cutover.rs'
V = 'crates/store/src/engine_cutover/verify.rs'
S = 'crates/store/src/engine_cutover/startup.rs'
A = 'crates/daemon/src/cutover_admin.rs'
R = 'crates/daemon/src/rpc/recovery.rs'
P = 'crates/daemon/src/rpc.rs'
store, daemon = 'farcooler-store', 'farcooler-daemon'
MUTANTS = [
 ('B1-orphan', T, 'let hold = held(&dir)?;\n', 'let hold = held(&dir)?;\n    if hold.is_none() { create_artifacts(&dir)?; }\n    let hold = hold.or(Some(Hold { attempt: owners.attempt().into() }));\n', store, 'source60_unknown_catalog_refuses'),
 ('B1-no-recovery-prepare', A, 'Cutover::Prepare { attempt } => prepare(root, attempt),', 'Cutover::Prepare { .. } => Err(Refused("live only".into())),', daemon, 'recovery_prepares_then_recovers'),
 ('B1-no-restore', T, 'fs::rename(&restore, db)?;', 'let _ = &restore;', store, 'recover_returns_a_frozen_attempt'),
 ('B2-capture-gap', T, [('let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;\n    let plan = preflight(&tx)?;', 'let plan = preflight(&conn)?;'),
    ('super::authority(&tx)?.phase != "legacy"', 'super::authority(&conn)?.phase != "legacy"'), ('super::resource_generations(&tx)?', 'super::resource_generations(&conn)?'),
    ('read_compatible_down_to(&tx)', 'read_compatible_down_to(&conn)'),
    ('    // Atomic upgrade: DDL + version', '    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;\n    // Atomic upgrade: DDL + version')], None, store, 'transfer_no_writer_lands'),
 ('B2-store-holds-nothing', O, 'if try_flock(&file, libc::LOCK_SH)? { Ok(file) }', 'if true { Ok(file) }', store, 'owner_lock_any_open_store'),
 ('B2-lost-write', T, 'if current != replayed(&dir, &baseline)? {', 'if current.is_empty() && replayed(&dir, &baseline)?.is_empty() {', store, 'recover_refuses_committed_truth'),
 ('B3-proof-without-lock', O, 'if !try_flock(&lock, libc::LOCK_EX)? {', 'if false {', store, 'owner_lock_any_open_store'),
 ('B3-any-attempt', E, 'if frozen.is_some_and(|id|id!=format!("frozen:{attempt}")) {', 'if frozen.is_some_and(|id|id.is_empty()&&!attempt.is_empty()) {', store, 'owner_lock_proof_refuses'),
 ('B3-any-file', O, 'std::fs::metadata(db).map(|m| (m.dev(), m.ino()) == identity).unwrap_or(false)', 'std::fs::metadata(db).map(|m| (m.dev(), m.ino()) == identity).unwrap_or(false) || true', store, 'owner_lock_proof_refuses'),
 ('B4-recovery-ignores-seat', R, 'if !self.caller_is_a_person() {', 'if false && !self.caller_is_a_person() {', daemon, 'recovery_refuses_every_caller'),
 ('B4-pid-reuse', R, '(self.start_of)(pid) == Some(start) && (self.seat_of)', '(self.seat_of)', daemon, 'recovery_a_reused_pid'),
 ('B4-no-recheck', A, '    still_allowed()?;\n', '    let _ = still_allowed;\n', daemon, 'live_hold_drains'),
 ('B4-agent-prepares', P, 'let admission = crate::workflow_runtime::Admission::capture(svc.clone(), self.peer.clone(), self.session.clone(),\n                            self.caller_pid, self.caller_pane.clone(), crate::workflow_runtime::Admission::HOST_WIDE).await?;\n                        admission.require_person()?;\n                        let held = crate::cutover_admin::hold(svc, attempt, || admission.recheck_now()\n                            .map_err(|_| crate::cutover_admin::Refused("the caller lost access during the drain".into()))).await;', 'let held = crate::cutover_admin::hold(svc, attempt, || Ok(())).await;', daemon, 'live_prepare_refuses_an_agent'),
 ('B5-no-schema60', E, 'if !matches!(registry,60|65..=67) {', 'if !matches!(registry,65..=67) {', store, 'source60_'),
 ('B5-catalog-unchecked', V, 'catalog_hash(after)==SOURCE_SCHEMA_HASH\n        && after', 'after', store, 'source60_verify_catches'),
 ('B5-any-catalog', E, 'if registry==60 && catalog!=SOURCE60_SCHEMA_HASH {', 'if registry==60 && catalog.is_empty() {', store, 'source60_unknown_catalog_refuses'),
 ('B6-no-identity', T, 'if baseline.root != root || baseline.database != database || baseline.identity != (meta.dev(), meta.ino()) {', 'let _ = (&root, &database, &meta);\n    if baseline.database.is_empty() {', store, 'baseline_binds_root'),
 ('B6-no-frozen-attempt', T, 'if manifest.as_deref() != Some(baseline.raw_hash.as_str()) {', 'if manifest.is_some() && baseline.raw_hash.is_empty() {', store, 'baseline_of_another_attempt'),
 ('B6-no-attempt-row', T, '    tx.execute("INSERT INTO engine_cutover_attempts VALUES(?1,?2,?3)", rusqlite::params![frozen_id(&baseline.attempt), baseline.raw_hash, baseline.owners_hash])?;\n', '', store, 'baseline_binds_root'),
 ('M1-no-settle', S, 'if std::fs::symlink_metadata(artifact_dir(db)).is_err() { settle_journal(db)?; }\n', '', store, 'hot_journal_is_rolled_back'),
 ('M1-journal-by-restore', T, 'if fs::metadata(&journal).is_ok_and(|m| m.is_file() && m.len() == 0) { fs::remove_file(&journal)?; }\n', '', store, 'hot_journal_never_lands'),
]
only = [a for a in os.environ.get('OV940_MUTANTS', '').split() if a]
env = dict(os.environ)
results = {}
for name, path, old, new, package, test in MUTANTS:
    if only and name not in only: continue
    file = root / path
    original = file.read_bytes()
    text = original.decode()
    for before, after in (old if isinstance(old, list) else [(old, new)]):
        assert text.count(before) >= 1, name
        text = text.replace(before, after, 1)
    file.write_text(text)
    try:
        run = subprocess.run(['/Users/e-liang/.cargo/bin/cargo', 'test', '--offline', '-p', package, '--lib', '--', test], cwd=root, env=env, capture_output=True, text=True)
    finally:
        file.write_bytes(original)
    out = run.stdout + run.stderr
    compiled = 'error[' not in out and 'could not compile' not in out
    failed = sorted(l.split()[1] for l in out.splitlines() if l.startswith('test ') and l.endswith('FAILED'))
    passed = sum(1 for l in out.splitlines() if l.startswith('test ') and l.endswith(' ok'))
    results[name] = {'test': test, 'exit': run.returncode, 'compiled': compiled, 'failed': failed, 'passed': passed}
    print(name, 'RED' if compiled and failed else 'NOT RED', failed if failed else out[-600:], flush=True)
path = root / 'reports/ov-940/fix-controls-results.json'
results = {**(json.loads(path.read_text()) if path.exists() else {}), **results}
path.write_text(json.dumps(results, indent=2) + '\n')
assert all(r['compiled'] and r['failed'] for r in results.values()), 'a mutant did not go red'
print('all red', len(results), flush=True)

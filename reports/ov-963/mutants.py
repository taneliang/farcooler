#!/usr/bin/python3
"""Each mutant puts one defect at one durable boundary. Its build must succeed
and its named boundaries or test must go red; a setup failure is not red. The
exact source is restored after each. Every boundary needs a mutant, or this
refuses to run. Usage: scripts/heavy -- /usr/bin/python3 reports/ov-963/mutants.py"""
import json, os, re, subprocess, sys
from pathlib import Path
root = Path(__file__).resolve().parents[2]
T = 'crates/store/src/engine_cutover/transfer.rs'
C = 'crates/store/src/engine_cutover/closing.rs'
O = 'crates/store/src/engine_cutover/owners.rs'
S = 'crates/store/src/engine_cutover/startup.rs'
E_ = 'crates/store/src/engine_cutover.rs'
V = 'crates/store/src/engine_cutover/verify.rs'
A = 'crates/daemon/src/cutover_admin.rs'
R = 'crates/daemon/src/rpc/recovery.rs'
N = 'crates/daemon/src/refusal.rs'
M = 'crates/daemon/src/main.rs'
GATE = 'if std::fs::symlink_metadata(artifact_dir(db)).is_ok() {\n        return refuse("cutover transfer artifacts exist; use authenticated recovery");\n    }'
SETTLE = 'if std::fs::symlink_metadata(artifact_dir(db)).is_err() { settle_journal(db)?; }'
PUBLISH_BASELINE = 'publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?, "begin.torn-baseline")?;\n    reach("begin.before-intent");\n'
BOUNDARIES = re.findall(r'"([a-z]+\.[a-z-]+)"', (root / 'crates/store/src/engine_cutover/boundary.rs').read_text().split('];')[0])
# name, [(file, old, new)], boundaries that must go red (matrix) or a test filter
MUTANTS = [
 ('hold-moves-first', [(T, 'reach("hold.before");\n    create_artifacts(&dir)?;', 'create_artifacts(&dir)?;\n    reach("hold.before");')], ['hold.before']),
 ('gate-needs-a-complete-intent', [(S, 'if std::fs::symlink_metadata(artifact_dir(db)).is_ok() {\n', 'if std::fs::symlink_metadata(artifact_dir(db).join("intent.json")).is_ok() {\n')],
    ['hold.torn', 'hold.after', 'begin.before-baseline-db', 'begin.after-baseline-db', 'begin.torn-baseline', 'begin.before-intent']),
 ('gate-ignores-artifacts', [(S, GATE, ''), (S, SETTLE, 'settle_journal(db)?;')], ['begin.torn-intent', 'begin.after-intent', 'begin.in-upgrade', 'release.after', 'finish.before-rename', 'recover.after-rename']),
 ('publish-is-not-torn', [(T, 'f.write_all(first)?;\n    reach(torn);\n    f.write_all(rest)?;', 'f.write_all(first)?;\n    f.write_all(rest)?;\n    reach(torn);')], ['hold.torn', 'begin.torn-baseline', 'begin.torn-intent', 'commit.torn-snapshot', 'journal.torn']),
 ('baseline-copy-after-its-record', [(T, 'copy.write_all(&bytes)?;\n    copy.sync_all()?;\n    let baseline = Baseline', 'let baseline = Baseline'),
    (T, 'reach("begin.after-baseline-db");\n', 'reach("begin.after-baseline-db");\n    copy.write_all(&bytes)?;\n    copy.sync_all()?;\n'), (T, 'if db_hash(&dir.join(BASELINE_DB))? != baseline.db_hash { return refuse("baseline copy differs from source"); }\n', '')], ['begin.after-baseline-db']),
 ('intent-before-baseline', [(T, PUBLISH_BASELINE, ''), (T, 'publish(&dir, INTENT, &serde_json::to_vec(&intent)?, "begin.torn-intent")?;\n', 'publish(&dir, INTENT, &serde_json::to_vec(&intent)?, "begin.torn-intent")?;\n    ' + PUBLISH_BASELINE)], ['begin.before-intent', 'begin.torn-intent']),
 ('upgrade-before-intent', [(T, 'publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?, "begin.torn-baseline")?;\n', 'freeze(&tx, plan.registry, &baseline)?;\n    tx.commit()?;\n    publish(&dir, BASELINE, &serde_json::to_vec(&baseline)?, "begin.torn-baseline")?;\n'),
    (T, 'freeze(&tx, plan.registry, &baseline)?;\n    reach("begin.in-upgrade");\n    tx.commit()?;\n', 'reach("begin.in-upgrade");\n')], ['begin.torn-baseline', 'begin.before-intent', 'begin.after-intent']),
 ('upgrade-commits-early', [(T, 'reach("begin.in-upgrade");\n    tx.commit()?;\n', 'tx.commit()?;\n    reach("begin.in-upgrade");\n')], ['begin.in-upgrade']),
 ('floor-after-the-commit', [(T, '    crate::compat::stamp_compatible_down_to(tx).map_err(|e| Error::Refused(e.to_string()))?;\n    let changed', '    let changed'),
    (T, 'reach("begin.after-upgrade");\n', 'reach("begin.after-upgrade");\n    crate::compat::stamp_compatible_down_to(&conn).map_err(|e| Error::Refused(e.to_string()))?;\n')], ['begin.after-upgrade']),
 ('frozen-attempt-after-the-commit', [(T, '    tx.execute("INSERT INTO engine_cutover_attempts VALUES(?1,?2,?3)", rusqlite::params![frozen_id(&baseline.attempt), baseline.raw_hash, baseline.owners_hash])?;\n', ''),
    (T, 'reach("begin.after-upgrade");\n', 'reach("begin.after-upgrade");\n    conn.execute("INSERT INTO engine_cutover_attempts VALUES(?1,?2,?3)", rusqlite::params![frozen_id(&baseline.attempt), baseline.raw_hash, baseline.owners_hash])?;\n')], ['begin.after-upgrade']),
 ('killed-journal-stays', [(T, 'if read == 0 || first[0] == 0 {', 'if read == 0 {')], ['begin.in-upgrade']),
 ('backup-number-is-fixed', [(C, 'for number in 1.. {', 'for number in 1..2 {'), (C, 'Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}\n', '')], ['commit.before-copy', 'commit.after-copy', 'commit.torn-snapshot', 'commit.after-snapshot', 'commit.after-restore', 'journal.torn', 'commit.after-permit']),
 ('copy-before-its-boundary', [(C, '    reach("commit.before-copy");\n    let bytes = fs::read(db)?;\n    write_private(&dir.join("db"), &bytes)?;\n', '    let bytes = fs::read(db)?;\n    write_private(&dir.join("db"), &bytes)?;\n    reach("commit.before-copy");\n')], ['commit.before-copy']),
 ('snapshot-before-the-copy', [(C, '    reach("commit.after-copy");\n', ''), (C, '    reach("commit.after-snapshot");\n', '    reach("commit.after-copy");\n    reach("commit.after-snapshot");\n')], ['commit.after-copy']),
 ('no-snapshot', [(C, 'transfer::publish(&dir, "SNAPSHOT.json", &serde_json::to_vec(&snapshot)?, "commit.torn-snapshot")?;\n', '')], ['commit.torn-snapshot', 'commit.after-snapshot', 'commit.after-restore']),
 ('restore-copy-left-behind', [(C, '    fs::remove_file(&restore)?;\n', '')], ['commit.after-restore']),
 ('permit-not-journaled', [(C, '    record(db, &baseline, &format!("backed_up-{number}"), &backup_hash)?;\n', '    let _ = number;\n')], ['journal.torn', 'commit.after-permit', 'commit.in-mapping', 'commit.after-mapping']),
 ('mapping-commits-early', [(E_, 'boundary::reach("commit.in-mapping");\n        tx.commit()?;\n', 'tx.commit()?;\n        boundary::reach("commit.in-mapping");\n')], ['commit.in-mapping']),
 ('committed-journal-first', [(C, '    let store = startup::open_recovery(db, &owners)?;\n    store.engine_commit(', '    record_up_to(db, &baseline, "committed")?;\n    let store = startup::open_recovery(db, &owners)?;\n    store.engine_commit(')], ['commit.in-mapping', 'commit.after-mapping']),
 ('commit-does-not-resume', [(C, 'record_up_to(db, &baseline, "committed")?;\n            return Ok(Committed { attempt: baseline.attempt, resumed: true, backup: None });', 'return refuse("already committed");')], ['commit.after-mapping', 'commit.after-journal', 'verify.after']),
 ('recover-rewinds-a-commit', [(T, 'Some("quiesced") => {', 'Some("quiesced" | "committed") => {'), (T, 'if current != replayed(&dir, &baseline)? {', 'if current.is_empty() && replayed(&dir, &baseline)?.is_empty() {')], ['commit.after-mapping', 'commit.after-journal', 'verify.after', 'release.in-transaction']),
 ('verified-journal-first', [(A, '    store.engine_verify(&plan).map_err(refuse)?;\n    boundary::reach("verify.after");\n    closing::record_up_to(&db, &baseline, "verified").map_err(refuse)?;\n', '    closing::record_up_to(&db, &baseline, "verified").map_err(refuse)?;\n    store.engine_verify(&plan).map_err(refuse)?;\n    boundary::reach("verify.after");\n')], ['verify.after']),
 ('release-commits-early', [(V, 'boundary::reach("release.in-transaction");\n        tx.commit()?;\n', 'tx.commit()?;\n        boundary::reach("release.in-transaction");\n')], ['release.in-transaction']),
 ('released-journal-first', [(A, '    store.engine_release(&plan, owners).map_err(refuse)?;\n    closing::record_up_to(&db, &baseline, "released").map_err(refuse)?;\n', '    closing::record_up_to(&db, &baseline, "released").map_err(refuse)?;\n    store.engine_release(&plan, owners).map_err(refuse)?;\n')], ['release.in-transaction', 'release.after']),
 ('finish-keeps-no-journal', [(T, '    super::closing::record_up_to(db, &baseline, "resolved")?;\n', '')], ['finish.before-rename']),
 ('gate-refuses-released', [(S, '"legacy" | "released" => Ok(Admission::Existing)', '"legacy" => Ok(Admission::Existing)')], ['finish.after-rename']),
 ('restore-overwrites-in-place', [(T, 'let restore = dir.join(RESTORE_DB);\n            let mut copy = OpenOptions::new().write(true).create(true).truncate(true).open(&restore)?;', 'let restore = db.to_path_buf();\n            let mut copy = OpenOptions::new().write(true).open(&restore)?;\n            copy.set_len(0)?;')], ['recover.before-rename']),
 ('recover-drops-the-baseline', [(T, '            reach("recover.after-rename");\n', '            fs::remove_file(dir.join(BASELINE_DB))?;\n            reach("recover.after-rename");\n')], ['recover.after-rename']),
 ('recover-keeps-the-artifacts', [(T, '            let evidence = retire(db, "abandoned")?;\n            reach("recover.after-retire");\n', '            let evidence = dir.clone();\n            reach("recover.after-retire");\n')], ['recover.after-retire']),
 ('stdio-says-nothing', [(N, 'matches!(err, DomainError::NewerData | DomainError::CutoverHeld)', 'matches!(err, DomainError::NewerData)')], ['hold.after']),
 ('main-skips-the-gate', [(M, 'if let Err(e) = farcooler_daemon::cutover_gate::admit(selected.root()) {', 'if let Err(e) = Ok::<(), farcooler_core::DomainError>(()) {')], ['hold.after']),
 ('store-skips-the-gate', [(S, '    ordinary_admission(db).map_err(refused)?;\n    Ok(lock)', '    Ok(lock)')], ['hold.after']),
 ('scan-sees-no-one', [(O, 'let others = settled_holders(db)?;', 'let others: Vec<i32> = Vec::new(); let _ = settled_holders;')], 'an_old_build_cannot_lose_its_writes_across_recover'),
 ('recovery-never-ends', [(R, '_ = done.notified() => Ok(true),', '_ = done.notified() => Ok(false),')], 'a_person_takes_a_real_runner'),
 ('no-verify-inside-commit', [(E_, '        verify::check(plan,&tx)?;\n', '')], ('store', 'commit_that_could_not_verify')),
 ('unknown-schema-commits', [(C, 'if plan.quarantines.iter().any(|q| q.resource == "schema:unknown") {', 'if plan.quarantines.iter().any(|q| q.resource == "never") {')], ('store', 'commit_a_schema_release_would_refuse')),
 ('lost-backup-passes', [(C, '    refuse("the attempt\'s closed backup is missing")', '    Ok(PathBuf::new())')], ('store', 'verify_and_release_need_the_backup')),
]
missing = [b for b in BOUNDARIES if not any(isinstance(m[2], list) and b in m[2] for m in MUTANTS)]
assert not missing, f'boundaries with no mutant: {missing}'
only = set(sys.argv[1:])
env = {**os.environ, 'PATH': f"{os.environ['HOME']}/.cargo/bin:{os.environ['PATH']}", 'CARGO_BUILD_JOBS': '6', 'RUST_BACKTRACE': '0'}
out = root / 'reports/ov-963/mutants-results.json'
results = json.loads(out.read_text()) if out.exists() else {}
for name, edits, want in MUTANTS:
    if only and name not in only: continue
    saved = {}
    try:
        for path, old, new in edits:
            f = root / path
            text = f.read_text()
            saved.setdefault(path, text)
            assert text.count(old) >= 1, (name, old)
            f.write_text(text.replace(old, new, 1))
        if isinstance(want, list):
            # A current source's floor is already the current one; only raw60 can show this.
            cmd = ['cargo', 'test', '-p', 'farcooler-daemon', '--test', 'a_cutover_survives_every_crash', 'a_raw_schema60' if name == 'floor-after-the-commit' else 'a_current_runner']
            extra = {'CUTOVER_ONLY': ','.join(want)}
        elif isinstance(want, tuple):
            cmd = ['cargo', 'test', '-p', 'farcooler-store', '--lib', want[1]]; extra = {}
        else:
            cmd = ['cargo', 'test', '-p', 'farcooler-daemon', '--test', 'a_cutover_survives_every_crash', want]; extra = {}
        r = subprocess.run(cmd, cwd=root, env={**env, **extra}, capture_output=True, text=True, timeout=1500)
        log = r.stdout + r.stderr
        built = 'running ' in log and 'could not compile' not in log
        if isinstance(want, list):
            red = sorted(set(re.findall(r'BOUNDARY-RED (\S+)', log)))
            ok = built and r.returncode != 0 and set(want) <= set(red)
        else:
            red = re.findall(r'^test (\S+) \.\.\. FAILED', log, re.M)
            ok = built and r.returncode != 0 and bool(red)
        results[name] = {'want': want, 'red': red, 'built': built, 'exit': r.returncode, 'pass': ok}
        print(('RED  ' if ok else 'MISS ') + name, 'built' if built else 'NOT-BUILT', red, flush=True)
        if not ok: (root / f'reports/ov-963/mutant-{name}.log').write_text(log[-6000:])
    finally:
        for path, text in saved.items(): (root / path).write_text(text)
    out.write_text(json.dumps(results, indent=1))
print('all red' if all(v['pass'] for v in results.values()) and len(results) == len(MUTANTS) else 'NOT ALL RED', len(results), '/', len(MUTANTS))

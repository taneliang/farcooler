//! Commit on a real file: the closed backup, what must stay frozen, and the
//! descriptor a process from before the owner lock holds.
use super::*;
use super::startup_gate::{bytes, source65};
use startup::{artifact_dir, open_recovery};
use transfer::begin;

fn frozen(dir: &std::path::Path, attempt: &str) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    drop(Store::open(&path).unwrap());
    drop(begin(&path, join_db(&path, attempt)).unwrap());
    path
}
fn phase(path: &std::path::Path) -> String { startup::read_authority(path).unwrap().unwrap().phase }
fn ledger(path: &std::path::Path) -> i64 {
    Connection::open(path).unwrap().query_row("SELECT count(*) FROM engine_cutover_ledger", [], |r| r.get(0)).unwrap()
}
/// A process that has the file open and takes no owner lock, as a build from
/// before the lock does: a real child, so the kernel has a real holder to name.
fn holder(path: &std::path::Path) -> std::process::Child {
    let child = std::process::Command::new("/bin/sh").arg("-c").arg("exec 3<>\"$0\"; echo open; exec sleep 60").arg(path)
        .stdout(std::process::Stdio::piped()).spawn().unwrap();
    child
}
fn opened(child: &mut std::process::Child) {
    use std::io::Read;
    let mut byte = [0];
    child.stdout.as_mut().unwrap().read_exact(&mut byte).unwrap();
}

#[test]
fn a_journal_with_no_header_is_inert() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    let journal = tmp.path().join("db.sqlite-journal");
    std::fs::write(&journal, vec![0u8; 4096]).unwrap();
    transfer::clear_inert_journal(&path).unwrap();
    assert!(!journal.exists(), "a process killed before any page moved leaves this");
}
#[test]
fn commit_takes_a_closed_backup_of_the_frozen_file_then_maps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    let before = bytes(&path);
    let done = closing::commit(&path, join_db(&path, "a")).unwrap();
    let backup = done.backup.unwrap();
    assert_eq!(bytes(&backup.join("db")), before, "the backup is the frozen file, not the baseline");
    assert_ne!(bytes(&backup.join("db")), bytes(&artifact_dir(&path).join("baseline.db")));
    assert!(!backup.join("restore-check").exists());
    assert_eq!(phase(&path), "committed");
    let snapshot: closing::Snapshot = serde_json::from_slice(&bytes(&backup.join("SNAPSHOT.json"))).unwrap();
    assert_eq!(snapshot.db_hash, farcooler_workflow::digest(&before));
    let journal = artifact_dir(&path).join("journal");
    assert!(journal.join("backed_up-closed-1.json").is_file() && journal.join("committed.json").is_file());
    let owners = join_db(&path, "a");
    let store = open_recovery(&path, &owners).unwrap();
    let (baseline, plan) = transfer::bound_plan(&path, &store).unwrap();
    assert_eq!(closing::held(&path, &store, &baseline).unwrap(), backup);
    store.engine_verify(&plan).unwrap();
}
#[test]
fn commit_a_second_time_resumes_and_takes_no_second_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    closing::commit(&path, join_db(&path, "a")).unwrap();
    let after = bytes(&path);
    fs_remove(&artifact_dir(&path).join("journal/committed.json"));
    let again = closing::commit(&path, join_db(&path, "a")).unwrap();
    assert!(again.resumed && again.backup.is_none());
    assert_eq!(bytes(&path), after, "database truth is not touched");
    assert!(artifact_dir(&path).join("journal/committed.json").is_file(), "only the journal catches up");
    assert!(!artifact_dir(&path).join("closed-2").exists());
}
fn fs_remove(path: &std::path::Path) { std::fs::remove_file(path).unwrap(); }
#[test]
fn commit_refuses_another_attempts_proof_and_a_database_that_is_not_frozen() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    assert!(closing::commit(&path, join_db(&path, "b")).is_err(), "another attempt's proof");
    assert!(!artifact_dir(&path).join("closed-1").exists());
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    transfer::hold(&path, "a").unwrap();
    assert!(closing::commit(&path, join_db(&path, "a")).is_err(), "a hold has no baseline to commit");
}
#[test]
fn commit_a_schema_release_would_refuse_stays_frozen_where_recover_still_works() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    let original = {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE owner_added(x);").unwrap();
        drop(conn);
        bytes(&path)
    };
    drop(begin(&path, join_db(&path, "a")).unwrap());
    let refused = closing::commit(&path, join_db(&path, "a")).unwrap_err().to_string();
    assert!(refused.contains("recover instead"), "{refused}");
    assert_eq!(phase(&path), "quiesced");
    assert!(transfer::recover(&path, join_db(&path, "recover")).unwrap().restored);
    assert_eq!(bytes(&path), original);
}
#[test]
fn commit_that_could_not_verify_rolls_back_whole_and_recover_still_works() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    BEFORE_SEAL.with(|h| *h.borrow_mut() = Some(Box::new(|conn: &Connection| {
        conn.execute("INSERT INTO meta VALUES('stray','a row no source had')", []).unwrap();
    })));
    assert!(closing::commit(&path, join_db(&path, "a")).is_err());
    assert_eq!(phase(&path), "quiesced");
    assert_eq!(ledger(&path), 0, "no part of the mapping survives");
    assert!(transfer::recover(&path, join_db(&path, "recover")).unwrap().restored);
}
#[test]
fn commit_after_a_failed_one_takes_a_new_backup_and_keeps_the_old() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    BEFORE_SEAL.with(|h| *h.borrow_mut() = Some(Box::new(|conn: &Connection| {
        conn.execute("INSERT INTO meta VALUES('stray','a row no source had')", []).unwrap();
    })));
    assert!(closing::commit(&path, join_db(&path, "a")).is_err());
    let done = closing::commit(&path, join_db(&path, "a")).unwrap();
    assert!(done.backup.unwrap().ends_with("closed-2"));
    assert!(artifact_dir(&path).join("closed-1/db").is_file());
}
#[test]
fn verify_and_release_need_the_backup_the_commit_named() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    let backup = closing::commit(&path, join_db(&path, "a")).unwrap().backup.unwrap();
    let owners = join_db(&path, "a");
    let store = open_recovery(&path, &owners).unwrap();
    let (baseline, _) = transfer::bound_plan(&path, &store).unwrap();
    let whole = bytes(&backup.join("db"));
    std::fs::write(backup.join("db"), &whole[..whole.len() / 2]).unwrap();
    assert!(closing::held(&path, &store, &baseline).is_err(), "truncated");
    std::fs::remove_dir_all(&backup).unwrap();
    assert!(closing::held(&path, &store, &baseline).is_err(), "lost");
}
#[test]
fn commit_from_raw_schema60_reaches_a_verified_release() {
    let tmp = tempfile::tempdir().unwrap();
    let path = super::source60::source60(tmp.path());
    drop(begin(&path, join_db(&path, "a")).unwrap());
    closing::commit(&path, join_db(&path, "a")).unwrap();
    let owners = join_db(&path, "a");
    let store = open_recovery(&path, &owners).unwrap();
    let (_, plan) = transfer::bound_plan(&path, &store).unwrap();
    store.engine_verify(&plan).unwrap();
    store.engine_release(&plan, owners).unwrap();
    assert_eq!(phase(&path), "released");
}
#[test]
fn old_process_is_named_and_blocks_every_stage_until_it_has_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let path = source65(tmp.path());
    let mut child = holder(&path);
    opened(&mut child);
    assert_eq!(owners::other_holders(&path).unwrap(), vec![child.id() as i32]);
    let refused = JoinedOwners::acquire(&path, "a").unwrap_err().to_string();
    assert!(refused.contains(&child.id().to_string()), "{refused}");
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(owners::other_holders(&path).unwrap().is_empty(), "the kernel closed it");
    drop(JoinedOwners::acquire(&path, "a").unwrap());
}
#[test]
fn old_process_with_the_frozen_file_open_stops_recover_before_a_byte_moves() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen(tmp.path(), "a");
    let frozen_bytes = bytes(&path);
    let mut child = holder(&path);
    opened(&mut child);
    assert!(JoinedOwners::acquire(&path, "recover").is_err(), "a Recover now would orphan its inode");
    assert_eq!(bytes(&path), frozen_bytes);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(transfer::recover(&path, join_db(&path, "recover")).unwrap().restored);
}

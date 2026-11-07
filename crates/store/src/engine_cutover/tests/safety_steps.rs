//! Each safety step of the closed backup refuses on damage a failing disk could
//! do, and a mutant that removes the step goes red (ov-963 M4). The damage is
//! made through `closing::TAMPER` at the point the step guards.
use super::*;
use super::startup_gate::bytes;
use startup::artifact_dir;
use transfer::begin;

/// A frozen file that holds a task, so a table and its indexes carry rows.
fn frozen_with_a_task(dir: &std::path::Path, attempt: &str) -> std::path::PathBuf {
    let path = dir.join("db.sqlite");
    {
        let store = Store::open(&path).unwrap();
        let repo = store.register_repository_for_test("Safety");
        let workspace = store.ensure_main_workspace(repo).unwrap();
        store.create_task(workspace.id, "kept", Actor::User).unwrap();
    }
    drop(begin(&path, join_db(&path, attempt)).unwrap());
    path
}
/// Change the file-change counter: the hash moves and SQLite reads on unharmed.
fn flip_header_counter(file: &std::path::Path) {
    let mut raw = std::fs::read(file).unwrap();
    raw[27] ^= 0xff;
    std::fs::write(file, raw).unwrap();
}
fn tamper_at(point: &'static str, hook: impl FnOnce(&std::path::Path) + 'static) {
    closing::TAMPER.with(|h| h.borrow_mut().insert(point, Box::new(hook)));
}
fn untouched_after_refusal(path: &std::path::Path) {
    assert_eq!(startup::read_authority(path).unwrap().unwrap().phase, "quiesced", "nothing committed");
}

#[test]
fn a_backup_that_does_not_restore_to_what_was_copied_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen_with_a_task(tmp.path(), "a");
    tamper_at("snapshot.after", flip_header_counter);
    let error = closing::commit(&path, join_db(&path, "a")).expect_err("a backup the disk changed").to_string();
    assert!(error.contains("independent restore differs"), "{error}");
    untouched_after_refusal(&path);
}
#[test]
fn a_live_file_that_changed_while_it_was_copied_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen_with_a_task(tmp.path(), "a");
    tamper_at("restore.after", flip_header_counter);
    let error = closing::commit(&path, join_db(&path, "a")).expect_err("a file that moved under the copy").to_string();
    assert!(error.contains("changed during the backup"), "{error}");
    untouched_after_refusal(&path);
}
#[test]
fn a_file_that_fails_its_integrity_check_is_never_backed_up_or_committed() {
    let tmp = tempfile::tempdir().unwrap();
    let path = frozen_with_a_task(tmp.path(), "a");
    tamper_at("close.before", |file| {
        // Point an index entry outside its page: every table still reads, only the integrity check sees it.
        let conn = Connection::open(file).unwrap();
        let page: i64 = conn.query_row("SELECT rootpage FROM sqlite_master WHERE type='index' AND tbl_name='tasks' AND rootpage>1 ORDER BY rootpage LIMIT 1", [], |r| r.get(0)).unwrap();
        let size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        drop(conn);
        let mut raw = std::fs::read(file).unwrap();
        let start = ((page - 1) * size) as usize;
        assert_eq!(raw[start], 0x0a, "a leaf index page");
        let cells = u16::from_be_bytes([raw[start + 3], raw[start + 4]]);
        assert!(cells >= 1, "the index holds an entry");
        raw[start + 8] = 0xff;
        raw[start + 9] = 0xff;
        std::fs::write(file, raw).unwrap();
        let conn = Connection::open(file).unwrap();
        let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap();
        assert_ne!(check, "ok", "the damage is real, or this test proves nothing");
        conn.query_row("SELECT count(*) FROM tasks", [], |r| r.get::<_, i64>(0)).expect("the table still reads");
    });
    let error = closing::commit(&path, join_db(&path, "a")).expect_err("a damaged file").to_string();
    assert!(error.contains("integrity check failed"), "{error}");
    assert!(!artifact_dir(&path).join("closed-1/db").exists(), "no copy of a damaged file is kept as a backup");
    untouched_after_refusal(&path);
    assert_ne!(bytes(&path), bytes(&artifact_dir(&path).join("baseline.db")));
}

#[test]
fn an_inert_journal_is_moved_aside_whole_not_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    drop(Store::open(&path).unwrap());
    let journal = tmp.path().join("db.sqlite-journal");
    let mut torn = vec![0u8; 512];
    torn[100..108].copy_from_slice(b"evidence");
    std::fs::write(&journal, &torn).unwrap();
    transfer::clear_inert_journal(&path).unwrap();
    assert!(!journal.exists(), "SQLite sees no journal beside the file");
    let kept: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("db.sqlite-journal.inert-")).collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(bytes(&kept[0]), torn);
    // A second one does not overwrite the first.
    std::fs::write(&journal, &torn[..256]).unwrap();
    transfer::clear_inert_journal(&path).unwrap();
    let count = std::fs::read_dir(tmp.path()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("db.sqlite-journal.inert-")).count();
    assert_eq!(count, 2);
}

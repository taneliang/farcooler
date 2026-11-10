//! ov-1024: a database created fresh is born released at incarnation 0, so an
//! answer, an ended hold and a message reach the orchestrator through the
//! engine's binding from the first day. A file from before the cutover keeps
//! `legacy` until the cutover releases it.
use super::*;
use crate::models::{Actor, NoteKind};

fn bound(store: &Store, note: uuid::Uuid) -> bool {
    store.conn().query_row("SELECT EXISTS(SELECT 1 FROM engine_manual_delivery WHERE note_id=?1 AND incarnation=0)",
        [note.as_bytes().as_slice()], |r| r.get(0)).unwrap()
}

#[test]
fn a_new_install_is_released_and_delivers_through_the_bound_path() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path().join("db.sqlite")).unwrap();
    let a = authority(&store.conn()).unwrap();
    assert_eq!((a.phase.as_str(), a.incarnation, a.manifest), ("released", 0, None), "born released");
    assert!(!store.legacy_authority(), "the legacy loops stay off");
    let repo = store.register_repository_for_test("Fresh");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let asks = store.create_task(ws.id, "asks", Actor::User).unwrap();
    let held = store.create_task(ws.id, "held", Actor::User).unwrap();

    let (answer, queued) = store.add_note_waking(asks.id, NoteKind::Answer, Actor::User, "yes, ship it", serde_json::json!({}), None, false).unwrap();
    assert!(queued, "the answer is queued");
    assert!(bound(&store, answer.id), "through the engine's binding");
    let pending = store.pending_answer_wakes().unwrap();
    assert_eq!(pending.iter().map(|p| p.note).collect::<Vec<_>>(), vec![answer.id]);
    assert!(store.claim_wake(&pending[0]).unwrap() && store.finish_wake(&pending[0], None).unwrap().is_some(), "and told");

    store.conn().execute("UPDATE tasks SET wait_kind='until', wait_until=?2 WHERE id=?1",
        rusqlite::params![held.id.as_bytes().as_slice(), crate::tasks::now_millis() - 1]).unwrap();
    let ended = store.release_due_holds(crate::tasks::now_millis()).unwrap();
    assert_eq!(ended.len(), 1);
    assert!(bound(&store, ended[0].1.id), "the ended hold is bound");
    let pending = store.pending_hold_wakes().unwrap();
    assert_eq!(pending.iter().map(|p| p.note).collect::<Vec<_>>(), vec![ended[0].1.id]);
    assert!(store.claim_wake(&pending[0]).unwrap() && store.finish_wake(&pending[0], None).unwrap().is_some(), "and told");

    let message = store.send_manual_message(asks.id, Actor::User, "look at this", None, serde_json::json!({})).unwrap();
    assert!(bound(&store, message.id), "the message is bound");
    let pending = store.pending_message_wakes().unwrap();
    assert_eq!(pending.iter().map(|p| p.note).collect::<Vec<_>>(), vec![message.id]);

    assert!(store.conn().execute("INSERT INTO answer_wakes(note_id,task_id,enqueued_at) VALUES(x'0102',?1,1)", [asks.id.as_bytes().as_slice()]).is_err(),
        "an unbound wake is fenced from the start");
}

#[test]
fn a_file_from_before_stays_legacy_until_it_is_cut_over() {
    let current = crate::migrate::CURRENT_SCHEMA_VERSION;
    let legacy_file = |version: u32| {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
        crate::migrate::migrate_only_to(&mut conn, version);
        crate::migrate::stamp_compatible_down_to(&conn).unwrap();
        (tmp, path)
    };
    // Written at the current schema by a build that still created legacy files.
    let (_tmp, path) = legacy_file(current);
    let store = Store::open(&path).unwrap();
    assert_eq!(authority(&store.conn()).unwrap().phase, "legacy", "an existing legacy file stays legacy");
    assert!(store.legacy_authority());
    drop(store);
    assert_eq!(authority(&Store::open(&path).unwrap().conn()).unwrap().phase, "legacy", "and again on the next open");
    // One registry behind: opening never migrates it forward into release.
    // It waits, still legacy, for the explicit cutover.
    let (_tmp, path) = legacy_file(current - 1);
    assert!(Store::open(&path).is_err(), "refused until it is cut over");
    assert_eq!(startup::read_authority(&path).unwrap().unwrap().phase, "legacy");
}

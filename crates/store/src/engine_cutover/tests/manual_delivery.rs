use super::*;
use crate::models::Actor;

fn released_file() -> (tempfile::TempDir, std::path::PathBuf, uuid::Uuid, uuid::Uuid) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("Manual");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let task = store.create_task(ws.id, "retained", Actor::User).unwrap();
    // A legacy queued message and a legacy pending notice: both pre-cutover history.
    store.send_message(task.id, Actor::User, "queued before cutover", None, serde_json::json!({})).unwrap();
    let c = store.conn();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    c.execute("INSERT INTO workflow_notice_outbox(id,workspace,revision,payload,payload_hash) VALUES('old-notice','w',1,'{}','h')", []).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    drop(c);
    let plan = preflight(&store.conn()).unwrap();
    store.engine_quiesce(&joined(&store)).unwrap();
    store.engine_commit_for_test(&plan, &proof(&store), "manual").unwrap();
    store.engine_release(&plan, &joined(&store)).unwrap();
    drop(store);
    (tmp, path, ws.id, task.id)
}

#[test]
fn manual_delivery_is_fresh_bound_and_never_revives_old_queue_rows() {
    let (_tmp, path, _ws, task) = released_file();
    let store = Store::open(&path).unwrap();
    assert!(store.pending_message_wakes().unwrap().is_empty(), "the pre-cutover queued row is inert history");
    assert_eq!(store.conn().query_row("SELECT count(*) FROM message_wakes", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "and is retained");
    assert!(store.conn().execute("UPDATE message_wakes SET claimed_at=1", []).is_err(), "old row cannot be claimed");
    assert!(store.send_message(task, Actor::User, "x", None, serde_json::json!({})).is_err(), "ordinary queue stays fenced");
    assert!(store.send_runner_notice(task, "x", serde_json::json!({})).is_err(), "automatic producers stay fenced");
    assert!(store.send_manual_message(task, Actor::Runner, "x", None, serde_json::json!({})).is_err(), "never the runner");
    let note = store.send_manual_message(task, Actor::User, "fresh manual", None, serde_json::json!({})).unwrap();
    let pending = store.pending_message_wakes().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].note, note.id);
    assert!(store.conn().execute("UPDATE message_wakes SET claimed_at=5 WHERE note_id=?1", [note.id.as_bytes().as_slice()]).is_ok(), "the bound row is claimable");
    assert!(store.pending_answer_wakes().unwrap().is_empty() && store.pending_hold_wakes().unwrap().is_empty());
}
#[test]
fn manual_binding_cannot_be_forged_for_a_stale_incarnation_or_before_release() {
    let (_tmp, path, _ws, task) = released_file();
    let store = Store::open(&path).unwrap();
    store.send_manual_message(task, Actor::User, "bound", None, serde_json::json!({})).unwrap();
    let c = store.conn();
    assert!(c.execute("INSERT INTO engine_manual_delivery VALUES(x'01',x'02',99)", []).is_err(), "wrong incarnation");
    assert!(c.execute("UPDATE engine_manual_delivery SET incarnation=2", []).is_err(), "bindings are immutable");
    drop(c);
    let fresh = Store::open_in_memory().unwrap();
    let repo = fresh.register_repository_for_test("Legacy");
    let ws = fresh.ensure_main_workspace(repo).unwrap();
    let t = fresh.create_task(ws.id, "legacy", Actor::User).unwrap();
    assert!(fresh.conn().execute("INSERT INTO engine_manual_delivery VALUES(x'01',?1,0)", [t.id.as_bytes().as_slice()]).is_err(), "no binding before release");
}
#[test]
fn notice_eligibility_floor_keeps_retained_pending_history_out_of_the_consumer() {
    let (_tmp, path, _ws, _task) = released_file();
    let store = Store::open(&path).unwrap();
    assert!(store.workflow_pending_notices(0).unwrap().is_empty(), "retained pending notice is not rediscovered");
    assert_eq!(store.conn().query_row("SELECT state FROM workflow_notice_outbox WHERE id='old-notice'", [], |r| r.get::<_, String>(0)).unwrap(), "pending", "and stays intact");
    assert!(!crate::engine_manual::notice_eligible(&store.conn(), "old-notice").unwrap());
    let c = store.conn();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    c.execute("INSERT INTO workflow_notice_outbox(id,workspace,revision,payload,payload_hash) VALUES('new-notice','w',2,'{}','h2')", []).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    drop(c);
    assert!(crate::engine_manual::notice_eligible(&store.conn(), "new-notice").unwrap());
}

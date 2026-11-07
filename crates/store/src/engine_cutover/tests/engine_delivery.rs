//! ov-949: after release the engine's binding (`engine_manual_delivery`) is how an
//! answer, an ended hold and a runner notice reach the orchestrator. Each is told
//! exactly once; every row queued before cutover stays inert history. Private
//! fixtures only.
use super::*;
use crate::models::{Actor, NoteKind};

struct World { _tmp: tempfile::TempDir, path: std::path::PathBuf, task: uuid::Uuid, held: uuid::Uuid }

/// A board with an answer wake, a hold wake and a notice still untold at cutover.
fn released_file() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("Delivery");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let task = store.create_task(ws.id, "asks", Actor::User).unwrap();
    let held = store.create_task(ws.id, "held", Actor::User).unwrap();
    store.add_note_waking(task.id, NoteKind::Answer, Actor::User, "before cutover", serde_json::json!({}), None, false).unwrap();
    let old = store.create_task(ws.id, "held before", Actor::User).unwrap();
    hold_due(&store, old.id);
    assert_eq!(store.release_due_holds(crate::tasks::now_millis()).unwrap().len(), 1);
    assert_eq!(store.pending_hold_wakes().unwrap().len(), 1, "a hold wake untold at cutover");
    store.send_runner_notice(task.id, "legacy notice", serde_json::json!({})).unwrap();
    let plan = preflight(&store.conn()).unwrap();
    store.engine_quiesce(joined(&store)).unwrap();
    store.engine_commit_for_test(&plan, &proof(&store), "delivery").unwrap();
    store.engine_release(&plan, joined(&store)).unwrap();
    drop(store);
    World { _tmp: tmp, path, task: task.id, held: held.id }
}
fn hold_due(store: &Store, task: uuid::Uuid) {
    store.conn().execute("UPDATE tasks SET wait_kind='until', wait_until=?2 WHERE id=?1",
        rusqlite::params![task.as_bytes().as_slice(), crate::tasks::now_millis() - 1]).unwrap();
}

#[test]
fn an_answer_after_release_is_told_exactly_once_and_old_answers_stay_history() {
    let w = released_file();
    let store = Store::open(&w.path).unwrap();
    assert!(store.pending_answer_wakes().unwrap().is_empty(), "the pre-cutover answer is inert history");
    assert!(store.conn().execute("INSERT INTO answer_wakes(note_id,task_id,enqueued_at) VALUES(x'0102',?1,1)", [w.task.as_bytes().as_slice()]).is_err(), "an unbound row is still fenced");
    let (note, queued) = store.add_note_waking(w.task, NoteKind::Answer, Actor::User, "yes, ship it", serde_json::json!({}), None, false).unwrap();
    assert!(queued, "the answer is queued through the engine");
    let pending = store.pending_answer_wakes().unwrap();
    assert_eq!(pending.len(), 1, "only the new one");
    assert_eq!(pending[0].note, note.id);
    assert!(store.claim_wake(&pending[0]).unwrap());
    assert!(!store.claim_wake(&pending[0]).unwrap(), "typed at most once");
    assert!(store.finish_wake(&pending[0], None).unwrap().is_some());
    assert!(store.finish_wake(&pending[0], None).unwrap().is_none(), "finished once");
    assert!(store.pending_answer_wakes().unwrap().is_empty());
    assert!(!store.any_pending_answer_wake().unwrap(), "and the cheap probe agrees; the old row is not pending");
    let rows: i64 = store.conn().query_row("SELECT count(*) FROM answer_wakes", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 2, "history kept");
}

#[test]
fn a_hold_that_ends_after_release_wakes_the_orchestrator_once() {
    let w = released_file();
    let store = Store::open(&w.path).unwrap();
    hold_due(&store, w.held);
    assert!(store.pending_hold_wakes().unwrap().is_empty(), "the hold wake from before cutover is inert history");
    let released = store.release_due_holds(crate::tasks::now_millis()).unwrap();
    assert_eq!(released.len(), 1);
    let pending = store.pending_hold_wakes().unwrap();
    assert_eq!(pending.len(), 1, "the orchestrator is told");
    assert_eq!(pending[0].note, released[0].1.id);
    assert!(store.release_due_holds(crate::tasks::now_millis()).unwrap().is_empty(), "a hold lets go once");
    assert_eq!(store.pending_hold_wakes().unwrap().len(), 1, "so there is still one wake");
    assert!(store.claim_wake(&pending[0]).unwrap() && !store.claim_wake(&pending[0]).unwrap());
    assert!(store.finish_wake(&pending[0], Some("Told the orchestrator.")).unwrap().is_some());
    assert!(store.pending_hold_wakes().unwrap().is_empty());
}

#[test]
fn a_runner_notice_after_release_is_told_once_and_folds_into_the_one_waiting() {
    let w = released_file();
    let store = Store::open(&w.path).unwrap();
    assert!(store.pending_message_wakes().unwrap().is_empty(), "the legacy notice is inert history");
    let first = store.send_runner_notice(w.task, "Agent 2 ended.", serde_json::json!({})).unwrap();
    assert_eq!(store.pending_message_wakes().unwrap().len(), 1);
    let second = store.send_runner_notice(w.task, "Agent 3 never started.", serde_json::json!({})).unwrap();
    let pending = store.pending_message_wakes().unwrap();
    assert_eq!(pending.len(), 1, "never dropped behind another of its own: folded into one");
    assert_eq!(pending[0].note, second.id);
    assert!(pending[0].body.contains("Agent 2 ended."), "and it says both: {}", pending[0].body);
    assert_ne!(first.id, second.id);
    assert!(store.claim_wake(&pending[0]).unwrap() && !store.claim_wake(&pending[0]).unwrap());
    assert!(store.send_runner_notice(w.task, "Agent 4 ended.", serde_json::json!({})).is_ok(), "one being typed is left to finish");
    assert_eq!(store.pending_message_wakes().unwrap().len(), 2);
    assert!(store.send_manual_message(w.task, Actor::Runner, "x", None, serde_json::json!({})).is_err(), "a manual message is still never the runner's");
}

#[test]
fn nothing_is_queued_while_the_engine_is_not_released() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path().join("db.sqlite")).unwrap();
    let repo = store.register_repository_for_test("Window");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let task = store.create_task(ws.id, "t", Actor::User).unwrap();
    store.engine_quiesce(joined(&store)).unwrap();
    let (_, queued) = store.add_note_waking(task.id, NoteKind::Answer, Actor::User, "mid-cutover", serde_json::json!({}), None, false).unwrap();
    assert!(!queued, "the answer lands as a note, and no wake is invented");
    assert!(store.send_runner_notice(task.id, "x", serde_json::json!({})).is_err());
}

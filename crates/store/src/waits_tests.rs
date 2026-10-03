use super::*;

use crate::models::TaskStatus::{Backlog, Cancelled, Done, InProgress, InReview, NeedsDecision, Todo};

const STATUSES: [TaskStatus; 7] = [Backlog, Todo, NeedsDecision, InProgress, InReview, Done, Cancelled];

/// A store with one board, and `n` tasks on it, in the backlog, in order.
fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn wait_of(store: &Store, task: &Task) -> Option<Wait> {
    store.get_task(task.id).unwrap().wait.map(|w| w.wait)
}

fn facts(store: &Store, task: &Task) -> TaskFacts {
    let task = store.get_task(task.id).unwrap();
    store.task_facts(std::slice::from_ref(&task)).unwrap().remove(0)
}

fn raw_kind(store: &Store, task: &Task) -> Option<String> {
    store.conn().query_row("SELECT wait_kind FROM tasks WHERE id = ?1", [uuid_blob(task.id)], |r| r.get(0)).unwrap()
}

fn wait_notes(store: &Store, task: &Task) -> Vec<String> {
    store.notes_for(task.id, Some(NoteKind::Wait)).unwrap().into_iter().map(|n| n.body).collect()
}

/// (1) Dispatch clears a place in the agent line: the line is for tasks not
/// started, and a started task left in it would hold a place forever. The
/// move's note says it cleared one.
#[test]
fn a_task_in_the_agent_line_leaves_it_when_dispatched() {
    let (store, main, t) = board(2);
    store.set_line(main, TaskLine::Agent, &[t[0].id, t[1].id], Actor::Manager).unwrap();

    let moved = store.set_task_status(t[0].id, InProgress, Actor::Manager).unwrap();
    assert_eq!(moved.wait, None, "the row handed back");
    assert_eq!(raw_kind(&store, &t[0]), None, "the column, not only the read's status gate");
    let note = store.notes_for(t[0].id, Some(NoteKind::StatusChange)).unwrap().pop().unwrap();
    assert_eq!(note.extra["wait_cleared"], "in_line");
    assert_eq!(facts(&store, &t[1]).position, 1, "the one behind it is next now");
}

/// A build-line place survives a move the line still takes, in progress to
/// in review, and a reorder writes no note.
#[test]
fn a_build_line_place_survives_review_and_a_reorder_writes_nothing() {
    let (store, main, t) = board(2);
    for task in &t {
        store.set_task_status(task.id, InProgress, Actor::Manager).unwrap();
    }
    store.set_line(main, TaskLine::Build, &[t[0].id, t[1].id], Actor::Manager).unwrap();
    store.set_task_status(t[0].id, InReview, Actor::Manager).unwrap();
    assert_eq!(wait_of(&store, &t[0]), Some(Wait::InLine(TaskLine::Build)));

    store.set_line(main, TaskLine::Build, &[t[1].id, t[0].id], Actor::Manager).unwrap();
    assert_eq!(wait_notes(&store, &t[0]), ["In line to build."], "one note, for joining");
    assert_eq!((facts(&store, &t[1]).position, facts(&store, &t[0]).position), (1, 2));
}

/// (2) A line is replaced whole: a task not named loses its place, rather
/// than keeping a stale one behind the named.
#[test]
fn a_line_is_replaced_whole() {
    let (store, main, t) = board(3);
    let (a, b, c) = (&t[0], &t[1], &t[2]);
    store.set_line(main, TaskLine::Agent, &[a.id, b.id, c.id], Actor::Manager).unwrap();
    let line = store.set_line(main, TaskLine::Agent, &[c.id, a.id], Actor::Manager).unwrap();

    assert_eq!(line.iter().map(|t| t.id).collect::<Vec<_>>(), [c.id, a.id], "answered in line order");
    assert_eq!((facts(&store, c).position, facts(&store, a).position), (1, 2));
    assert_eq!(facts(&store, b).position, 0);
    assert_eq!(wait_of(&store, b), None, "b is out of the line, not ranked behind it");
    assert_eq!(wait_notes(&store, b), ["In line to start.", "Out of the line to start."]);
    assert_eq!(facts(&store, a).ahead, [c.key.clone()]);
}

/// Who is ahead: the front of the line, at most three, in line order.
#[test]
fn ahead_names_the_front_of_the_line() {
    let (store, main, t) = board(5);
    let ids: Vec<Uuid> = t.iter().map(|t| t.id).collect();
    store.set_line(main, TaskLine::Agent, &ids, Actor::Manager).unwrap();
    let keys: Vec<String> = t.iter().map(|t| t.key.clone()).collect();
    assert_eq!(facts(&store, &t[0]).ahead, Vec::<String>::new());
    assert_eq!(facts(&store, &t[2]).ahead, keys[..2]);
    assert_eq!(facts(&store, &t[4]).ahead, keys[..3]);
    assert_eq!(facts(&store, &t[4]).position, 5);
}

/// (3) A task ranked first whose status no longer fits the line, left by an
/// older build that moved it, doesn't take position 1 from the task behind.
#[test]
fn a_done_task_ranked_first_takes_no_place() {
    let (store, main, t) = board(2);
    store.set_line(main, TaskLine::Agent, &[t[0].id, t[1].id], Actor::Manager).unwrap();
    // An older build moves it: no clear.
    store.conn().execute("UPDATE tasks SET status = 'done' WHERE id = ?1", [uuid_blob(t[0].id)]).unwrap();

    assert_eq!(wait_of(&store, &t[0]), None, "its own read hides it");
    let behind = facts(&store, &t[1]);
    assert_eq!((behind.position, behind.ahead.len()), (1, 0));
}

/// The line refuses what it can't hold, and writes nothing when it does.
#[test]
fn a_line_refuses_a_task_it_cannot_hold() {
    let (store, main, t) = board(2);
    store.set_task_status(t[1].id, InProgress, Actor::Manager).unwrap();
    let refused = |ids: &[Uuid], line| store.set_line(main, line, ids, Actor::Manager).unwrap_err();
    assert!(matches!(refused(&[t[0].id, t[1].id], TaskLine::Agent), DomainError::InvalidArgument { what: "line_status" }));
    assert!(matches!(refused(&[t[0].id], TaskLine::Build), DomainError::InvalidArgument { what: "line_status" }));
    assert!(matches!(refused(&[t[0].id, t[0].id], TaskLine::Agent), DomainError::InvalidArgument { what: "task_twice" }));

    let repo = store.get_task(t[0].id).unwrap().repository_id;
    let other = store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let elsewhere = store.create_task(other, "elsewhere", Actor::Manager).unwrap();
    assert!(matches!(refused(&[t[0].id, elsewhere.id], TaskLine::Agent), DomainError::InvalidArgument { what: "other_board" }));
    assert_eq!(wait_of(&store, &t[0]), None, "a refused line wrote nothing");
}

/// (4) `waiting_on` counts only blockers that haven't finished: ov-37 to
/// ov-42 read "Waiting on ov-36" for days after ov-36 was done. The edge
/// stays, as history.
#[test]
fn a_finished_blocker_stops_counting() {
    let (store, _, t) = board(3);
    let (phase_b, follower, other) = (&t[0], &t[1], &t[2]);
    store.set_block(follower.id, phase_b.id, Some("follows Phase B landing")).unwrap();
    store.set_block(follower.id, other.id, None).unwrap();
    assert_eq!(facts(&store, follower).waiting_on, [phase_b.key.clone(), other.key.clone()]);

    store.set_task_status(phase_b.id, Done, Actor::Manager).unwrap();
    store.set_task_status(other.id, Cancelled, Actor::Manager).unwrap();
    assert!(facts(&store, follower).waiting_on.is_empty());
    assert_eq!(store.blocks_for(follower.id).unwrap().len(), 2, "the edges stay");
    assert_eq!(store.dependents_of(phase_b.id).unwrap()[0].id, follower.id);
}

/// A held todo task moves to backlog, recorded: held isn't ready.
#[test]
fn holding_a_todo_task_moves_it_to_the_backlog() {
    let (store, _, t) = board(1);
    store.set_task_status(t[0].id, Todo, Actor::Manager).unwrap();
    let held = store.set_wait(t[0].id, Some(Wait::Parked), Actor::Manager).unwrap();
    assert_eq!((held.status, held.wait.map(|w| w.wait)), (Backlog, Some(Wait::Parked)));
    let moves = store.notes_for(t[0].id, Some(NoteKind::StatusChange)).unwrap();
    assert_eq!(moves.last().unwrap().extra["to"], "backlog");
    assert_eq!(wait_notes(&store, &t[0]), ["Parked: nobody plans to start it."]);
}

/// A started task can't be held, a time must be ahead, a line is set whole,
/// and clearing works in any status.
#[test]
fn set_wait_refuses_what_it_cannot_mean() {
    let (store, _, t) = board(1);
    let later = now_millis() + 3_600_000;
    assert!(matches!(
        store.set_wait(t[0].id, Some(Wait::Until(now_millis() - 1)), Actor::Manager),
        Err(DomainError::InvalidArgument { what: "until" })
    ));
    assert!(matches!(
        store.set_wait(t[0].id, Some(Wait::InLine(TaskLine::Agent)), Actor::Manager),
        Err(DomainError::InvalidArgument { what: "kind" })
    ));
    store.set_wait(t[0].id, Some(Wait::Until(later)), Actor::Manager).unwrap();
    assert_eq!(wait_of(&store, &t[0]), Some(Wait::Until(later)));
    store.set_task_status(t[0].id, InProgress, Actor::Manager).unwrap();
    assert!(matches!(
        store.set_wait(t[0].id, Some(Wait::Parked), Actor::Manager),
        Err(DomainError::InvalidArgument { what: "wait_status" })
    ));
    store.set_wait(t[0].id, None, Actor::Manager).unwrap();
}

/// Setting the same wait again writes no second note; changing it does.
#[test]
fn only_a_change_of_wait_writes_a_note() {
    let (store, _, t) = board(1);
    store.set_wait(t[0].id, Some(Wait::After(WaitEvent::Release)), Actor::Manager).unwrap();
    store.set_wait(t[0].id, Some(Wait::After(WaitEvent::Release)), Actor::Manager).unwrap();
    store.set_wait(t[0].id, None, Actor::Manager).unwrap();
    assert_eq!(wait_notes(&store, &t[0]), ["Held until the next release.", "No longer held."]);
    let refused = store.add_note(t[0].id, NoteKind::Wait, Actor::Manager, "forged", json!({}));
    assert!(matches!(refused, Err(DomainError::InvalidArgument { what: "kind" })), "only the store writes these");
}

/// (5, the store's half) A hold whose time has come is let go, with a note
/// from the runner and a `hold_ended` wake on that note, and one not yet due
/// is left alone. The answer pump doesn't see the wake.
#[test]
fn a_hold_whose_time_came_is_let_go() {
    let (store, _, t) = board(2);
    let now = now_millis();
    store.set_wait(t[0].id, Some(Wait::Until(now + 1_000)), Actor::Manager).unwrap();
    store.set_wait(t[1].id, Some(Wait::Until(now + 60_000)), Actor::Manager).unwrap();
    assert_eq!(store.next_hold_due().unwrap(), Some(now + 1_000));
    assert!(store.release_due_holds(now).unwrap().is_empty(), "not yet");

    let released = store.release_due_holds(now + 1_000).unwrap();
    assert_eq!(released.len(), 1);
    let (task, note) = &released[0];
    assert_eq!((task.id, task.wait), (t[0].id, None));
    assert_eq!((note.kind, note.actor), (NoteKind::Wait, Actor::Runner));
    assert!(note.body.ends_with("That time has come."), "{}", note.body);
    assert_eq!(store.next_hold_due().unwrap(), Some(now + 60_000));
    let kind: String = store
        .conn()
        .query_row("SELECT kind FROM answer_wakes WHERE note_id = ?1", [uuid_blob(note.id)], |r| r.get(0))
        .unwrap();
    assert_eq!(kind, "hold_ended");
    assert!(store.pending_answer_wakes().unwrap().is_empty() && !store.any_pending_answer_wake().unwrap());
}

/// (7) Opening the store clears a wait an older build left on a task it
/// moved, and closes a subagent it left open on a task it finished.
#[test]
fn opening_sweeps_what_an_older_build_left() {
    let dir = std::env::temp_dir().join(format!("farcooler-sweep-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let (task, kept) = {
        let store = Store::open(&path).unwrap();
        let repo = store.register_repository_for_test("overnight");
        let main = store.ensure_main_workspace(repo).unwrap().id;
        let task = store.create_task(main, "held", Actor::Manager).unwrap();
        let kept = store.create_task(main, "kept", Actor::Manager).unwrap();
        store.set_wait(task.id, Some(Wait::Parked), Actor::Manager).unwrap();
        store.set_wait(kept.id, Some(Wait::Parked), Actor::Manager).unwrap();
        let record = crate::workers::WorkerRecord {
            harness: "claude".into(),
            agent_id: "a1".into(),
            session_id: None,
            session_cwd: None,
            orchestrator_terminal: None,
            label: None,
            model: None,
            linked_by: crate::workers::LinkedBy::Orchestrator,
        };
        store.record_worker(task.id, &record, Actor::Manager).unwrap();
        // An older build moves it to done, knowing nothing of either.
        store.conn().execute("UPDATE tasks SET status = 'done' WHERE id = ?1", [uuid_blob(task.id)]).unwrap();
        (task, kept)
    };

    let store = Store::open(&path).unwrap();
    assert_eq!(raw_kind(&store, &task), None, "the column, not only the read's status gate");
    assert_eq!(raw_kind(&store, &kept).as_deref(), Some("parked"), "a wait that fits stays");
    let worker = store.workers_for(task.id).unwrap().remove(0);
    assert_eq!(worker.end_reason, Some(crate::workers::EndReason::TaskClosed));
    drop(store);
    std::fs::remove_dir_all(&dir).ok();
}

/// `Wait::fits` and `FITS_SQL` are one rule written twice; every kind in
/// every status, each way.
#[test]
fn the_two_copies_of_the_fit_rule_agree() {
    let (store, _, t) = board(1);
    let waits = [
        Wait::InLine(TaskLine::Agent),
        Wait::InLine(TaskLine::Build),
        Wait::Until(1),
        Wait::After(WaitEvent::Recurrence),
        Wait::Parked,
    ];
    for wait in waits {
        for status in STATUSES {
            let conn = store.conn();
            write_wait(&conn, t[0].id, Some(wait), Some(1), 1).unwrap();
            conn.execute("UPDATE tasks SET status = ?1 WHERE id = ?2", params![status.as_str(), uuid_blob(t[0].id)])
                .unwrap();
            let sql: bool = conn
                .query_row(&format!("SELECT {FITS_SQL} FROM tasks WHERE id = ?1"), [uuid_blob(t[0].id)], |r| r.get(0))
                .unwrap();
            assert_eq!(sql, wait.fits(status), "{wait:?} in {status:?}");
        }
    }
}

/// A task leaving its board leaves that board's line; a hold goes with it.
#[test]
fn moving_a_task_to_another_board_takes_it_out_of_line() {
    let (store, main, t) = board(2);
    store.set_line(main, TaskLine::Agent, &[t[0].id], Actor::Manager).unwrap();
    store.set_wait(t[1].id, Some(Wait::Parked), Actor::Manager).unwrap();
    let repo = t[0].repository_id;
    let other = store.create_workspace(repo, "Billing", "bil").unwrap().id;
    store.move_tasks(&[t[0].id, t[1].id], other, Actor::Manager).unwrap();
    assert_eq!(wait_of(&store, &t[0]), None);
    assert_eq!(wait_of(&store, &t[1]), Some(Wait::Parked));
}

/// (6) A copy of a real runner's database at schema 20 migrates to 21 with
/// every card, key and note intact, and then says no build before 21 may
/// open it (0021 is `Refused`; see its doc).
///
/// By hand, against a copy, never the live file:
/// `sqlite3 <live db> ".backup /tmp/copy.db"`, then
/// `FARCOOLER_DB_COPY=/tmp/copy.db cargo test -p farcooler-store -- --ignored a_copy_of_a_real_board`.
#[test]
#[ignore = "needs FARCOOLER_DB_COPY: a copy of a real database"]
fn a_copy_of_a_real_board_keeps_every_card_key_and_note() {
    let source = std::env::var("FARCOOLER_DB_COPY").expect("FARCOOLER_DB_COPY names a copy");
    let dir = std::env::temp_dir().join(format!("farcooler-copy-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    std::fs::copy(&source, &path).unwrap();
    let census = |conn: &Connection| -> (Vec<(Vec<u8>, String, String)>, Vec<(Vec<u8>, String, String)>) {
        let rows = |sql: &str| {
            let mut stmt = conn.prepare(sql).unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|r| r.unwrap()).collect()
        };
        (
            rows("SELECT id, key, status FROM tasks ORDER BY id"),
            rows("SELECT id, kind, body FROM task_notes ORDER BY id"),
        )
    };
    let before = {
        let conn = Connection::open(&path).unwrap();
        assert_eq!(crate::migrate::read_schema_version(&conn).unwrap(), 20, "a schema-20 copy");
        census(&conn)
    };
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(crate::migrate::read_schema_version(&conn).unwrap(), 21);
    let after = census(&conn);
    assert!(!before.0.is_empty(), "a real board has cards");
    assert_eq!(before, after, "every card, key, status and note");
    assert_eq!(crate::compat::read_compatible_down_to(&conn).unwrap(), Some(21), "no build before 21 opens it");
    eprintln!("{} cards and {} notes kept", after.0.len(), after.1.len());
    std::fs::remove_dir_all(&dir).ok();
}

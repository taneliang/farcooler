//! Private fixtures only: a scratch store with throwaway tasks.
use crate::{Store, models::{Actor, TaskStatus}, ticket_lines::{LineUpdate, NewTicketLine}};
use crate::waits::Wait;
use farcooler_workflow::*;
use std::collections::BTreeSet;

struct Board { store: Store, ws: uuid::Uuid, task: crate::models::Task, other: crate::models::Task }

fn board() -> Board {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("Projection");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(ws, "owned", Actor::User).unwrap();
    let other = store.create_task(ws, "hand moved", Actor::User).unwrap();
    Board { store, ws, task, other }
}
fn input(b: &Board, id: &str, revision: u64, grant: Grant, command: Command) -> Input {
    Input { schema: 1, id: id.into(), workspace: b.ws.to_string(), expected_revision: revision, epoch: 1,
        source: "controller".into(), source_sequence: revision + 1, now_ms: 1000 + revision as i64, grant,
        busy_resources: BTreeSet::new(), command }
}
fn enqueue(b: &Board, definition: Definition) -> u64 {
    let tracker = definition.source.starts_with("tracker") || definition.is_ticket();
    let candidate = tracker.then(|| definition.pin());
    let c = Command::Enqueue { item: b.task.key.clone(), policy_pin: "policy-1".into(), resource: "own".into(),
        dependencies: BTreeSet::new(), initial_candidate: candidate, definition };
    let receipt = b.store.workflow_apply(&input(b, "enqueue", 0, Grant::Controller, c)).unwrap();
    receipt.result.unwrap()
}
/// Settle the stage in flight as passed, the way the runtime would.
fn pass(b: &Board, n: usize) -> u64 {
    let state = b.store.workflow_state(&b.ws.to_string()).unwrap().unwrap();
    let item = &state.items[&b.task.key];
    let effect = item.pending.clone().expect("a stage is in flight");
    let claim = b.store.workflow_claim_effect(&state.workspace, &effect, "fixture-runtime", state.revision, state.epoch, 1000, 2000).unwrap().unwrap();
    assert!(b.store.workflow_admit_effect(&claim, 1001).unwrap());
    let e = Evidence { candidate: "candidate".into(), manifest: "manifest".into(), receipt: "receipt".into(), policy_pin: "policy-1".into() };
    let c = Command::Observe { item: b.task.key.clone(), effect: effect.clone(), run: item.run.clone(), fence: item.fence, outcome: Outcome::Passed(e) };
    let grant = Grant::Operation { effect, run: item.run.clone(), fence: item.fence };
    b.store.workflow_apply(&input(b, &format!("pass-{n}"), state.revision, grant, c)).unwrap().result.unwrap()
}
fn seen(b: &Board) -> (String, Option<String>) {
    let status = b.store.get_task(b.task.id).unwrap().status.as_str().to_string();
    let stage = b.store.conn().query_row("SELECT stage FROM ticket_lines WHERE task_id=?1", [b.task.id.as_bytes().as_slice()], |r| r.get(0)).ok();
    (status, stage)
}

#[test]
fn an_owned_card_follows_its_run_with_no_hand_move() {
    let b = board();
    assert_eq!(seen(&b), ("backlog".into(), None));
    enqueue(&b, Definition::coding());
    assert_eq!(seen(&b), ("in_progress".into(), Some("building".into())), "provision runs: someone is building it");
    pass(&b, 1); pass(&b, 2); pass(&b, 3); // provision, build, check
    assert_eq!(seen(&b), ("in_review".into(), Some("reviewing".into())), "the review stage is running");
    pass(&b, 4);
    assert_eq!(seen(&b), ("in_review".into(), Some("landing".into())), "land");
    pass(&b, 5); pass(&b, 6);
    assert_eq!(seen(&b), ("done".into(), Some("landed".into())), "every stage passed");
    // The card says how it got there, and who moved it.
    let notes = b.store.notes_for(b.task.id, Some(crate::models::NoteKind::StatusChange)).unwrap();
    assert!(notes.iter().all(|n| n.actor == Actor::Runner), "the runner moved it, not a person");
    assert_eq!(notes.len(), 3, "backlog to in_progress to in_review to done");
}

#[test]
fn a_run_waiting_on_a_person_asks_for_one_and_cancelling_it_drops_the_card() {
    let b = board();
    enqueue(&b, Definition::tracker("owner"));
    assert_eq!(seen(&b), ("needs_decision".into(), Some("needs_you".into())));
    let r = b.store.workflow_apply(&input(&b, "cancel", 1, Grant::Controller, Command::Cancel { item: b.task.key.clone() })).unwrap();
    assert!(r.result.is_ok());
    assert_eq!(seen(&b), ("cancelled".into(), Some("dropped".into())));
}

#[test]
fn a_hand_move_on_an_owned_card_is_refused_in_plain_words_and_changes_nothing() {
    let b = board();
    enqueue(&b, Definition::coding());
    let before = seen(&b);
    let user = Actor::User;
    let line = NewTicketLine { reason: "x".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None };
    let results = [
        b.store.set_task_status(b.task.id, TaskStatus::Done, user).err(),
        b.store.set_wait(b.task.id, Some(Wait::Parked), user).err(),
        b.store.put_on_line(b.task.id, &line, user).err(),
        b.store.update_line(b.task.id, &LineUpdate { branch: Some("c".into()), ..Default::default() }, user).err(),
    ];
    for r in results {
        let e = r.expect("refused");
        assert!(matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned" }), "{e:?}");
        assert!(e.to_string().contains("workflow") && !e.to_string().to_lowercase().contains("cancel"), "plain words that promise nothing: {e}");
    }
    assert_eq!(seen(&b), before, "the card did not move");
    // Outside a run the hand still moves it.
    assert!(b.store.set_task_status(b.other.id, TaskStatus::InProgress, user).is_ok());
}

#[test]
fn an_engine_move_is_told_once_after_the_commit_and_a_replay_tells_nothing() {
    use std::sync::{Arc, Mutex};
    let b = board();
    let told: Arc<Mutex<Vec<crate::BoardMove>>> = Default::default();
    let sink = told.clone();
    b.store.set_workflow_board_listener(Some(Arc::new(move |m| sink.lock().unwrap().extend_from_slice(m))));
    let c = Command::Enqueue { item: b.task.key.clone(), policy_pin: "policy-1".into(), resource: "own".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: Definition::coding() };
    let i = input(&b, "enqueue", 0, Grant::Controller, c);
    b.store.workflow_apply(&i).unwrap();
    {
        let told = told.lock().unwrap();
        assert_eq!(told.len(), 1, "one card moved: {told:?}");
        assert_eq!((told[0].task, told[0].from, told[0].to), (b.task.id, Some(TaskStatus::Backlog), TaskStatus::InProgress));
    }
    b.store.workflow_apply(&i).unwrap();
    assert_eq!(told.lock().unwrap().len(), 1, "a replay moves nothing, so tells nothing");
    // A refused command projects nothing.
    let again = input(&b, "dup", 1, Grant::Controller, Command::Cancel { item: "no-such".into() });
    let _ = b.store.workflow_apply(&again);
    assert_eq!(told.lock().unwrap().len(), 1);
}

#[test]
fn an_ended_run_refuses_a_hand_move_without_promising_the_card_comes_free() {
    let b = board();
    enqueue(&b, Definition::tracker("owner"));
    b.store.workflow_apply(&input(&b, "cancel", 1, Grant::Controller, Command::Cancel { item: b.task.key.clone() })).unwrap();
    let e = b.store.set_task_status(b.task.id, TaskStatus::InProgress, Actor::User).unwrap_err();
    assert!(matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned_ended" }), "{e:?}");
    assert!(!e.to_string().to_lowercase().contains("cancel"), "cancelling is what ended it: {e}");
    let live = b.store.set_task_status(b.other.id, TaskStatus::InProgress, Actor::User);
    assert!(live.is_ok());
}

#[test]
fn a_failed_run_is_on_the_needs_you_list_until_the_controller_cancels_it() {
    let b = board();
    enqueue(&b, Definition::coding());
    let mut revision = 1;
    // The provision stage runs out of attempts.
    for n in 0..5 {
        let state = b.store.workflow_state(&b.ws.to_string()).unwrap().unwrap();
        if state.items[&b.task.key].phase == Phase::Failed { break; }
        let item = &state.items[&b.task.key];
        let effect = item.pending.clone().expect("a stage is in flight");
        let claim = b.store.workflow_claim_effect(&state.workspace, &effect, "fixture-runtime", state.revision, state.epoch, 1000, 2000).unwrap().unwrap();
        assert!(b.store.workflow_admit_effect(&claim, 1001).unwrap());
        let c = Command::Observe { item: b.task.key.clone(), effect: effect.clone(), run: item.run.clone(), fence: item.fence, outcome: Outcome::SafeFailure };
        let grant = Grant::Operation { effect, run: item.run.clone(), fence: item.fence };
        revision = b.store.workflow_apply(&input(&b, &format!("fail-{n}"), state.revision, grant, c)).unwrap().result.unwrap();
    }
    assert_eq!(seen(&b), ("needs_decision".into(), Some("needs_you".into())), "the failure waits on a person");
    let e = b.store.set_task_status(b.task.id, TaskStatus::Done, Actor::User).unwrap_err();
    assert!(matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned" }), "{e:?}");
    // The orchestrator's way out is the same Cancel it uses on any run.
    b.store.workflow_apply(&input(&b, "cancel", revision, Grant::Controller, Command::Cancel { item: b.task.key.clone() })).unwrap();
    assert_eq!(seen(&b), ("cancelled".into(), Some("dropped".into())));
    let e = b.store.set_task_status(b.task.id, TaskStatus::Done, Actor::User).unwrap_err();
    assert!(matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned_ended" }), "{e:?}");
}

fn command(b: &Board, n: &str, command: Command) -> Result<u64, Refusal> {
    let state = b.store.workflow_state(&b.ws.to_string()).unwrap().unwrap();
    b.store.workflow_apply(&input(b, n, state.revision, Grant::Controller, command)).unwrap().result
}

#[test]
fn a_tracked_ticket_sits_in_the_column_the_orchestrator_put_it_in() {
    let b = board();
    enqueue(&b, Definition::ticket());
    assert_eq!(seen(&b), ("backlog".into(), None), "nothing is asked of anyone, and no line is made for it");
    for (to, status) in [("todo", "todo"), ("doing", "in_progress"), ("review", "in_review"), ("decision", "needs_decision"), ("backlog", "backlog")] {
        command(&b, &format!("move-{to}"), Command::Move { item: b.task.key.clone(), to: to.into() }).unwrap();
        assert_eq!(seen(&b), (status.into(), None), "{to}: only the column moves");
    }
    command(&b, "close", Command::Move { item: b.task.key.clone(), to: "done".into() }).unwrap();
    assert_eq!(seen(&b).0, "done");
    assert!(b.store.set_task_status(b.task.id, TaskStatus::Todo, Actor::User).is_err(), "the column is final");
}

#[test]
fn a_started_ticket_follows_its_run_from_the_column_it_was_in() {
    let b = board();
    enqueue(&b, Definition::ticket());
    command(&b, "doing", Command::Move { item: b.task.key.clone(), to: "doing".into() }).unwrap();
    let coding = Definition::coding();
    command(&b, "start", Command::Start { item: b.task.key.clone(), policy_pin: "policy-1".into(), resource: "repo".into(),
        dependencies: BTreeSet::new(), initial_candidate: None, definition: coding }).unwrap();
    assert_eq!(seen(&b), ("in_progress".into(), Some("building".into())));
    pass(&b, 1); pass(&b, 2); pass(&b, 3);
    assert_eq!(seen(&b), ("in_review".into(), Some("reviewing".into())));
}

/// A tracked ticket keeps its wait: a hold is the card's own note, so tracking every card must not make a
/// waiting card unmanageable (ov-962). The column stays the engine's, so a held Todo card stays in Todo.
#[test]
fn a_tracked_ticket_can_be_held_and_released_but_a_run_still_refuses() {
    let b = board();
    enqueue(&b, Definition::ticket());
    b.store.set_task_status(b.task.id, TaskStatus::Backlog, Actor::User).expect_err("its column is the engine's");
    let held = b.store.set_wait(b.task.id, Some(Wait::Parked), Actor::User).expect("a ticket may be parked");
    assert_eq!(held.status, TaskStatus::Backlog);
    assert!(b.store.set_wait(b.task.id, None, Actor::User).is_ok(), "and released");
    // A timed hold is waited for and lets go, which a sweep that skipped every owned card never did.
    let now = crate::tasks::now_millis();
    b.store.set_wait(b.task.id, Some(Wait::Until(now + 60_000)), Actor::User).unwrap();
    assert_eq!(b.store.next_hold_due().unwrap(), Some(now + 60_000));
    assert_eq!(b.store.release_due_holds(now + 120_000).unwrap().len(), 1, "the ticket's hold let go");
    assert_eq!(b.store.next_hold_due().unwrap(), None);
    // Once a run owns it, the wait is refused as ever.
    let b = board();
    enqueue(&b, Definition::coding());
    let e = b.store.set_wait(b.task.id, Some(Wait::Parked), Actor::User).expect_err("a run owns it");
    assert!(matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned" }), "{e:?}");
}

/// A rep's desk card is a person's thread, not work: tracking skips it.
#[test]
fn a_reps_desk_card_is_never_tracked() {
    let b = board();
    let handle = "alice";
    b.store.conn().execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    b.store.conn().execute("INSERT INTO rep_desks(workspace_id, handle, task_id) VALUES(?1,?2,?3)",
        rusqlite::params![b.ws.as_bytes().as_slice(), handle, b.task.id.as_bytes().as_slice()]).unwrap();
    assert!(b.store.untracked_tickets(None).unwrap().is_empty(), "a board with nothing published tracks nothing");
    let definition = Definition::tracker("owner");
    b.store.workflow_wire_write(&b.ws.to_string(), "publish", "publish", crate::workflow_publication::Write::Publish {
        expected_pin: None, definition: &definition, adapter: None, notifications: None, revision: 0 }).unwrap();
    let tracked: Vec<String> = b.store.untracked_tickets(None).unwrap().into_iter().map(|(_, k, _)| k).collect();
    assert_eq!(tracked, vec![b.other.key.clone()], "only the real card is tracked");
}

/// Tracking a card must not rewrite the line it is already on: its stage, rank and hold are the board's own
/// history (ov-962). Only the run that Start makes projects the line.
#[test]
fn tracking_a_card_leaves_its_line_rank_and_hold_alone() {
    let b = board();
    let line = NewTicketLine { reason: "x".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None };
    b.store.put_on_line(b.task.id, &line, Actor::User).unwrap();
    b.store.conn().execute("UPDATE ticket_lines SET stage='landing', plan_rank=3, held_off=1 WHERE task_id=?1", [b.task.id.as_bytes().as_slice()]).unwrap();
    let read = |b: &Board| -> (String, Option<i64>, i64) { b.store.conn().query_row(
        "SELECT stage, plan_rank, held_off FROM ticket_lines WHERE task_id=?1", [b.task.id.as_bytes().as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap() };
    let before = read(&b);
    enqueue(&b, Definition::ticket());
    assert_eq!(read(&b), before, "tracking rewrote the line");
}

/// A card the engine would refuse stays out of the tracking list, so asking each start doesn't write one
/// permanent refusal receipt per card (ov-962).
#[test]
fn a_card_with_a_live_worker_is_not_offered_for_tracking() {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("Projection");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let working = store.create_task(ws, "has a pane", Actor::User).unwrap();
    let free = store.create_task(ws, "free", Actor::User).unwrap();
    let definition = Definition::tracker("owner");
    store.workflow_wire_write(&ws.to_string(), "publish", "publish", crate::workflow_publication::Write::Publish {
        expected_pin: None, definition: &definition, adapter: None, notifications: None, revision: 0 }).unwrap();
    let tree = store.create_worktree(repo, "topic", "/isolated", false).unwrap().id;
    store.create_terminal_for_task(tree, "w", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(working.id)).unwrap();
    let keys = |s: &Store| -> Vec<String> { s.untracked_tickets(None).unwrap().into_iter().map(|(_, k, _)| k).collect() };
    assert_eq!(keys(&store), vec![free.key.clone()], "a live pane keeps its card out");
}

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
    let tracker = definition.source.starts_with("tracker");
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
        assert!(e.to_string().contains("workflow"), "plain words, not a Rust error: {e}");
    }
    assert_eq!(seen(&b), before, "the card did not move");
    // Outside a run the hand still moves it.
    assert!(b.store.set_task_status(b.other.id, TaskStatus::InProgress, user).is_ok());
}

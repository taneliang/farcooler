//! ov-946: after release the fences guard only what the engine owns. Every
//! ordinary board, pane and check-in command still works on every other card.
use super::*;
use crate::models::Actor;
use crate::ticket_lines::{LineUpdate, NewTicketLine};
use crate::waits::{Wait, WaitEvent};
use crate::workers::WorkerRecord;
use crate::models::{TaskStatus, NoteKind};

struct Released { _tmp: tempfile::TempDir, store: Store, ws: uuid::Uuid, free: uuid::Uuid, other: uuid::Uuid, owned: uuid::Uuid, intent: uuid::Uuid, terminal: uuid::Uuid, tree: uuid::Uuid }

fn released() -> Released {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("Board");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let free = store.create_task(ws.id, "free", Actor::User).unwrap().id;
    let other = store.create_task(ws.id, "other", Actor::User).unwrap().id;
    let owned = store.create_task(ws.id, "owned", Actor::User).unwrap();
    let intent = store.create_task(ws.id, "intent", Actor::User).unwrap().id;
    let tree = store.create_worktree(repo, "topic", "/isolated", false).unwrap().id;
    let terminal = store.create_terminal_for_test(tree, ws.id);
    let c = store.enable_check_in(crate::check_ins::EnableCheckIn { workspace: ws.id, task: free, device: None,
        expected_generation: None, expected_occurrence: None, acknowledge_uncertain: false, now: 1 });
    c.unwrap();
    store.conn().execute("INSERT INTO check_in_occurrences (id, workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at, template_version, due_at, state, reason)
        SELECT x'01010101010101010101010101010101', workspace_id, task_id, generation, eligibility_epoch, authorizer_source, authorized_at, template_version, next_due, 'claimed', 'fixture' FROM check_in_schedules", []).unwrap();
    store.conn().execute("INSERT INTO landings(id,workspace_id,number,route,base,pushed_sha,state,state_since,title,collecting,actor,created_at,resource_version) VALUES(x'02020202020202020202020202020202',?1,2,'push','main','exact-sha','pushed',2,'t',0,'user',1,7)", [ws.id.as_bytes().as_slice()]).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let plan = preflight(&store.conn()).unwrap();
    store.engine_quiesce(joined(&store)).unwrap();
    store.engine_commit_for_test(&plan, &proof(&store), "board").unwrap();
    store.engine_release(&plan, joined(&store)).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let definition = farcooler_workflow::Definition::tracker("owner");
    let input = farcooler_workflow::Input { schema: 1, id: "e".into(), workspace: ws.id.to_string(), expected_revision: 0, epoch: 1,
        source: "controller".into(), source_sequence: 1, now_ms: 0, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Enqueue { item: owned.key.clone(), policy_pin: definition.pin(), resource: "own".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } };
    assert!(store.workflow_apply(&input).unwrap().result.is_ok(), "the engine takes one card");
    Released { _tmp: tmp, store, ws: ws.id, free, other, owned: owned.id, intent, terminal, tree }
}

#[test]
fn released_board_commands_all_work_on_a_card_the_engine_does_not_own() {
    let r = released();
    let s = &r.store;
    let user = Actor::User;
    let line = NewTicketLine { reason: "plan".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None };
    type Step<'a> = (&'static str, Box<dyn Fn() -> bool + 'a>);
    let steps: Vec<Step<'_>> = vec![
        ("status", Box::new(|| s.set_task_status(r.free, TaskStatus::InProgress, user).is_ok())),
        ("block", Box::new(|| s.set_block(r.free, r.other, Some("because")).is_ok())),
        ("unblock", Box::new(|| s.unblock(r.free, r.other).is_ok())),
        ("hold", Box::new(|| s.set_wait(r.other, Some(Wait::Until(crate::tasks::now_millis() + 60_000)), user).is_ok())),
        ("park", Box::new(|| s.set_wait(r.other, Some(Wait::After(WaitEvent::Release)), user).is_ok())),
        ("clear", Box::new(|| s.set_wait(r.other, None, user).is_ok())),
        ("put on line", Box::new(|| s.put_on_line(r.free, &line, user).is_ok())),
        ("line set", Box::new(|| s.update_line(r.free, &LineUpdate { branch: Some("c".into()), ..Default::default() }, user).is_ok())),
        ("line order", Box::new(|| s.set_line_order(r.ws, &[r.free], user).is_ok())),
        ("answer", Box::new(|| s.add_note_waking(r.free, NoteKind::Answer, user, "yes", serde_json::json!({}), None, false).is_ok())),
        ("worker", Box::new(|| s.record_worker(r.free, &WorkerRecord { harness: "claude".into(), agent_id: "a1".into(), session_id: None, session_cwd: None,
            orchestrator_terminal: None, label: None, model: None, linked_by: crate::workers::LinkedBy::Orchestrator }, user).is_ok())),
        ("stop pane", Box::new(|| s.reserve_check_in_target_change(r.terminal, uuid::Uuid::from_bytes([9; 16]), "stop", None).is_ok())),
        ("check-in off", Box::new(|| s.disable_check_in(r.ws, None, None).is_ok())),
        ("startup recovery", Box::new(|| s.recover_check_ins_after_exclusive_start().is_ok())),
        ("remove pane", Box::new(|| s.get_terminal(r.terminal).is_ok_and(|t| s.delete_terminal(r.terminal, t.resource_version).is_ok()))),
    ];
    let claimed = |state: &str| s.conn().query_row("SELECT count(*) FROM check_in_occurrences WHERE state=?1", [state], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(claimed("claimed"), 1, "an in-flight check-in survived the cutover");
    let failed: Vec<_> = steps.iter().filter(|(_, f)| !f()).map(|(n, _)| *n).collect();
    assert!(failed.is_empty(), "aborted after release: {failed:?}");
    assert_eq!(claimed("uncertain"), 1, "startup recovery settled the in-flight check-in");
    let _ = (r.tree, r.owned);
}

#[test]
fn released_board_still_fences_the_card_the_engine_owns() {
    let r = released();
    let s = &r.store;
    let engine_owned = |e: farcooler_core::DomainError| matches!(e, farcooler_core::DomainError::Conflict { what: "engine_owned" });
    assert!(s.set_task_status(r.owned, TaskStatus::Done, Actor::User).is_err_and(engine_owned), "status is the engine's");
    assert!(s.set_wait(r.owned, Some(Wait::Parked), Actor::User).is_err_and(engine_owned), "so is its wait");
    assert!(s.put_on_line(r.owned, &NewTicketLine { reason: "x".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None }, Actor::User).is_err());
    assert!(s.set_block(r.owned, r.other, None).is_err(), "its dependencies are the engine's");
    let refused = s.create_terminal_for_task(r.tree, "w", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(r.owned));
    assert!(refused.is_err_and(engine_owned), "a trigger's refusal says the engine owns it, not that the caller should retry");
    assert!(s.set_block(r.owned, r.other, None).is_err_and(engine_owned));
}

#[test]
fn released_holds_still_let_go_and_ci_chips_no_longer_move_landings() {
    use crate::board_ci::{CiRead, CiStatus};
    let r = released();
    let s = &r.store;
    assert!(!s.legacy_authority(), "the daemon can tell the loops to stop");
    // A hold whose time came: the free card lets go and is told once. A card the
    // engine owns is left alone (its fence is lifted here only to seed the hold).
    let past = crate::tasks::now_millis() - 1;
    s.conn().execute_batch("DROP TRIGGER engine_task_wait_fence").unwrap();
    s.conn().execute("UPDATE tasks SET wait_kind='until', wait_until=?1 WHERE id IN (?2,?3)", rusqlite::params![past, r.other.as_bytes().as_slice(), r.owned.as_bytes().as_slice()]).unwrap();
    let released = s.release_due_holds(crate::tasks::now_millis()).unwrap();
    assert_eq!(released.iter().map(|(t, _)| t.id).collect::<Vec<_>>(), vec![r.other], "only the unowned card");
    assert_eq!(s.pending_hold_wakes().unwrap().len(), 1, "and the orchestrator is told, through the engine");
    assert_eq!(s.next_hold_due().unwrap(), None, "the owned card's hold is not ours to wait for");
    // A CI read moves the chip and only the chip: the pushed landing stays where it was.
    let read = CiRead { subject: crate::board_ci::sha_subject("exact-sha"), sha: "exact-sha".into(), status: CiStatus::Passed, url: String::new(),
        jobs: Vec::new(), fetched_at: 5, changed_at: 5, asked_at: 5 };
    assert!(s.record_ci(r.ws, &read).unwrap().landings_moved.is_empty());
    assert_eq!(s.conn().query_row("SELECT state FROM landings", [], |x| x.get::<_, String>(0)).unwrap(), "pushed");
    assert_eq!(s.conn().query_row("SELECT status FROM board_ci", [], |x| x.get::<_, String>(0)).unwrap(), "passed");
}

fn enqueue(r: &Released, id: &str, key: &str) -> farcooler_workflow::Input {
    let definition = farcooler_workflow::Definition::tracker("owner");
    farcooler_workflow::Input { schema: 1, id: id.into(), workspace: r.ws.to_string(), expected_revision: 1, epoch: 1,
        source: "controller".into(), source_sequence: 2, now_ms: 0, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Enqueue { item: key.into(), policy_pin: definition.pin(), resource: "own2".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } }
}

#[test]
fn a_migrated_card_with_a_hand_agent_refuses_the_engine() {
    let r = released();
    let s = &r.store;
    let key = s.get_task(r.intent).unwrap().key;
    let disposition: String = s.conn().query_row("SELECT disposition FROM engine_cutover_items WHERE task=?1", [r.intent.as_bytes().as_slice()], |x| x.get(0)).unwrap();
    assert_eq!(disposition, "retained-intent", "the fixture card is a migrated retained-intent card");
    s.create_terminal_for_task(r.tree, "hand", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(r.intent)).unwrap();
    assert_eq!(s.workflow_apply(&enqueue(&r, "e2", &key)).unwrap().result, Err(farcooler_workflow::Refusal::Resource),
        "a hand agent and the engine must never run one card");
    // And the other side: once a hand agent has left, the same card is free to enqueue.
    let r2 = released();
    let key2 = r2.store.get_task(r2.intent).unwrap().key;
    assert!(r2.store.workflow_apply(&enqueue(&r2, "e3", &key2)).unwrap().result.is_ok(), "an idle migrated card is takeable");
}

#[test]
fn a_stage_move_into_or_out_of_landing_works_after_release() {
    use crate::ticket_lines::Stage;
    let r = released();
    let s = &r.store;
    let user = Actor::User;
    let up = |stage| LineUpdate { stage: Some(stage), ..Default::default() };
    s.put_on_line(r.free, &NewTicketLine { reason: "plan".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None }, user).unwrap();
    for stage in [Stage::Building, Stage::Reviewing] { s.update_line(r.free, &up(stage), user).unwrap(); }
    let into = s.update_line(r.free, &up(Stage::Landing), user);
    assert!(into.is_ok(), "into landing: {into:?}");
    assert!(s.landing_of(r.free).unwrap().is_none(), "no legacy landing is joined after release");
    // A card stranded on an open landing at cutover can still be moved back by hand.
    s.conn().execute_batch("DROP TRIGGER engine_fence_landings_i; DROP TRIGGER engine_fence_landing_tickets_i").unwrap();
    s.conn().execute("INSERT INTO landings(id,workspace_id,number,route,base,state,state_since,title,collecting,actor,created_at,resource_version) VALUES(x'03030303030303030303030303030303',?1,3,'push','main','open',2,'t',1,'user',1,1)", [r.ws.as_bytes().as_slice()]).unwrap();
    s.conn().execute("INSERT INTO landing_tickets(landing_id,task_id,added_at) SELECT x'03030303030303030303030303030303',?1,1", [r.free.as_bytes().as_slice()]).unwrap();
    assert!(s.update_line(r.free, &up(Stage::Building), user).is_ok(), "out of landing");
}

#[test]
fn a_main_ci_read_after_release_does_not_send_the_collecting_landing_off() {
    use crate::board_ci::{CiRead, CiStatus, MAIN_SUBJECT};
    let r = released();
    let s = &r.store;
    // Lift the landing fences so only the legacy gate, not the fence, can keep depart out.
    for table in ["landings", "landing_tickets"] {
        for suffix in ["i", "u", "d"] { s.conn().execute_batch(&format!("DROP TRIGGER engine_fence_{table}_{suffix}")).unwrap(); }
    }
    s.conn().execute("INSERT INTO landings(id,workspace_id,number,route,base,state,state_since,title,collecting,actor,created_at,resource_version) VALUES(x'04040404040404040404040404040404',?1,4,'push','main','open',2,'',1,'user',1,1)", [r.ws.as_bytes().as_slice()]).unwrap();
    s.conn().execute("INSERT INTO landing_tickets(landing_id,task_id,added_at) VALUES(x'04040404040404040404040404040404',?1,1)", [r.other.as_bytes().as_slice()]).unwrap();
    // Make depart eligible: no landing ahead, and a ready ticket on the collecting one.
    s.conn().execute("UPDATE landings SET state='landed' WHERE number=2", []).unwrap();
    s.put_on_line(r.other, &NewTicketLine { reason: "p".into(), branch: "o".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None }, Actor::User).unwrap();
    s.conn().execute("UPDATE ticket_lines SET stage='landing' WHERE task_id=?1", [r.other.as_bytes().as_slice()]).unwrap();
    let read = CiRead { subject: MAIN_SUBJECT.into(), sha: "abc".into(), status: CiStatus::Passed, url: String::new(),
        jobs: Vec::new(), fetched_at: 5, changed_at: 5, asked_at: 5 };
    let write = s.record_ci(r.ws, &read).unwrap();
    assert!(write.landings_moved.is_empty());
    assert_eq!(s.conn().query_row("SELECT state||collecting FROM landings WHERE number=4", [], |x| x.get::<_, String>(0)).unwrap(), "open1", "the collecting landing stays");
}

#[test]
fn hourly_check_ins_still_fire_after_release() {
    use rusqlite::params;
    let r = released();
    let s = &r.store;
    let ws = r.ws;
    s.recover_check_ins_after_exclusive_start().unwrap();
    // The cutover left a settled schedule; make one that is due now on a card the engine does not own.
    s.conn().execute("DELETE FROM check_in_occurrences", []).unwrap();
    let repo = s.get_workspace(ws).unwrap().repository_id;
    let tree = s.create_worktree(repo, "test/check-in", "/check-in", false).unwrap();
    let pane = s.create_terminal(tree.id, "manager", "codex", farcooler_protocol::v1::TerminalIntent::Running, 80, 24).unwrap();
    s.conn().execute("UPDATE terminals SET workspace_id=?2, role=2, runtime_confirmed=1, agent_session_id='thread' WHERE id=?1",
        params![pane.id.as_bytes().as_slice(), ws.as_bytes().as_slice()]).unwrap();
    let generation: Vec<u8> = s.conn().query_row("SELECT generation FROM check_in_schedules", [], |x| x.get(0)).unwrap();
    s.enable_check_in(crate::check_ins::EnableCheckIn { workspace: r.ws, task: r.free, device: None,
        expected_generation: Some(uuid::Uuid::from_slice(&generation).unwrap()), expected_occurrence: None, acknowledge_uncertain: true, now: 1 }).unwrap();
    s.conn().execute("UPDATE workspaces SET wake_on_answer=1 WHERE id=?1", [ws.as_bytes().as_slice()]).unwrap();
    let sched: Vec<(String, i64)> = s.conn().prepare("SELECT state, next_due FROM check_in_schedules").unwrap().query_map([], |x| Ok((x.get(0)?, x.get(1)?))).unwrap().map(|x| x.unwrap()).collect();
    assert!(s.tick_check_in(ws, 3_600_001).unwrap(), "an occurrence became due: {sched:?}");
    let occ = s.pending_check_ins().unwrap()[0].id;
    let (attempt, daemon) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    assert!(s.claim_check_in(occ, attempt, daemon, pane.id, "thread", "%1", 123, 1, 3_600_002).unwrap(), "claimed");
    assert!(s.pin_check_in_rollout(crate::PinCheckInRollout { occurrence: occ, attempt, path: "/rollout", tty: "/dev/test", incarnation: 7, device: 1, inode: 2, offset: 0 }).unwrap());
    assert!(s.admit_check_in_enter(occ, attempt, daemon, pane.id, "thread", "%1", 123, 1, 7, 3_600_003).unwrap(), "submitted");
    assert!(s.settle_check_in_enter(occ, attempt, 3_600_004).unwrap());
}

#[test]
fn released_agents_work_panes_and_moves_on_an_unowned_card_all_work() {
    use crate::agents::{AgentPlace, AgentRole, NewAgent};
    use crate::ticket_work::{LinkedBy, Verdict, WorkRole};
    let r = released();
    let s = &r.store;
    let user = Actor::User;
    let agent = s.record_agent(r.ws, &NewAgent { role: AgentRole::Builder, place: AgentPlace::Subagent { id: "sub-1".into(), parent: None },
        harness: "claude".into(), session_id: None, session_cwd: None, model: None, title: Some("ov-1: build".into()) }).expect("agent dispatch");
    s.start_work(agent.id, r.free, WorkRole::Build, LinkedBy::Orchestrator, user).expect("agent work");
    let pane = s.create_terminal_for_task(r.tree, "w", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(r.free)).expect("agent-pane task start");
    s.reserve_check_in_target_change(pane.id, uuid::Uuid::from_bytes([8; 16]), "stop", None).expect("task-bound pane stop");
    s.end_work(agent.id, r.free, WorkRole::Build, Verdict::Finished, user).expect("agent work ends");
    s.end_agent(agent.id, Verdict::Finished).expect("agent end");
    let other = s.create_workspace(s.get_workspace(r.ws).unwrap().repository_id, "Second", "sc").expect("a second board");
    s.move_tasks(&[r.free], other.id, user).expect("task move");
}

#[test]
fn released_engine_moves_the_card_it_owns_and_leaves_the_catalog_as_it_was() {
    let r = released();
    let s = &r.store;
    let catalog = || -> Vec<(String, Option<String>)> {
        let conn = s.conn();
        let mut q = conn.prepare("SELECT name, sql FROM sqlite_master ORDER BY type, name").unwrap();
        q.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<std::result::Result<_, rusqlite::Error>>().unwrap()
    };
    let status = |t: uuid::Uuid| s.get_task(t).unwrap().status;
    // The tracker run waits on its owner, and `released()` has already enqueued it.
    assert_eq!(status(r.owned), TaskStatus::NeedsDecision, "the engine moved the owned card after release");
    assert_eq!(status(r.free), TaskStatus::Backlog, "and no other card");
    let before = catalog();
    let cancel = farcooler_workflow::Input { schema: 1, id: "c".into(), workspace: r.ws.to_string(), expected_revision: 1, epoch: 1,
        source: "controller".into(), source_sequence: 2, now_ms: 1, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Cancel { item: s.get_task(r.owned).unwrap().key } };
    assert!(s.workflow_apply(&cancel).unwrap().result.is_ok());
    assert_eq!(status(r.owned), TaskStatus::Cancelled);
    let stage: String = s.conn().query_row("SELECT stage FROM ticket_lines WHERE task_id=?1", [r.owned.as_bytes().as_slice()], |row| row.get(0)).unwrap();
    assert_eq!(stage, "dropped");
    assert_eq!(catalog(), before, "the fences the projection stepped past are back, word for word");
    assert!(s.set_task_status(r.owned, TaskStatus::Todo, Actor::User).is_err(), "and still fence a hand");
}

#[test]
fn released_empty_workspace_with_landing_history_says_why_it_stays() {
    let r = released();
    let s = &r.store;
    let repo: uuid::Uuid = s.conn().query_row("SELECT repository_id FROM workspaces WHERE id=?1", [r.ws.as_bytes().as_slice()], |x| Ok(uuid::Uuid::from_slice(&x.get::<_, Vec<u8>>(0)?).unwrap())).unwrap();
    let empty = s.create_workspace(repo, "Empty", "EM").unwrap();
    assert!(s.delete_workspace(empty.id).is_ok(), "no landing history: it goes");
    let kept = s.create_workspace(repo, "Kept", "KP").unwrap();
    s.conn().execute_batch("DROP TRIGGER engine_fence_landings_i").unwrap();
    s.conn().execute("INSERT INTO landings(id,workspace_id,number,route,base,pushed_sha,state,state_since,title,collecting,actor,created_at,resource_version) VALUES(x'03030303030303030303030303030303',?1,1,'push','main','sha','landed',2,'t',0,'user',1,1)", [kept.id.as_bytes().as_slice()]).unwrap();
    let err = s.delete_workspace(kept.id).unwrap_err();
    assert!(matches!(err, farcooler_core::DomainError::InvalidArgument { what: "landing_history" }), "not the not-empty copy: {err:?}");
}

/// A new card the engine holds as a ticket with no run, on a released board.
fn tracked_ticket(r: &Released) -> crate::models::Task {
    let s = &r.store;
    let card = s.create_task(r.ws, "ticket", Actor::User).unwrap();
    let definition = farcooler_workflow::Definition::ticket();
    let policy_pin = s.workflow_ticket_bundle(&definition).unwrap();
    let input = farcooler_workflow::Input { schema: 1, id: "t".into(), workspace: r.ws.to_string(), expected_revision: 1, epoch: 1,
        source: "controller".into(), source_sequence: 2, now_ms: 0, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Enqueue { item: card.key.clone(), policy_pin, resource: "own-ticket".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } };
    assert!(s.workflow_apply(&input).unwrap().result.is_ok(), "the engine tracks the ticket");
    card
}

/// ov-962 M-g: a coordinator's subagent is recorded on a tracked ticket, and still refused on a card with a run.
#[test]
fn a_subagent_is_recorded_on_a_tracked_ticket_after_release() {
    let r = released();
    let s = &r.store;
    let card = tracked_ticket(&r);
    let record = |id: &str| WorkerRecord { harness: "claude".into(), agent_id: id.into(), session_id: None, session_cwd: None,
        orchestrator_terminal: None, label: Some("review".into()), model: None, linked_by: crate::workers::LinkedBy::Orchestrator };
    s.record_worker(card.id, &record("a1"), Actor::User).expect("a ticket takes its subagent");
    s.record_worker(card.id, &record("a1"), Actor::User).expect("and a resumed one");
    s.end_worker(card.id, "claude", Some("a1"), crate::workers::EndReason::Finished, Actor::User).expect("and sees it finish");
    assert!(s.record_worker(r.owned, &record("a2"), Actor::User).is_err(), "a card with a run does not");
    assert!(s.set_task_status(card.id, TaskStatus::Done, Actor::User).is_err(), "the fences are back");
}

/// ov-962 B4: a ticket the engine holds with no run keeps its wait and its blocks after release, and a timed
/// hold lets go. The fences stay for a card with a run, and are back, byte for byte, after each write.
#[test]
fn a_tracked_ticket_keeps_its_wait_and_blocks_after_release() {
    let r = released();
    let s = &r.store;
    let card = tracked_ticket(&r);
    assert!(s.task_is_tracked_ticket(card.id).unwrap());
    let triggers = || -> Vec<String> { let c = s.conn(); let mut q = c.prepare("SELECT sql FROM sqlite_master WHERE type='trigger' ORDER BY name").unwrap();
        q.query_map([], |x| x.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
    let before = triggers();
    s.set_wait(card.id, Some(Wait::Parked), Actor::User).expect("a ticket parks after release");
    s.set_wait(card.id, None, Actor::User).expect("and is let go");
    s.set_block(card.id, r.other, Some("because")).expect("a ticket takes a block after release");
    assert_eq!(s.blocks_for(card.id).unwrap().len(), 1);
    s.unblock(card.id, r.other).unwrap();
    assert!(s.blocks_for(card.id).unwrap().is_empty(), "and drops it");
    // A timed hold is waited for, and lets go when its time comes, with its note and its wake.
    let now = crate::tasks::now_millis();
    s.set_wait(card.id, Some(Wait::Until(now + 60_000)), Actor::User).expect("a ticket takes a timed hold");
    assert_eq!(s.next_hold_due().unwrap(), Some(now + 60_000), "the sweep waits for it");
    let released = s.release_due_holds(now + 120_000).unwrap();
    assert_eq!(released.iter().map(|(t, _)| t.id).collect::<Vec<_>>(), vec![card.id], "the hold let go");
    assert_eq!(s.next_hold_due().unwrap(), None);
    assert_eq!(triggers(), before, "every fence is back as it was");
    // The column is still the engine's, and a card with a run still refuses all of it.
    assert!(s.set_task_status(card.id, TaskStatus::Done, Actor::User).is_err());
    assert!(s.set_wait(r.owned, Some(Wait::Parked), Actor::User).is_err());
    assert!(s.set_block(r.owned, r.other, None).is_err());
}

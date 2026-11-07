//! ov-946: after release the fences guard only what the engine owns. Every
//! ordinary board, pane and check-in command still works on every other card.
use super::*;
use crate::models::Actor;
use crate::ticket_lines::{LineUpdate, NewTicketLine};
use crate::waits::{Wait, WaitEvent};
use crate::workers::WorkerRecord;
use crate::models::{TaskStatus, NoteKind};

struct Released { _tmp: tempfile::TempDir, store: Store, ws: uuid::Uuid, free: uuid::Uuid, other: uuid::Uuid, owned: uuid::Uuid, terminal: uuid::Uuid, tree: uuid::Uuid }

fn released() -> Released {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("Board");
    let ws = store.ensure_main_workspace(repo).unwrap();
    let free = store.create_task(ws.id, "free", Actor::User).unwrap().id;
    let other = store.create_task(ws.id, "other", Actor::User).unwrap().id;
    let owned = store.create_task(ws.id, "owned", Actor::User).unwrap();
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
    Released { _tmp: tmp, store, ws: ws.id, free, other, owned: owned.id, terminal, tree }
}

#[test]
fn released_board_commands_all_work_on_a_card_the_engine_does_not_own() {
    let r = released();
    let s = &r.store;
    let user = Actor::User;
    let line = NewTicketLine { reason: "plan".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None };
    let steps: Vec<(&str, Box<dyn Fn() -> bool + '_>)> = vec![
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
    assert!(s.set_task_status(r.owned, TaskStatus::Done, Actor::User).is_err(), "status is the engine's");
    assert!(s.set_wait(r.owned, Some(Wait::Parked), Actor::User).is_err(), "so is its wait");
    assert!(s.put_on_line(r.owned, &NewTicketLine { reason: "x".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None }, Actor::User).is_err());
    assert!(s.set_block(r.owned, r.other, None).is_err(), "its dependencies are the engine's");
    assert!(s.create_terminal_for_task(r.tree, "w", "claude", farcooler_protocol::v1::TerminalIntent::Running, 80, 24, Some(r.owned)).is_err());
}

#[test]
fn released_holds_still_let_go_and_ci_chips_no_longer_move_landings() {
    use crate::board_ci::{CiRead, CiStatus};
    let r = released();
    let s = &r.store;
    assert!(!s.legacy_authority(), "the daemon can tell the loops to stop");
    // A hold whose time came: the free card lets go with no legacy wake. A card the
    // engine owns is left alone (its fence is lifted here only to seed the hold).
    let past = crate::tasks::now_millis() - 1;
    s.conn().execute_batch("DROP TRIGGER engine_task_execution_fence").unwrap();
    s.conn().execute("UPDATE tasks SET wait_kind='until', wait_until=?1 WHERE id IN (?2,?3)", rusqlite::params![past, r.other.as_bytes().as_slice(), r.owned.as_bytes().as_slice()]).unwrap();
    let released = s.release_due_holds(crate::tasks::now_millis()).unwrap();
    assert_eq!(released.iter().map(|(t, _)| t.id).collect::<Vec<_>>(), vec![r.other], "only the unowned card");
    assert_eq!(s.conn().query_row("SELECT count(*) FROM hold_wakes", [], |x| x.get::<_, i64>(0)).unwrap(), 0, "no fenced wake queued");
    assert_eq!(s.next_hold_due().unwrap(), None, "the owned card's hold is not ours to wait for");
    // A CI read moves the chip and only the chip: the pushed landing stays where it was.
    let read = CiRead { subject: crate::board_ci::sha_subject("exact-sha"), sha: "exact-sha".into(), status: CiStatus::Passed, url: String::new(),
        jobs: Vec::new(), fetched_at: 5, changed_at: 5, asked_at: 5 };
    assert!(s.record_ci(r.ws, &read).unwrap().landings_moved.is_empty());
    assert_eq!(s.conn().query_row("SELECT state FROM landings", [], |x| x.get::<_, String>(0)).unwrap(), "pushed");
    assert_eq!(s.conn().query_row("SELECT status FROM board_ci", [], |x| x.get::<_, String>(0)).unwrap(), "passed");
}

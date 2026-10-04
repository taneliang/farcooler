//! What the runner's observing of subagents (ov-213) writes in the store.

use farcooler_core::usage::TokenCounts;

use super::*;
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use crate::wakes::WakeKind;

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Sidebar polish", Actor::Manager).unwrap().id;
    (store, main, task)
}

fn record(agent: &str) -> WorkerRecord {
    WorkerRecord {
        harness: "claude".into(),
        agent_id: agent.into(),
        session_id: Some("76e86926".into()),
        session_cwd: Some("/r".into()),
        orchestrator_terminal: None,
        label: None,
        model: None,
        linked_by: LinkedBy::Orchestrator,
    }
}

fn subagent_turn(agent: &str, task: Option<Uuid>) -> NewTurn {
    NewTurn {
        key: format!("claude-log:agent:{agent}"),
        terminal_id: None,
        worktree_id: None,
        repository_id: None,
        workspace_id: None,
        task_id: task,
        harness: "claude".into(),
        surface: Surface::Terminal,
        started_at: Some(1_000),
        ended_at: 2_000,
        active_ms: Some(1_000),
        usage: "reported",
        models: vec![TurnModel::priced(
            Some("claude-opus-5".into()),
            TokenCounts { output: 500, ..Default::default() },
            None,
        )],
        kind: TurnKind::Subagent,
    }
}

/// The follower reads a subagent's transcript from the moment the session is
/// followed, so its spend exists before the orchestrator says which task it
/// works. Recording it then moves what was spent onto that task.
#[test]
fn spend_written_before_the_link_moves_to_the_task() {
    let (store, _, task) = board();
    store.record_turn(&subagent_turn("a1", None)).unwrap();
    assert_eq!(store.task_usage(task).unwrap().0.tokens.output, 0, "nobody's yet");
    store.record_worker(task, &record("a1"), Actor::Manager).unwrap();
    let (total, _) = store.task_usage(task).unwrap();
    assert_eq!((total.subagent_runs, total.tokens.output), (1, 500));
}

/// Another subagent's spend, and a codex one with the same id, stay where
/// they are.
#[test]
fn only_that_subagents_spend_moves() {
    let (store, _, task) = board();
    store.record_turn(&subagent_turn("a1", None)).unwrap();
    store.record_turn(&subagent_turn("a2", None)).unwrap();
    store.record_worker(task, &record("a2"), Actor::Manager).unwrap();
    let (total, _) = store.task_usage(task).unwrap();
    assert_eq!(total.subagent_runs, 1);
    assert_eq!(store.task_of_subagent("claude", "a2").unwrap(), Some(task));
    assert_eq!(store.task_of_subagent("claude", "a1").unwrap(), None);
}

/// A subagent that ended is still followed for a while, so a resume is
/// seen; one with no session is never followed.
#[test]
fn followed_workers_are_the_open_ones_and_the_recently_ended() {
    let (store, main, task) = board();
    let other = store.create_task(main, "Other", Actor::Manager).unwrap().id;
    store.record_worker(task, &record("open"), Actor::Manager).unwrap();
    store.record_worker(task, &record("ended"), Actor::Manager).unwrap();
    store.end_worker(task, "claude", Some("ended"), EndReason::Finished, Actor::Runner).unwrap();
    store.record_worker(other, &WorkerRecord { session_id: None, ..record("no-session") }, Actor::Manager).unwrap();
    let ids = |since: i64| {
        let mut ids: Vec<String> =
            store.followed_workers(since).unwrap().into_iter().map(|(w, ws)| {
                assert_eq!(ws, main);
                w.agent_id
            }).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(0), ["ended", "open"]);
    assert_eq!(ids(i64::MAX), ["open"], "the one that ended long ago is let go");
}

/// A hold that ended is queued apart from the answers, and each queue is
/// claimed and finished on its own.
#[test]
fn a_hold_that_ended_is_its_own_wake() {
    let (store, _, task) = board();
    let due = now_millis() + 60_000;
    store.set_wait(task, Some(crate::waits::Wait::Until(due)), Actor::Manager).unwrap();
    assert!(store.pending_hold_wakes().unwrap().is_empty());
    store.release_due_holds(due + 1).unwrap();
    let pending = store.pending_hold_wakes().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].kind, pending[0].task, pending[0].actor), (WakeKind::HoldEnded, task, Actor::Runner));
    assert!(pending[0].body.starts_with("Held until"), "{}", pending[0].body);
    assert!(store.pending_answer_wakes().unwrap().is_empty());
    assert!(store.claim_wake(&pending[0]).unwrap());
    assert!(!store.claim_wake(&pending[0]).unwrap(), "claimed once");
    assert!(store.finish_wake(&pending[0], Some("Told the orchestrator the hold ended")).unwrap().is_some());
    assert!(store.pending_hold_wakes().unwrap().is_empty());
    assert!(store.finish_wake(&pending[0], None).unwrap().is_none(), "finished once");
}

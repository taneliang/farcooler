//! The answer pump and subagents (ov-213), and a hold that ended (ov-212):
//! what the orchestrator is told, and when. On the same tmux server and
//! `stand_in.pl` as `tests`, whose board and helpers these use.

use farcooler_store::models::TaskStatus;
use farcooler_store::workers::{LinkedBy, WorkerRecord};
use farcooler_store::PendingWake;

use super::*;

const AGENT: &str = "a3fd8fceef581c787";

fn subagent(agent: &str, label: &str) -> WorkerRecord {
    WorkerRecord {
        harness: "claude".into(),
        agent_id: agent.into(),
        session_id: Some("76e86926".into()),
        session_cwd: Some("/r".into()),
        orchestrator_terminal: None,
        label: Some(label.into()),
        model: None,
        linked_by: LinkedBy::Orchestrator,
    }
}

// ---- the words ----

/// The message names the subagent by the id `SendMessage` takes, and its
/// label, and says to pass the decision on.
#[test]
fn the_message_names_the_subagent() {
    let said = message_for("ov-12", "Mac: polish", "Keep B", &[(AGENT.into(), "ov-12 Mac polish".into())]);
    assert_eq!(
        said,
        "Decision on ov-12 (“Mac: polish”): Keep B. Its subagent (a3fd8fceef581c787, “ov-12 Mac polish”) is working on it: pass this on, then continue."
    );
    let two = message_for("ov-12", "T", "Keep B", &[("a1".into(), "one".into()), ("a2".into(), String::new())]);
    assert!(two.contains("Its subagents (a1, “one”; a2) are working on it: pass this on"), "{two}");
    assert_eq!(message_for("ov-12", "T", "Keep B", &[]), message("ov-12", "T", "Keep B"));
}

/// A subagent's label is the orchestrator's words, and gets the same
/// treatment an answer does: nothing that acts.
#[test]
fn a_subagents_label_is_typed_as_text() {
    let said = message_for("ov-1", "T", "Yes", &[("a1".into(), "x\x1b[31m\r\ny".into())]);
    assert!(!said.chars().any(|c| c.is_control()), "{said:?}");
}

#[test]
fn the_hold_message_says_when_and_that_it_is_the_orchestrators_call() {
    assert_eq!(
        hold_message("ov-12", "Mac: polish", "Held until Oct 5, 9:00 AM. That time has come."),
        "Hold ended on ov-12 (“Mac: polish”): Held until Oct 5, 9:00 AM. That time has come. Start it when you're ready."
    );
}

// ---- an answer on a subagent's task ----

/// The orchestrator is told, and the message names the subagent open on the
/// task, which is who it should pass the decision to.
#[tokio::test]
async fn an_answer_on_a_subagents_task_names_the_subagent_to_the_orchestrator() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.svc.store.record_worker(b.task.id, &subagent(AGENT, "ov-12 Mac polish"), Actor::Manager).unwrap();
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.submits(1).await;
    let told = si.submitted();
    assert_eq!(told.len(), 1, "{}", si.log());
    assert_eq!(
        told[0],
        format!("Decision on {} (“Drill-in layout”): Drill in. Its subagent (a3fd8fceef581c787, “ov-12 Mac polish”) is working on it: pass this on, then continue.", b.task.key)
    );
    assert_eq!(b.progress(), ["Told the orchestrator about the decision"]);
}

/// A subagent that ended isn't named: it can't be told anything.
#[tokio::test]
async fn a_subagent_that_ended_is_not_named() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.svc.store.record_worker(b.task.id, &subagent(AGENT, "ov-12 Mac polish"), Actor::Manager).unwrap();
    b.svc
        .store
        .end_worker(b.task.id, "claude", Some(AGENT), farcooler_store::workers::EndReason::Finished, Actor::Runner)
        .unwrap();
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.answer("Drill in");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

// ---- a hold that ended ----

impl Board {
    /// Hold the task until a time that comes, and let it go: its wake is
    /// queued, as the watcher's tick does it. The tick is a minute after
    /// now (a hold can't be set in the past), so the wake reads as enqueued
    /// then.
    fn hold_ended(&self) {
        let due = now_millis() + 60_000;
        self.svc.store.set_wait(self.task.id, Some(farcooler_store::waits::Wait::Until(due)), Actor::Manager).unwrap();
        self.watcher.wakes_hint.store(false, Ordering::SeqCst);
        self.watcher.release_due_holds(due + 1);
    }

    fn holds(&self) -> Vec<PendingWake> {
        self.svc.store.pending_hold_wakes().unwrap()
    }

    fn hold_told(&self) -> String {
        let said = self.svc.store.notes_for(self.task.id, Some(NoteKind::Wait)).unwrap().pop().unwrap().body;
        hold_message(&self.task.key, "Drill-in layout", &said)
    }

    /// Nothing went near the pane, and the wake still waits.
    fn hold_untouched(&self, si: &StandIn) {
        let holds = self.holds();
        assert_eq!(holds.len(), 1, "{holds:?}");
        assert_eq!(holds[0].claimed_at, None, "claimed");
        assert!(!si.log().contains("PASTE") && !si.log().contains("ENTER"), "{}", si.log());
    }
}

/// Until's time comes: the task is let go, and its orchestrator, idle with an
/// empty box, is told in one pasted line, once.
#[tokio::test]
async fn a_hold_that_ended_is_told_to_the_idle_orchestrator() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.hold_ended();
    assert!(b.watcher.wakes_hint.load(Ordering::SeqCst), "the pump is asked to look");
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.hold_told()], "{}", si.log());
    assert!(si.log().contains("PASTE "), "a bracketed paste: {}", si.log());
    assert_eq!(b.progress(), ["Told the orchestrator the hold ended"]);
    assert!(b.holds().is_empty());
    b.pump().await;
    assert_eq!(si.submitted().len(), 1, "told once");
}

/// Check 1: the watcher reads it working, so the wake waits and says why,
/// and lands when it goes idle.
#[tokio::test]
async fn a_hold_that_ended_waits_for_a_busy_orchestrator() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    b.hold_ended();
    b.pump().await;
    b.hold_untouched(&si);
    assert_eq!(b.progress(), ["Waiting to tell the orchestrator the hold ended: it's busy."]);
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.hold_told()], "{}", si.log());
}

/// Check 2: someone typing there.
#[tokio::test]
async fn a_hold_that_ended_is_not_typed_over_someone_typing() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), orchestrator.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, now_millis().to_string()).unwrap();
    b.hold_ended();
    b.pump().await;
    b.hold_untouched(&si);
    std::fs::remove_file(&mark).unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.hold_told()], "{}", si.log());
}

/// Check 3: the pane's foreground is not the agent.
#[tokio::test]
async fn a_hold_that_ended_is_not_typed_to_a_process_that_isnt_an_agent() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "perl").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.hold_ended();
    b.pump().await;
    b.hold_untouched(&si);
    assert_eq!(b.progress(), ["Waiting to tell the orchestrator the hold ended: no agent is running in its pane."]);
}

/// Check 4: a draft in the box.
#[tokio::test]
async fn a_hold_that_ended_is_not_pasted_beside_a_draft() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.hold_ended();
    b.pump().await;
    b.hold_untouched(&si);
    assert_eq!(b.progress(), ["Waiting to tell the orchestrator the hold ended: there's a draft in its box."]);
}

/// Check 5: bracketed paste off.
#[tokio::test]
async fn a_hold_that_ended_is_not_pasted_without_bracketed_paste() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    si.show("nobracket").await;
    b.stream_says(orchestrator.id, Some(false)).await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.hold_ended();
    b.pump().await;
    b.hold_untouched(&si);
    si.show("idle").await;
    b.stream_says(orchestrator.id, Some(true)).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.hold_told()], "{}", si.log());
}

/// A task started since its hold ended has nothing to be told.
#[tokio::test]
async fn a_hold_that_ended_is_dropped_once_the_task_has_started() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    b.hold_ended();
    b.pump().await;
    b.svc.store.set_task_status(b.task.id, TaskStatus::InProgress, Actor::Manager).unwrap();
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.pump().await;
    assert!(b.holds().is_empty());
    assert!(!si.log().contains("PASTE"), "{}", si.log());
}

/// One that waited past the half hour is given up on, and says why.
#[tokio::test]
async fn a_hold_that_ended_is_given_up_on_after_half_an_hour() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Working).await;
    b.hold_ended();
    b.pump().await;
    farcooler_store::testing::backdate_hold_wakes(&b.svc.store, GIVE_UP_AFTER_MS + 120_000);
    b.pump().await;
    assert!(b.holds().is_empty());
    assert!(b.settled().iter().any(|n| n == "Not delivered: the agent stayed busy."), "{:?}", b.settled());
    assert!(!si.log().contains("PASTE"));
}

/// An answer and a hold on one task are two different news: neither
/// replaces the other.
#[tokio::test]
async fn an_answer_does_not_replace_a_hold_that_ended() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.hold_ended();
    b.answer("Drill in");
    b.pump().await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    tokio::time::sleep(Duration::from_millis(TOLD_SPACING_MS as u64 + 100)).await;
    b.pump().await;
    si.submits(1).await;
    let said = si.submitted();
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said.contains(&b.hold_told()) && said.contains(&b.told("Drill in")), "{said:?}");
}

/// The switch that lets this runner type into an agent covers a hold too.
#[tokio::test]
async fn with_the_switch_off_a_hold_that_ended_is_only_a_note() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ws = b.svc.store.get_workspace(b.task.workspace_id).unwrap();
    b.svc.store.set_workspace_wake_on_answer(ws.id, ws.resource_version, false).unwrap();
    b.hold_ended();
    b.pump().await;
    assert!(b.holds().is_empty());
    assert!(!si.log().contains("PASTE"));
}

//! The per-terminal typing lock (`Watcher::typing`) as the answer wake and
//! the held-draft pass use it (ov-394). On the board and tmux server of
//! `tests`, whose helpers these use.

use farcooler_protocol::v1::DraftHoldState;

use super::*;

/// An answer is never pasted into a box someone else holds (a composed
/// message, a draft or another send, between its gate and its Enter): it
/// lands once the box is free. Without the wake's own lock (answer_wake.rs,
/// `wake`), the answer pastes while the box is held, and this goes red.
#[tokio::test]
async fn an_answer_waits_for_a_box_someone_else_holds() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let held = b.watcher.typing(agent.id).await;
    // `answer` starts a pass of its own; give it every chance to type.
    b.answer("Drill in");
    tokio::time::sleep(Duration::from_millis(800)).await;
    nothing_typed(&si);
    drop(held);
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// A compose that holds one pane's box for a long time (here, for ever)
/// stalls no other pane: the answer pass skips that pane and returns, and
/// the drafts' pass pastes another pane's draft. The pumps `try_lock` the
/// box rather than wait for it while they hold `wake_pump` and
/// `draft_pump`; awaiting it there hangs both passes and this goes red.
#[tokio::test]
async fn a_held_box_stalls_no_other_panes_answer_or_draft() {
    let b = board().await;
    let (orchestrator, drafting, _) = super::draft_hold_tests::held(&b).await;
    drafting.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    let agent = b.agent("Agent 2", "claude").await;
    let answered = b.stand_in(&agent, "claude", "claude").await;
    // Starting the second stand-in sampled every pane, the menu included.
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let composing = b.watcher.typing(agent.id).await;
    b.answer("Drill in");
    b.watcher.wakes_hint.store(true, Ordering::SeqCst);
    // A stalled pass never comes back and a free one does, however slow the
    // runner, so the bound is far past any honest pass.
    let passes = tokio::time::timeout(Duration::from_secs(60), async {
        b.watcher.pump_wakes().await;
        b.watcher.pump_draft_holds().await;
    })
    .await;
    assert!(passes.is_ok(), "a pass waited on the held box");
    nothing_typed(&answered);
    drafting.pasted().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Sent as i32, "{}", drafting.log());
    // The answer was only put off: it lands once the box is free.
    drop(composing);
    b.pump().await;
    answered.submits(1).await;
    assert_eq!(answered.submitted(), [b.told("Drill in")], "{}", answered.log());
}

/// A draft prompt waiting for a busy pane's box holds neither pump's lock:
/// another pane's held draft still lands, and its own prompt still answers.
/// (`draft_into` takes the box, then `draft_pump`; the other way round, the
/// wait would hold `draft_pump` and skip every pass.)
#[tokio::test]
async fn a_draft_waiting_for_a_box_stalls_no_other_pane() {
    let b = board().await;
    let (orchestrator, drafting, _) = super::draft_hold_tests::held(&b).await;
    drafting.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    let agent = b.agent("Agent 2", "claude").await;
    let _busy = b.stand_in(&agent, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let composing = b.watcher.typing(agent.id).await;
    let waiting = tokio::spawn({
        let watcher = b.watcher.clone();
        async move { watcher.draft_into(agent.id, "About ov-2: ", false).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "the draft pasted past the held box");
    let pass = tokio::time::timeout(Duration::from_secs(60), b.watcher.pump_draft_holds()).await;
    assert!(pass.is_ok(), "a pass waited behind the draft prompt");
    drafting.pasted().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Sent as i32, "{}", drafting.log());
    drop(composing);
    waiting.await.unwrap().expect("pasted once the box was free");
}

/// A held draft whose own pane's box is taken is put off to the next pass,
/// not waited for: the pass returns with the draft still waiting, then pastes
/// it once the box is free.
#[tokio::test]
async fn a_held_draft_is_skipped_while_its_own_box_is_taken() {
    let b = board().await;
    let (orchestrator, si, _) = super::draft_hold_tests::held(&b).await;
    si.show("idle").await;
    b.screen_with(orchestrator.id, "? for shortcuts").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let composing = b.watcher.typing(orchestrator.id).await;
    let pass = tokio::time::timeout(Duration::from_secs(60), b.watcher.pump_draft_holds()).await;
    assert!(pass.is_ok(), "the pass waited on the held box");
    nothing_typed(&si);
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Waiting as i32);
    drop(composing);
    b.watcher.pump_draft_holds().await;
    si.pasted().await;
    assert_eq!(b.watcher.draft_hold(orchestrator.id).unwrap().state, DraftHoldState::Sent as i32, "{}", si.log());
}

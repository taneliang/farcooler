//! The per-terminal typing lock (`Watcher::typing`) as the answer wake and
//! the held-draft pass use it (ov-394). On the board and tmux server of
//! `tests`, whose helpers these use.

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

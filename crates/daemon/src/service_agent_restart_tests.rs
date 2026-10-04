//! A chat whose agent stopped, restarted where it is (ov-174): the Restart
//! the three apps offer is `set_pane_mode(Agent)` on a pane already in agent
//! mode, against a real tmux.

use super::agent_mode_wiring_tests::a_pane_that_looks_like_claude;
use super::restart_wiring_tests::{a_worktree, pane_start_command};
use super::*;

#[tokio::test]
async fn a_chat_whose_agent_stopped_restarts_as_a_chat() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
    svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a claude pane opens as a chat");
    svc.agents.stopped_for_test(term.id);
    let epoch = svc.store.get_terminal(term.id).unwrap().epoch;

    // The pane now runs the shim, which names no agent by process or by
    // screen: asked of the pane, this was "nothing in this pane is an agent".
    let restarted = svc
        .set_pane_mode(term.id, models::PaneMode::Agent, false)
        .await
        .expect("a chat whose agent stopped can be restarted as a chat");

    assert_eq!(restarted.pane_mode, models::PaneMode::Agent);
    assert!(restarted.epoch > epoch, "a new shim is a new stream");
    let command = pane_start_command(&svc, term.id).await;
    assert!(command.contains("agent-host"), "the pane runs a new shim: {command}");
    assert!(command.contains("--preset 'claude'"), "for the agent it had: {command}");
    assert_eq!(svc.agents.failure(term.id), None, "the stopped agent's word goes with it");
}

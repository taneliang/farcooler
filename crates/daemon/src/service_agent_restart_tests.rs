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

/// A healthy chat asked into agent mode again is left alone, and the call
/// succeeds (review finding 1): several clients ask from a view that may be
/// stale, and nothing about a working agent is a reason to kill it.
#[tokio::test]
async fn a_healthy_chat_asked_for_agent_mode_again_is_not_restarted() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
    svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a claude pane opens as a chat");
    let _shim = svc.agents.connected_for_test(term.id);
    let before = svc.store.get_terminal(term.id).unwrap().epoch;
    let command = pane_start_command(&svc, term.id).await;

    let same = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a no-op, not a refusal");

    assert_eq!(same.epoch, before, "no new shim");
    assert_eq!(pane_start_command(&svc, term.id).await, command);
    assert!(svc.agents.deliver(term.id, prompt("still there")).is_ok(), "the working agent is still connected");
}

/// Restart pressed twice restarts once, and the prompt it kept arrives once.
#[tokio::test]
async fn a_double_restart_restarts_once_and_sends_once() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
    svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a claude pane opens as a chat");
    svc.agents.queued_for_test(term.id, "kept");
    svc.agents.stopped_for_test(term.id);

    let first = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("restarted");
    let mut shim = svc.agents.connected_for_test(term.id);
    let second = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a no-op");

    assert_eq!(second.epoch, first.epoch, "the second tap found a healthy chat");
    let sent: Vec<_> = std::iter::from_fn(|| shim.try_recv().ok())
        .filter_map(|m| match m {
            farcooler_agent::link::DaemonMessage::Prompt { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["kept"]);
    assert!(svc.agents.deliver(term.id, prompt("next")).is_ok(), "and the new agent is still connected");
}

/// A record that names no agent is refused, not started with `--preset ''`
/// (review finding 5).
#[tokio::test]
async fn a_stopped_chat_with_no_recorded_agent_is_refused() {
    let (_dir, svc, ws) = a_worktree().await;
    let term = a_pane_that_looks_like_claude(&svc, &ws, "agent").await;
    let chat = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await.expect("a claude pane opens as a chat");
    svc.store
        .update_terminal(chat.id, chat.resource_version, terminal_update(&chat, |u| u.command_preset = String::new()))
        .unwrap();
    svc.agents.stopped_for_test(term.id);
    let epoch = svc.store.get_terminal(term.id).unwrap().epoch;

    let refused = svc.set_pane_mode(term.id, models::PaneMode::Agent, false).await;

    // Said as what it is: no agent named, not an agent without an adapter.
    assert!(
        matches!(refused, Err(DomainError::InvalidArgument { what: "nothing in this pane is an agent" })),
        "{refused:?}"
    );
    assert_eq!(svc.store.get_terminal(term.id).unwrap().epoch, epoch, "nothing was started");
}

fn prompt(text: &str) -> farcooler_agent::link::DaemonMessage {
    farcooler_agent::link::DaemonMessage::Prompt { text: text.into(), images: Vec::new() }
}

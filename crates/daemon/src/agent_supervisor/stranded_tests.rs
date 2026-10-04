//! A pane whose agent stopped, as the supervisor answers for it (ov-174). Its
//! own file to keep `agent_supervisor.rs` inside its size budget.

use super::*;
use farcooler_agent::event::QueuedPrompt;
use farcooler_core::error::DomainError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// A shim's end of the daemon link, as `serve` registers it.
fn a_shim(supervisor: &AgentSupervisor, terminal: Uuid) -> UnboundedReceiver<DaemonMessage> {
    let (tx, rx) = unbounded_channel();
    supervisor.writers.lock().unwrap().insert(terminal, tx);
    rx
}

fn established(supervisor: &AgentSupervisor, terminal: Uuid) {
    let hello = ShimMessage::Established { session_id: "s".into(), available_modes: Vec::new() };
    supervisor.apply(terminal, hello, &|_, _| {});
}

fn stopped(supervisor: &AgentSupervisor, terminal: Uuid) {
    supervisor.apply(terminal, ShimMessage::Failed { failure: AgentFailure::AdapterFailed }, &|_, _| {});
}

fn queued(supervisor: &AgentSupervisor, terminal: Uuid, texts: &[&str]) {
    let items = texts
        .iter()
        .map(|t| QueuedPrompt { id: format!("q-{t}"), text: t.to_string(), images: Vec::new() })
        .collect();
    supervisor.record(terminal, vec![AgentEvent::PromptQueue { items }], &|_, _| {});
}

fn prompt(text: &str) -> DaemonMessage {
    DaemonMessage::Prompt { text: text.into(), images: Vec::new() }
}

/// The prompts a shim was sent, in order.
fn prompts(rx: &mut UnboundedReceiver<DaemonMessage>) -> Vec<String> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|m| match m {
            DaemonMessage::Prompt { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn texts(supervisor: &AgentSupervisor, terminal: Uuid) -> Vec<String> {
    supervisor.stranded(terminal).into_iter().map(|q| q.text).collect()
}

#[test]
fn a_prompt_to_a_stopped_agent_is_refused_as_stopped() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    // The shim keeps its link open after its adapter dies, so a writer is
    // there: this is the refusal that used to read as "still connecting".
    let _shim = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    stopped(&supervisor, terminal);
    assert!(matches!(supervisor.deliver(terminal, prompt("hi")), Err(DomainError::AgentStopped)));
    for message in [DaemonMessage::Cancel, DaemonMessage::SteerQueued { id: "q".into() }] {
        assert!(matches!(supervisor.deliver(terminal, message), Err(DomainError::AgentStopped)));
    }
}

#[test]
fn a_pane_with_no_shim_yet_is_still_only_connecting() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    assert!(matches!(supervisor.deliver(terminal, prompt("hi")), Err(DomainError::AgentNotConnected)));
}

/// The review's item 5: `send` comes back once a restarted pane's new shim
/// establishes. Nothing pinned it, so a change to the `Established` arm could
/// have left a restarted pane refusing every prompt.
#[test]
fn sending_works_again_after_a_restart() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let _old = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    stopped(&supervisor, terminal);
    assert!(supervisor.deliver(terminal, prompt("before")).is_err());

    supervisor.restarting(terminal);
    // Between the respawn and the new shim dialing: a wait, not a death.
    assert!(matches!(supervisor.deliver(terminal, prompt("early")), Err(DomainError::AgentNotConnected)));

    let mut new = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    assert!(supervisor.deliver(terminal, prompt("after")).is_ok());
    assert_eq!(prompts(&mut new), ["after"]);
}

/// The gate's other half on its own: whatever cleared the failure before
/// it, a new shim's `Established` lets prompts through.
#[test]
fn a_fresh_established_lets_prompts_through_again() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let mut shim = a_shim(&supervisor, terminal);
    stopped(&supervisor, terminal);
    assert!(supervisor.deliver(terminal, prompt("before")).is_err());
    established(&supervisor, terminal);
    assert!(supervisor.deliver(terminal, prompt("after")).is_ok());
    assert_eq!(prompts(&mut shim), ["after"]);
}

#[test]
fn a_stopped_panes_queued_prompt_can_be_removed() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let mut shim = a_shim(&supervisor, terminal);
    queued(&supervisor, terminal, &["first", "second"]);
    stopped(&supervisor, terminal);

    supervisor.deliver(terminal, DaemonMessage::CancelQueued { id: "q-first".into() }).expect("removed");
    assert_eq!(texts(&supervisor, terminal), ["second"]);
    // Said to every reader, the way the shim would have said it.
    let (_, events) = supervisor.replay(terminal, 0, 0);
    let last = events.last().map(|s| &s.event);
    assert!(matches!(last, Some(AgentEvent::PromptQueue { items }) if items.len() == 1), "{last:?}");
    // And never handed to the dead shim.
    assert!(shim.try_recv().is_err());

    supervisor
        .deliver(terminal, DaemonMessage::EditQueued { id: "q-second".into(), text: "fixed".into() })
        .expect("edited");
    assert_eq!(texts(&supervisor, terminal), ["fixed"]);
    assert!(matches!(
        supervisor.deliver(terminal, DaemonMessage::CancelQueued { id: "q-gone".into() }),
        Err(DomainError::NotFound)
    ));
}

#[test]
fn a_restart_sends_what_was_queued_to_the_new_shim_once() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    let _old = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    queued(&supervisor, terminal, &["one", "two", "three"]);
    stopped(&supervisor, terminal);
    supervisor.deliver(terminal, DaemonMessage::CancelQueued { id: "q-two".into() }).expect("removed");

    supervisor.restarting(terminal);
    let mut new = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    assert_eq!(prompts(&mut new), ["one", "three"], "what was left, in order");

    // A daemon link that drops and comes back establishes again.
    established(&supervisor, terminal);
    assert!(prompts(&mut new).is_empty(), "sent once");
}

#[test]
fn a_switch_to_the_terminal_after_a_restart_keeps_nothing_to_send() {
    let supervisor = AgentSupervisor::new();
    let terminal = Uuid::now_v7();
    queued(&supervisor, terminal, &["one"]);
    stopped(&supervisor, terminal);
    supervisor.restarting(terminal);
    supervisor.left_agent_mode(terminal);
    let mut new = a_shim(&supervisor, terminal);
    established(&supervisor, terminal);
    assert!(prompts(&mut new).is_empty());
}

//! cursor's box, read as the gate and the read-back read it (ov-632).

use super::*;

/// cursor-agent 2026.10.16 in an orchestrator after a few turns, a message
/// pasted into its box and not yet sent: the banner has scrolled away, so
/// the screen carries none of the words the registry knows cursor by.
const SCROLLED_DRAFT: &str = include_str!("../../../../core/captures/cursor-2026.10.16-draft-banner-scrolled-120x40-e.txt");

/// A pasted message in a cursor box whose banner has scrolled away is a
/// draft in a familiar pane, not an unfamiliar one. Asked as the preset
/// `cursor`, the registry knows no agent here and answers nothing, so every
/// message pasted into such a pane read back as `paste_left` and was never
/// sent (found live, ov-632).
#[tokio::test]
async fn a_pasted_message_in_a_scrolled_cursor_box_is_read_as_a_draft() {
    let b = board().await;
    assert!(!SCROLLED_DRAFT.contains("Cursor Agent"), "the banner is gone from this screen");
    let registry = b.svc.registry();
    assert_eq!(registry.classify_main("cursor", SCROLLED_DRAFT), AgentActivity::None, "asked by preset name, it is nobody");
    let said = "[from the terminal] Reply with just the word LIME and use no tools.";
    let (held, turn, _) = b.watcher.box_on("cursor", SCROLLED_DRAFT).expect("a familiar pane");
    assert_eq!((held.clone(), turn), (Composer::Holds(said.into()), Turn::Between));
    assert!(composer::drawn::Expected::typed_into("cursor", said).shown_by(&held));
}

/// Another agent's preset is the registry's name for it, as before.
#[test]
fn only_cursors_preset_differs_from_its_registry_name() {
    assert_eq!(registry_command("cursor"), "cursor-agent");
    assert_eq!(registry_command("claude"), "claude");
    assert_eq!(registry_command("codex"), "codex");
}

/// A cursor pane, idle, in a stand-in.
async fn cursor_box(b: &Board) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "cursor").await;
    let si = b.stand_in(&agent, "cursor", "cursor-agent").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.watcher.with_timing(|t| t.compose_confirm = Duration::from_millis(800));
    (agent, si)
}

/// `text` pasted into `agent`'s box, no Enter, as the wake pastes it; the
/// ring's end from before the paste.
async fn paste(b: &Board, agent: &Terminal, si: &StandIn, text: &str) -> Option<farcooler_agent_core::event::Seq> {
    let ring = b.watcher.ring_end(agent.id);
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    runtime.paste_text(agent.id, "cursor", text).await.unwrap();
    si.pasted().await;
    ring
}

fn said(text: &str) -> Vec<farcooler_agent_core::event::AgentEvent> {
    vec![farcooler_agent_core::event::AgentEvent::Message {
        role: farcooler_agent_core::event::Role::User,
        text: text.into(),
        parent: None,
    }]
}

/// An Enter that didn't send leaves the text in the box, and nothing in the
/// ring says cursor took it: not confirmed, so not reported as sent.
#[tokio::test]
async fn text_left_in_the_box_is_not_taken() {
    let b = board().await;
    let (agent, si) = cursor_box(&b).await;
    let ring = paste(&b, &agent, &si, "Run the gates").await;
    assert!(!b.watcher.cursor_took(&agent, "cursor", 0, ring, "Run the gates").await);
}

/// The box emptied of the text is the Enter having sent it.
#[tokio::test]
async fn the_text_leaving_the_box_is_taken() {
    let b = board().await;
    let (agent, si) = cursor_box(&b).await;
    let ring = paste(&b, &agent, &si, "Run the gates").await;
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    runtime.send_bytes_hex(agent.id, "0d").await.unwrap();
    assert!(b.watcher.cursor_took(&agent, "cursor", 0, ring, "Run the gates").await);
}

/// `beforeSubmitPrompt` reaching the ring after the paste, with the text
/// (whitespace aside), is cursor taking it, box or no box.
#[tokio::test]
async fn a_prompt_hook_after_the_paste_is_cursor_taking_it() {
    let b = board().await;
    let (agent, si) = cursor_box(&b).await;
    let ring = paste(&b, &agent, &si, "Run the gates").await;
    b.svc.agents().record(agent.id, said("Run  the gates\n"), &|_, _| {});
    assert!(b.watcher.cursor_took(&agent, "cursor", 0, ring, "Run the gates").await);
}

/// The same words from before the paste are an earlier message, and other
/// words are another one.
#[tokio::test]
async fn an_earlier_or_another_prompt_is_not_it() {
    let b = board().await;
    let (agent, si) = cursor_box(&b).await;
    b.svc.agents().record(agent.id, said("Run the gates"), &|_, _| {});
    let ring = paste(&b, &agent, &si, "Run the gates").await;
    assert!(!b.watcher.cursor_took(&agent, "cursor", 0, ring, "Run the gates").await, "from before the paste");
    b.svc.agents().record(agent.id, said("Something else"), &|_, _| {});
    assert!(!b.watcher.cursor_took(&agent, "cursor", 0, ring, "Run the gates").await, "other words");
}

/// Someone typing in Far Cooler since the paste may have cleared the box
/// themselves: an empty box is then not the Enter having sent it (review 1, M1).
#[tokio::test]
async fn a_box_emptied_after_someone_typed_is_not_taken() {
    let b = board().await;
    let (agent, si) = cursor_box(&b).await;
    let ring = paste(&b, &agent, &si, "Run the gates").await;
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    runtime.send_bytes_hex(agent.id, "0d").await.unwrap();
    crate::runtime::mark_input(b.svc.root_dir(), agent.id);
    let pasted = crate::watch::now_millis() - 60_000;
    assert!(!b.watcher.cursor_took(&agent, "cursor", pasted, ring, "Run the gates").await);
}

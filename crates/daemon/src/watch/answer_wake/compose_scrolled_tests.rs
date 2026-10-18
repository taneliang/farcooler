//! A message taller than claude's box (ov-618): claude grows its box to half
//! the pane less five rows, then shows only the last rows of what was
//! pasted, and the read-back takes that tail for the text
//! (`Expected::shown_on`), in a compose, a tell and an answer alike; and a
//! draft typed just before each one's paste, which a tall paste would hide,
//! is caught first. The stand-in draws Bring Here's box
//! (`STAND_IN_DRAFT`), which wraps and windows as claude does, in a pane
//! 40x24: seven rows of 36 columns. On the board and tmux server of `tests`.

use std::sync::atomic::{AtomicBool, Ordering};

use super::compose_empty_box_tests::draw_a_draft;
use super::compose_tests::hook_on_submit;
use super::tell_tests::refused_with;
use super::*;

/// One line of 719 characters: short of claude's 800, so drawn as typed,
/// and twenty rows at 36 columns.
fn tall() -> String {
    (0..80).map(|i| format!("w{i:02} tall")).collect::<Vec<_>>().join(" ")
}

/// `terminal`'s pane made 40x24, and a claude stand-in drawing Bring Here's
/// box in it, `env` added.
async fn short_pane(b: &Board, terminal: &Terminal, env: &str) -> StandIn {
    b.svc.resize_terminal(terminal.id, 40, 24).await.expect("resized");
    for _ in 0..750 {
        if b.svc.screen(terminal.id).await.is_ok_and(|(_, columns, rows)| (columns, rows) == (40, 24)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let si = b.stand_in_with(terminal, "claude", "claude", &format!("STAND_IN_DRAFT=1 {env}")).await;
    b.doing(terminal.id, AgentActivity::Idle).await;
    b.svc.hooks().asks().heard("stand-in", false);
    b.svc.hooks().asks().turn_bounded("stand-in");
    si
}

/// Between turns: the box shows the message's last seven rows, and it's
/// Sent, whole. Before ov-618, `paste_left`.
#[tokio::test]
async fn a_message_taller_than_the_box_is_sent() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    let _hook = hook_on_submit(&b, &si);
    let text = tall();
    assert_eq!(b.watcher.compose_into(agent.id, &text, &[]).await.expect("sent"), Turn::Between);
    si.submits(1).await;
    assert_eq!(si.submitted(), [text], "{}", si.log());
}

/// Mid-turn, the box read back by its tail before the Enter and again
/// under the fence (`enter_fenced`): Queued.
#[tokio::test]
async fn a_message_taller_than_the_box_is_queued_mid_turn() {
    let b = board().await;
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    si.show("working").await;
    b.screen_with(agent.id, "Pondering").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let text = tall();
    assert_eq!(b.watcher.compose_into(agent.id, &text, &[]).await.expect("queued"), Turn::During);
    assert!(si.log().contains(&format!("QUEUED {text}")), "{}", si.log());
}

/// `terminal tell`'s one line, 400 characters: eleven rows, its tail read
/// back. Before ov-618, `paste_left`.
#[tokio::test]
async fn a_tell_taller_than_the_box_is_submitted() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = short_pane(&b, &orchestrator, "").await;
    let text: String = tall().chars().take(400).collect::<String>().trim_end().to_string();
    assert_eq!(b.watcher.tell_into(orchestrator.id, &text).await.expect("told"), Turn::Between);
    si.submits(1).await;
    assert_eq!(si.submitted(), [text], "{}", si.log());
}

/// A box that shows the tail but isn't as tall as claude grows it, here
/// six rows (`STAND_IN_WINDOW`): what's above it is missing for no reason
/// claude gives, so the text is left in the box, never sent.
#[tokio::test]
async fn a_box_short_of_claudes_tallest_is_left() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "STAND_IN_WINDOW=6").await;
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(refused_with(b.watcher.compose_into(agent.id, &tall(), &[]).await), "paste_left");
    assert!(si.submitted().is_empty(), "{}", si.log());
}

/// What the screen can't show once the box scrolls, a draft above the
/// paste, is kept out before the paste: a draft typed straight into the
/// terminal after the gate read the box empty (`before_paste`, no input
/// mark) is caught by the box read last (`box_still_empty`), and nothing is
/// typed. Without that read the tall paste would hide the draft and go.
#[tokio::test]
async fn a_draft_drawn_before_a_tall_compose_is_refused() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, agent.id, "my own draft", ran.clone()));
    assert_eq!(refused_with(b.watcher.compose_into(agent.id, &tall(), &[]).await), "draft");
    assert!(ran.load(Ordering::SeqCst));
    assert!(!si.log().contains("PASTE "), "{}", si.log());
}

/// The same for `terminal tell`.
#[tokio::test]
async fn a_draft_drawn_before_a_tall_tell_is_refused() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = short_pane(&b, &orchestrator, "").await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, orchestrator.id, "my own draft", ran.clone()));
    let text: String = tall().chars().take(400).collect::<String>().trim_end().to_string();
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, &text).await), "draft");
    assert!(ran.load(Ordering::SeqCst));
    assert!(!si.log().contains("PASTE "), "{}", si.log());
}

/// A decision's answer cut to its longest, about 300 characters told: nine
/// rows at 36 columns.
fn tall_answer() -> String {
    "word ".repeat(80)
}

/// An answer taller than the box, between turns: read back by its tail
/// (`type_into`) and told. Before ov-618, a paste left in the box.
#[tokio::test]
async fn an_answer_taller_than_the_box_is_told() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    b.answer(&tall_answer());
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told(&tall_answer())], "{}", si.log());
    assert_eq!(b.progress(), ["Told Agent 2 about the decision"]);
}

/// An answer a tool call stopped short of its Enter, mid-turn: its first
/// pass reads the tall box back (`type_into`) and waits; once the call has
/// ended, the next reads it again (`finish_paste`) and presses only the
/// Enter. Before ov-618, either read left the paste in the box.
#[tokio::test]
async fn a_tall_answer_a_dialog_stopped_is_finished() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    si.show("working").await;
    b.screen_with(agent.id, "Pondering").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let asks = b.svc.hooks().asks().clone();
    let log = si.log.clone();
    let gate = tokio::spawn(async move {
        for _ in 0..1_000 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("PASTE ") {
                asks.mark_tool_starting("stand-in", Some("toolu_dialog"));
                return true;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        false
    });
    b.answer(&tall_answer());
    b.pump().await;
    assert!(gate.await.unwrap(), "the paste never reached the stand-in");
    assert!(b.settled().is_empty(), "{:?}", b.settled());
    assert!(b.pending()[0].pasted_at.is_some(), "not marked pasted: {:?}", b.pending());
    b.svc.hooks().asks().tool_ended("stand-in", Some("toolu_dialog"));
    b.pump().await;
    assert!(si.log().contains(&format!("QUEUED {}", b.told(&tall_answer()))), "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "pasted again: {}", si.log());
    assert!(b.pending().is_empty(), "{:?}", b.pending());
}

/// An answer, a draft typed straight into the terminal just before its
/// paste: the box read last finds it, and the answer waits untyped.
#[tokio::test]
async fn a_draft_drawn_before_a_tall_answer_holds_it() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = short_pane(&b, &agent, "").await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, agent.id, "my own draft", ran.clone()));
    b.answer(&tall_answer());
    b.pump().await;
    assert!(ran.load(Ordering::SeqCst));
    b.untouched(&si);
}

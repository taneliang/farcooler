//! `terminal tell` (ov-214, ov-360): a message typed into a terminal
//! orchestrator and submitted, and every refusal that leaves it untyped. A
//! working one is told too: `mid_turn_tests`. On the board and tmux server
//! of `tests`, whose helpers these use.

use super::*;

/// The refusal's stable word, from a `tell_into` error.
pub(super) fn refused_with(result: Result<Turn>) -> &'static str {
    match result {
        Err(e) => e.what(),
        Ok(turn) => panic!("it was typed ({turn:?})"),
    }
}

/// An idle claude run by hand in the adopted orchestrator's pane: the
/// message is pasted on one line, read back, and submitted, once, as its
/// next prompt.
#[tokio::test]
async fn a_message_is_typed_into_an_idle_orchestrator_and_submitted() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let turn = b.watcher.tell_into(orchestrator.id, "-x land ov-214\nafter the rebase").await.expect("told");
    assert_eq!(turn, Turn::Between);
    si.pasted().await;
    assert!(si.log().contains("PASTE "), "a bracketed paste: {}", si.log());
    si.submits(1).await;
    assert_eq!(si.submitted(), ["-x land ov-214 after the rebase"], "{}", si.log());
}

/// Two messages a moment apart: the second waits out the spacing an answer
/// would, rather than be refused for it.
#[tokio::test]
async fn a_second_message_right_after_the_first_is_typed_too() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.tell_into(orchestrator.id, "first").await.expect("told");
    si.submits(1).await;
    b.watcher.tell_into(orchestrator.id, "second").await.expect("told again");
    si.submits(2).await;
    assert_eq!(si.submitted(), ["first", "second"], "{}", si.log());
}

/// Every check that holds an answer, but working, refuses a message, typing
/// nothing, and names why.
#[tokio::test]
async fn a_message_is_refused_where_an_answer_would_wait() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    si.show("menu").await;
    b.screen_with(orchestrator.id, "Tab to amend").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "prompt");
    nothing_typed(&si);
    si.show("picker").await;
    b.screen_with(orchestrator.id, "Select model").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "unfamiliar");
    nothing_typed(&si);
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "draft");
    nothing_typed(&si);
    si.show("idle").await;
    b.doing(orchestrator.id, AgentActivity::Blocked).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "prompt");
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    crate::runtime::mark_input(b.svc.root_dir(), orchestrator.id);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "typing");
    nothing_typed(&si);
}

/// A process that isn't an agent, drawing a perfect agent screen: refused.
#[tokio::test]
async fn a_message_is_refused_for_a_process_that_is_not_an_agent() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "perl").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "not_an_agent");
    nothing_typed(&si);
}

/// Too long is refused before anything is typed, never cut; empty too; one
/// starting with what a box reads as a command too; and only an
/// orchestrator in a terminal is typed to.
#[tokio::test]
async fn a_message_too_long_empty_a_command_or_to_the_wrong_pane_is_refused() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let long = "a".repeat(super::tell::LONGEST_MESSAGE + 1);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, &long).await), "too_long");
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, " \n\t ").await), "text");
    for command in ["/cost", "!rm -rf build", " \n!ls", "#remember this", "@src/main.rs", "?", "\\"] {
        assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, command).await), "command", "{command:?}");
    }
    nothing_typed(&si);
    let agent = b.agent("Agent 2", "claude").await;
    let other = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(agent.id, "hello").await), "terminal");
    nothing_typed(&other);
}

/// What a message is typed as: one line, its slash or bang after the start
/// left alone.
#[test]
fn only_the_start_of_a_message_is_read_as_a_command() {
    assert_eq!(super::tell::told_text("land it, then /review").unwrap(), "land it, then /review");
    assert_eq!(super::tell::told_text("“/review” it").unwrap(), "“/review” it");
    assert_eq!(super::tell::told_text("-x land").unwrap(), "-x land");
}

/// A paste the box doesn't hold exactly gets no Enter, and says so.
#[tokio::test]
async fn a_message_the_box_doesnt_hold_is_left_and_not_sent() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    si.show("mangle").await;
    b.screen_with(orchestrator.id, "stand-in").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "paste_left");
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// The words a refusal can name are the ones the CLI has a line for.
#[test]
fn a_held_answer_names_a_word_the_cli_knows() {
    for held in [Held::Busy, Held::Prompt, Held::Draft, Held::Typing, Held::NotAnAgent, Held::Unfamiliar, Held::Unproven] {
        assert!(
            ["busy", "prompt", "draft", "typing", "not_an_agent", "unfamiliar", "unproven"]
                .contains(&super::tell::held_word(held))
        );
    }
}

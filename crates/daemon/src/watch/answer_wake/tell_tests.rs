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

/// What a tell into a claude that can't be typed to safely yet comes to
/// since it's a composed message (ov-620): answered Queued and held
/// (`compose::held`), nothing typed.
pub(super) fn held(b: &Board, id: Uuid, result: Result<Turn>) {
    assert_eq!(result.expect("held"), Turn::During);
    assert!(b.svc.store.holds_compose(id).unwrap(), "held");
}

/// An idle claude run by hand in the adopted orchestrator's pane: the
/// message is pasted, its line break kept as a composed message's is, read
/// back, and submitted, once, as its next prompt, Sent once claude's hook
/// names it.
#[tokio::test]
async fn a_message_is_typed_into_an_idle_orchestrator_and_submitted() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let turn = b.watcher.tell_into(orchestrator.id, "-x land ov-214\nafter the rebase", crate::watch::answer_wake::route::Author::Person).await.expect("told");
    assert_eq!(turn, Turn::Between);
    assert!(si.log().contains("PASTE "), "a bracketed paste: {}", si.log());
    assert_eq!(si.submitted(), ["-x land ov-214\\nafter the rebase"], "{}", si.log());
}

/// The native view's composer (`terminal.compose`, ov-372, ov-367): any
/// claude pane, not only the orchestrator, past the same gate, its line
/// breaks kept, while `tell` still refuses the worker.
#[tokio::test]
async fn a_composed_message_is_submitted_into_a_worker_claude_pane() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(agent.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "terminal");
    nothing_typed(&si);
    b.svc.hooks().asks().heard("stand-in", false);
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let turn = b.watcher.compose_into(agent.id, "fix the build\nthen the docs", &[]).await.expect("composed");
    assert_eq!(turn, Turn::Between);
    si.submits(1).await;
    assert_eq!(si.submitted(), ["fix the build\\nthen the docs"], "{}", si.log());
}

/// A composed message and a draft never type into one box at once
/// (ov-372): whichever holds the box (`Watcher::typing`, as an answer, a
/// held draft's pass or another send does) is through its Enter before the
/// other pastes. Here the box is held, and neither types until it's free.
#[tokio::test]
async fn a_composed_message_and_a_draft_wait_for_the_box() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.svc.hooks().asks().heard("stand-in", false);
    let _hook = super::compose_tests::hook_on_submit(&b, &si);
    let held = b.watcher.typing(agent.id).await;
    let watcher = std::sync::Arc::clone(&b.watcher);
    let composed = tokio::spawn(async move { watcher.compose_into(agent.id, "carry on", &[]).await });
    let watcher = std::sync::Arc::clone(&b.watcher);
    let drafted = tokio::spawn(async move { watcher.draft_into(agent.id, "About ov-1: ", false).await });
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    nothing_typed(&si);
    assert!(!composed.is_finished() && !drafted.is_finished(), "one typed past the held box");
    drop(held);
    // One goes first and the other finds its box taken (a draft, or a
    // message), never both pasted together.
    let (composed, drafted) = (composed.await.unwrap(), drafted.await.unwrap());
    match &composed {
        Ok(_) => si.submits(1).await,
        Err(e) => assert_eq!(e.what(), "draft", "the draft went first, and the message found it: {composed:?}"),
    }
    assert!(composed.is_ok() || drafted.is_ok(), "{composed:?} {drafted:?}");
    // What was submitted is the message alone, never a draft pasted in
    // after its read-back.
    for submitted in si.submitted() {
        assert_eq!(submitted, "carry on", "{}", si.log());
    }
}

/// Two messages a moment apart: the second waits out the spacing an answer
/// would, rather than be refused for it.
#[tokio::test]
async fn a_second_message_right_after_the_first_is_typed_too() {
    let b = board().await;
    // The spacing the second waits out, short: that it waits is what's tested.
    b.watcher.with_timing(|t| t.told_spacing = crate::watch::Timing::SHORT);
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    b.watcher.tell_into(orchestrator.id, "first", crate::watch::answer_wake::route::Author::Person).await.expect("told");
    si.submits(1).await;
    b.watcher.tell_into(orchestrator.id, "second", crate::watch::answer_wake::route::Author::Person).await.expect("told again");
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
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "prompt");
    nothing_typed(&si);
    si.show("picker").await;
    b.screen_with(orchestrator.id, "Select model").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "unfamiliar");
    nothing_typed(&si);
    si.show("draft:fix the flaky").await;
    b.screen_with(orchestrator.id, "fix the flaky").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "draft");
    nothing_typed(&si);
    si.show("idle").await;
    b.doing(orchestrator.id, AgentActivity::Blocked).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "prompt");
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    crate::runtime::mark_input(b.svc.root_dir(), orchestrator.id);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "typing");
    nothing_typed(&si);
}

/// A process that isn't an agent, drawing a perfect agent screen: refused.
#[tokio::test]
async fn a_message_is_refused_for_a_process_that_is_not_an_agent() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "perl").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "not_an_agent");
    nothing_typed(&si);
}

/// A tell is a composed message (ov-620), refused by compose's checks before
/// anything is typed: too long, never cut; empty; a shell escape or a
/// command that isn't one; one of claude's own commands, handed off to the
/// terminal; and only an orchestrator in a terminal is told.
#[tokio::test]
async fn a_message_too_long_empty_a_command_or_to_the_wrong_pane_is_refused() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let long = "a".repeat(super::compose::LONGEST_TEXT + 1);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, &long, crate::watch::answer_wake::route::Author::Person).await), "too_long");
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, " \n\t ", crate::watch::answer_wake::route::Author::Person).await), "text");
    for (command, word) in [("!rm -rf build", "command"), (" \n!ls", "command"), ("/~x", "command"), ("/cost", "handoff")] {
        assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, command, crate::watch::answer_wake::route::Author::Person).await), word, "{command:?}");
    }
    nothing_typed(&si);
    let agent = b.agent("Agent 2", "claude").await;
    let other = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused_with(b.watcher.tell_into(agent.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "terminal");
    nothing_typed(&other);
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
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello", crate::watch::answer_wake::route::Author::Person).await), "paste_left");
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

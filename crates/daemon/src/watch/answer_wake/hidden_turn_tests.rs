//! ov-392 (review of ov-367, finding 1): after a long paste is queued, claude
//! 2.1.290's footer says `paste again to expand` where `esc to interrupt`
//! was. The watcher read that screen idle, and `terminal tell` and the
//! answer wake pressed an unfenced Enter with a tool call in flight. Now the
//! screen reads Working by its spinner row (core), and claude's registry
//! saying busy makes it mid-turn whatever the screen says (`proven_tui`):
//! either way the Enter needs the fence, and a call in flight stops it. On
//! the board and tmux server of `tests`, whose helpers these use.

use super::tell_tests::refused_with;
use super::*;

const SESSION: &str = "stand-in";

/// A claude stand-in working as claude does after a queued long paste
/// (`mode`), its hooks heard from, a call in flight, the watcher reading it
/// idle as it did before the fix.
async fn working_with_a_call(b: &Board, terminal: &Terminal, si: &StandIn, mode: &str) {
    b.svc.hooks().asks().heard(SESSION, false);
    b.svc.hooks().asks().turn_bounded(SESSION);
    si.show(mode).await;
    b.screen_with(terminal.id, "paste again to expand").await;
    b.doing(terminal.id, AgentActivity::Idle).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
}

/// The reviewer's probe: `tell` into that screen with a call in flight.
#[tokio::test]
async fn a_tell_after_a_queued_long_paste_waits_for_the_call() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working_with_a_call(&b, &orchestrator, &si, "working-long").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello there").await), "busy");
    nothing_typed(&si);
}

/// The same for an answer: it waits, untyped.
#[tokio::test]
async fn an_answer_after_a_queued_long_paste_waits_for_the_call() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    working_with_a_call(&b, &agent, &si, "working-long").await;
    b.answer("Drill in");
    b.pump().await;
    b.untouched(&si);
}

/// The screen alone, no registry to say: its spinner row reads Working.
#[tokio::test]
async fn the_spinner_alone_makes_it_mid_turn() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    let sessions = b.dir.path().join(format!("si-{}", orchestrator.id.simple())).join("config").join("sessions");
    for registry in std::fs::read_dir(&sessions).unwrap() {
        std::fs::remove_file(registry.unwrap().path()).unwrap();
    }
    working_with_a_call(&b, &orchestrator, &si, "working-long").await;
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello there").await), "busy");
    nothing_typed(&si);
}

/// The registry alone, the screen showing no sign of the turn at all.
#[tokio::test]
async fn the_registry_alone_makes_it_mid_turn() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    working_with_a_call(&b, &orchestrator, &si, "working-hidden").await;
    assert_eq!(b.svc.registry().classify("claude", &b.svc.screen(orchestrator.id).await.unwrap().0), AgentActivity::Idle);
    assert_eq!(refused_with(b.watcher.tell_into(orchestrator.id, "hello there").await), "busy");
    nothing_typed(&si);
}

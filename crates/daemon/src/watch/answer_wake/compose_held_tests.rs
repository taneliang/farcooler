//! ov-468: a composed message claude can't take safely yet is held and
//! answered Queued, then typed when it can (`compose::held`); and a claude
//! whose turn has ended while background agents run takes one at once, as
//! its next turn. Through `compose_into` and the stand-in's real screen.

use super::compose_tests::{hook_on_submit, idle_claude};
use super::tell_tests::refused_with;
use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// Wait, up to 15 seconds, for the stand-in to log `line`.
async fn logs(si: &StandIn, line: &str) {
    for _ in 0..750 {
        if si.log().contains(line) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the stand-in never logged {line}: {}", si.log());
}

/// claude 2.1.296 with its turn over and two background agents running
/// (`captures/claude-2.1.296-background-agents-main-idle-120x40-e.txt`): the
/// fleet reads it working, its registry says busy, and a background agent
/// has a call in flight, all of which once held the send as mid-turn and
/// refused it as `busy`. Its box takes the message as a turn of its own, so
/// it goes in at once, Sent.
#[tokio::test]
async fn a_claude_waiting_on_background_agents_takes_a_message_as_its_next_turn() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("background").await;
    let screen = b.screen_with(agent.id, "↓ to manage").await;
    assert_eq!(b.svc.registry().classify("claude", &screen), AgentActivity::Working, "the fleet still says it");
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_call(SESSION, Some("toolu_bg"), Some("agent-bg"));
    let _hook = hook_on_submit(&b, &si);
    let turn = b.watcher.compose_into(agent.id, "what did they find", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between);
    assert_eq!(si.submitted(), ["what did they find"], "{}", si.log());
    assert!(!si.log().contains("QUEUED"), "{}", si.log());
}

/// Working with a call in flight, when an Enter could land on a dialog:
/// both sends are answered Queued at once with nothing typed, and once the
/// call ends each goes into claude's queue, in the order sent.
#[tokio::test]
async fn sends_claude_cant_take_yet_are_held_and_queued_in_order() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    assert_eq!(b.watcher.compose_into(agent.id, "first", &[]).await.expect("held"), Turn::During);
    assert_eq!(b.watcher.compose_into(agent.id, "second", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    nothing_typed(&si);
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    logs(&si, "QUEUED second").await;
    let log = si.log();
    assert!(log.find("QUEUED first").unwrap() < log.find("QUEUED second").unwrap(), "{log}");
    si.show("idle").await;
    si.submits(2).await;
    assert_eq!(si.submitted(), ["first", "second"], "{}", si.log());
}

/// A command mid-turn waits for the turn to end: held, then run.
#[tokio::test]
async fn a_command_mid_turn_is_held_until_the_turn_ends() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(b.watcher.compose_into(agent.id, "/init", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    nothing_typed(&si);
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    logs(&si, "COMMAND /init").await;
}

/// A held message the gate then refuses for good, a draft left in the box,
/// is dropped: never typed over the draft, and a later send isn't stuck
/// behind it.
#[tokio::test]
async fn a_held_message_the_gate_refuses_is_dropped() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    assert_eq!(b.watcher.compose_into(agent.id, "lost", &[]).await.expect("held"), Turn::During);
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    for _ in 0..750 {
        if !b.watcher.composed_held.lock().unwrap().contains_key(&agent.id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!b.watcher.composed_held.lock().unwrap().contains_key(&agent.id), "still held");
    nothing_typed(&si);
    assert_eq!(refused_with(b.watcher.compose_into(agent.id, "next", &[]).await), "draft");
}

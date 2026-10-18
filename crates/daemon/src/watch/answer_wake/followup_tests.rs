//! A message for a cursor pane is answered to its `stop` hook (ov-563),
//! driven as the hook binary drives it: a frame over the ingress's own
//! connection handler, and the line read back. No cursor runs.

use farcooler_agent_hooks::Agent;
use farcooler_agent_hooks::wire::{HookLine, TAKES_FOLLOWUP, encode_line};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use super::*;

const SESSION: &str = "68d64051-6ab2-4668-97a1-a45df658bada";

/// A cursor pane in the store that `SESSION` names.
async fn cursor_pane(b: &Board) -> Terminal {
    let pane = b.agent("Agent 2", "cursor").await;
    b.svc.store.set_pane_mode(pane.id, pane.resource_version, PaneMode::Terminal, Some(SESSION.into()), false).unwrap();
    pane
}

/// Send cursor's `stop` over a connection and return the line the daemon
/// writes back, or `None` when it wrote nothing.
async fn stop(b: &Board, takes: &[&str]) -> Option<String> {
    let payload = serde_json::json!({ "session_id": SESSION, "hook_event_name": "stop", "workspace_roots": [b.lane.worktree_path.clone()] });
    let line = HookLine { agent: Agent::Cursor, event: "stop".into(), payload, takes: takes.iter().map(|t| t.to_string()).collect() };
    let (mut hook, daemon) = tokio::net::UnixStream::pair().unwrap();
    let ingress = b.svc.hooks().clone();
    let served = tokio::spawn(async move { ingress.serve(daemon, &|_, _| {}).await });
    hook.write_all(encode_line(&line).unwrap().as_bytes()).await.unwrap();
    // The hook hangs up once it has written, as a hook does: the handler
    // then ends, and what it wrote is read to the end.
    hook.shutdown().await.unwrap();
    let mut reader = tokio::io::BufReader::new(&mut hook);
    let mut said = String::new();
    let n = reader.read_line(&mut said).await.unwrap();
    drop(reader);
    drop(hook);
    served.await.unwrap().unwrap();
    (n > 0).then(|| said.trim().to_string())
}

/// The turn's end takes the message, settles it, and the next end has
/// nothing to say.
#[tokio::test]
async fn a_cursor_turn_s_end_takes_the_message_waiting_for_it() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    b.send("manager", &b.task.key, "Rebase on main first").await.unwrap();
    assert_eq!(b.messages().len(), 1);
    let said = stop(&b, &[TAKES_FOLLOWUP]).await.expect("a line");
    let said: serde_json::Value = serde_json::from_str(&said).unwrap();
    assert_eq!(said["followup_message"], "[from the orchestrator] Rebase on main first", "{said}");
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
    assert_eq!(stop(&b, &[TAKES_FOLLOWUP]).await.as_deref(), Some("{}"), "nothing more waits for {}", pane.id);
}

/// A hook that never said it prints a followup is told nothing, and the
/// message stays for the pump.
#[tokio::test]
async fn a_hook_that_cannot_print_one_takes_none() {
    let b = board().await;
    cursor_pane(&b).await;
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    assert_eq!(stop(&b, &[]).await, None);
    assert_eq!(b.messages().len(), 1, "still waiting");
}

/// A message for another pane is not this one's.
#[tokio::test]
async fn a_message_for_another_pane_stays_put() {
    let b = board().await;
    cursor_pane(&b).await;
    let other = b.agent("Agent 3", "claude").await;
    b.svc.store.send_message(b.task.id, Actor::Manager, "Ship it", Some(other.id), serde_json::json!({})).unwrap();
    assert_eq!(stop(&b, &[TAKES_FOLLOWUP]).await.as_deref(), Some("{}"));
    assert_eq!(b.messages().len(), 1);
}

/// A hook that hung up before the daemon's line was written has not been
/// told anything: the message stays claimed, and is not settled as
/// delivered, nor typed as well. (The pump then records it as one it
/// couldn't confirm, as for any claimed wake.)
#[tokio::test]
async fn a_hook_that_hung_up_leaves_the_message_claimed() {
    let b = board().await;
    cursor_pane(&b).await;
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    // The pump `send` set off runs beside this test, and a pass that read the
    // row claimed would settle it as unconfirmed before it's looked at
    // (1 in 30 on main): no pass runs until the claim has been checked.
    let no_pump = b.watcher.wake_pump.lock().await;
    let payload = serde_json::json!({ "session_id": SESSION, "hook_event_name": "stop", "workspace_roots": [b.lane.worktree_path.clone()] });
    let line = HookLine { agent: Agent::Cursor, event: "stop".into(), payload, takes: vec![TAKES_FOLLOWUP.into()] };
    let (mut hook, daemon) = tokio::net::UnixStream::pair().unwrap();
    hook.write_all(encode_line(&line).unwrap().as_bytes()).await.unwrap();
    // Gone before the daemon has read a word of it (a current-thread runtime
    // runs the handler only once this test yields).
    drop(hook);
    // And gone as the kernel sees it: a write just after the close can still
    // land (1 in 30 on main, so the daemon's line went out and the message
    // settled as delivered). Wait until a write from the daemon's end fails;
    // what it wrote goes to nobody.
    let since = std::time::Instant::now();
    loop {
        daemon.writable().await.unwrap();
        match daemon.try_write(b"\n") {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
            Ok(_) => {
                assert!(since.elapsed() < Duration::from_secs(30), "the hook's end never closed");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    let ingress = b.svc.hooks().clone();
    let _ = ingress.serve(daemon, &|_, _| {}).await;
    let waiting = b.messages();
    assert_eq!(waiting.len(), 1, "not settled as delivered");
    assert!(waiting[0].claimed_at.is_some(), "claimed: {waiting:?}");
    drop(no_pump);
    b.pump().await;
    assert_eq!(b.settled(), ["Couldn't confirm the message reached its agent"], "recorded as unconfirmed, never delivered twice");
}

/// Idle, a cursor pane has no turn to end: the pump types the message.
#[tokio::test]
async fn an_idle_cursor_is_typed_to() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    let si = b.stand_in(&pane, "cursor", "cursor-agent").await;
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the orchestrator] Ship it"], "{}", si.log());
}

/// A working one is not typed into: the message waits for its turn's end,
/// which takes it, and the pump then has nothing left to type. (Fails if
/// the hook doesn't take it, as the message then types once the pane idles.)
#[tokio::test]
async fn a_working_cursor_waits_for_its_turn_to_end() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    let si = b.stand_in(&pane, "cursor", "cursor-agent").await;
    si.show("working").await;
    b.doing(pane.id, AgentActivity::Working).await;
    b.send("manager", &b.task.key, "Ship it").await.unwrap();
    b.pump().await;
    assert!(!si.log().contains("PASTE"), "typed mid-turn: {}", si.log());
    assert_eq!(b.messages().len(), 1);
    let said = stop(&b, &[TAKES_FOLLOWUP]).await.expect("a line");
    assert!(said.contains("Ship it"), "{said}");
    // The turn is over and the pane idle: nothing is typed, the hook had it.
    si.show("idle").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.pump().await;
    assert!(!si.log().contains("PASTE"), "typed as well: {}", si.log());
    assert!(b.messages().is_empty());
}

/// A person's message to a working cursor pane goes by its turn's followup,
/// the same role:user `<user_query>` as typing (ov-627): the pump never types
/// it, says on its card why it waits, and the turn's end takes it (round 5
/// typed it, and left it for typing at a turn's end).
#[tokio::test]
async fn a_persons_message_to_a_working_cursor_goes_by_the_followup() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    b.doing(pane.id, AgentActivity::Working).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    assert!(
        b.progress().iter().any(|n| n.ends_with("it's working, and Cursor takes it when this turn ends.")),
        "{:?}",
        b.progress()
    );
    let said: serde_json::Value = serde_json::from_str(&stop(&b, &[TAKES_FOLLOWUP]).await.expect("a line")).unwrap();
    assert_eq!(said["followup_message"], "[from the terminal] Rename foo to bar", "{said}");
    assert!(b.messages().is_empty(), "settled: {:?}", b.messages());
}

/// Past cursor's loop limit, with nothing typed in its pane, a person's
/// message waits and says why, never dropped.
#[tokio::test]
async fn past_the_followup_limit_a_persons_message_says_why_it_waits() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    b.doing(pane.id, AgentActivity::Working).await;
    for n in 0..super::queued::FOLLOWUP_LIMIT {
        b.send("manager", &b.task.key, &format!("Step {n}")).await.unwrap();
        stop(&b, &[TAKES_FOLLOWUP]).await.expect("a line");
    }
    assert!(b.watcher.followup_limited(pane.id));
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    assert_eq!(b.messages().len(), 1, "held, not dropped");
    assert!(b.progress().iter().any(|n| n.contains("stopped asking for more")), "{:?}", b.progress());
}

/// A person's message to an idle cursor is typed, as on main: the followup
/// fires only at a turn's end, which an idle cursor may never reach (the
/// ruling on ov-620 re-review 4). Round 6 held it for the followup.
#[tokio::test]
async fn a_persons_message_to_an_idle_cursor_is_typed() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    let si = b.stand_in(&pane, "cursor", "cursor-agent").await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Rename foo to bar"], "{}", si.log());
}

/// An answer to a cursor pane takes the typed path, as on main, never the
/// followup, which takes only messages: a working one waits as busy, then is
/// typed. Round 6 held it for a followup that never came (re-review 4, C1).
#[tokio::test]
async fn an_answer_to_a_cursor_is_typed_never_held_for_the_followup() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    let si = b.stand_in(&pane, "cursor", "cursor-agent").await;
    si.show("working").await;
    b.doing(pane.id, AgentActivity::Working).await;
    b.answer("Drill in");
    b.pump().await;
    assert_eq!(b.progress(), ["Waiting to tell Agent 2 about the decision: it's busy."]);
    si.show("idle").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [b.told("Drill in")], "{}", si.log());
}

/// A person's message held for a working cursor's followup is typed once the
/// pane reads idle, even though no stop hook came: the hold is decided again
/// on every pass, never remembered (re-review 5).
#[tokio::test]
async fn a_message_held_for_a_working_cursor_is_typed_once_it_idles_without_a_hook() {
    let b = board().await;
    let pane = cursor_pane(&b).await;
    let si = b.stand_in(&pane, "cursor", "cursor-agent").await;
    si.show("working").await;
    b.doing(pane.id, AgentActivity::Working).await;
    b.send("user", &b.task.key, "Rename foo to bar").await.unwrap();
    b.pump().await;
    assert!(!si.log().contains("PASTE"), "typed mid-turn: {}", si.log());
    assert_eq!(b.messages().len(), 1, "held for the followup");
    si.show("idle").await;
    b.doing(pane.id, AgentActivity::Idle).await;
    b.pump().await;
    si.submits(1).await;
    assert_eq!(si.submitted(), ["[from the terminal] Rename foo to bar"], "{}", si.log());
}

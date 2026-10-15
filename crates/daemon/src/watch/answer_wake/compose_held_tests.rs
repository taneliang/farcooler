//! ov-468: a composed message claude can't take safely yet is held and
//! answered Queued, then typed when it can (`compose::held`); and a claude
//! whose turn has ended while background agents run takes one at once, as
//! its next turn. Through `compose_into` and the stand-in's real screen.

use farcooler_core::session_log::projector::{NoticeKind, QueuedState, Row, RowKind};

use super::compose_tests::{hook_on_submit, idle_claude};
use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// Held messages retried, and sends spaced, sooner than the daemon does:
/// each retry still runs the whole gate, so what's held stays held.
fn quick(b: &Board) {
    b.watcher.with_timing(|t| (t.held_retry, t.told_spacing) = (RETRY, crate::watch::Timing::SHORT));
}

/// How often a held message retries here (`quick`).
const RETRY: Duration = Duration::from_millis(100);

/// Long enough for a held message to have retried several times, and been
/// typed if the gate had let it.
const SEVERAL_RETRIES: Duration = Duration::from_millis(600);

/// Wait, up to 15 seconds, for the stand-in to log `line`.
pub(super) async fn logs(si: &StandIn, line: &str) {
    for _ in 0..750 {
        if si.log().contains(line) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the stand-in never logged {line}: {}", si.log());
}

/// Wait, up to 15 seconds, until nothing is held for `terminal`.
async fn nothing_held(b: &Board, terminal: Uuid) {
    for _ in 0..750 {
        if !b.svc.store.holds_compose(terminal).unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("still held");
}

/// A projector open on `terminal`, so its rows can be read.
fn rows_open(b: &Board, terminal: Uuid) {
    let transcript = b.dir.path().join(format!("{terminal}.jsonl"));
    std::fs::write(&transcript, "").unwrap();
    crate::session_projectors::global().open(terminal, transcript);
}

fn rows(terminal: Uuid) -> Vec<Row> {
    crate::session_projectors::global().page(terminal, None, 100).unwrap_or_default()
}

/// The held rows' texts and states, retracted ones left out.
fn held_rows(terminal: Uuid) -> Vec<(String, QueuedState)> {
    rows(terminal)
        .into_iter()
        .filter(|r| r.id.starts_with("held:") && !r.retracted)
        .filter_map(|r| match r.kind {
            RowKind::Queued(q) => Some((q.text, q.state)),
            _ => None,
        })
        .collect()
}

fn not_sent(terminal: Uuid) -> Vec<String> {
    rows(terminal)
        .into_iter()
        .filter_map(|r| match r.kind {
            RowKind::Notice(n) if n.kind == NoticeKind::NotSent => Some(n.text),
            _ => None,
        })
        .collect()
}

/// A working claude with a call in flight, when an Enter could land on a
/// dialog.
async fn working_with_a_call(b: &Board) -> (Terminal, StandIn) {
    let (agent, si) = idle_claude(b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    (agent, si)
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
    quick(&b);
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

/// A turn running beside background agents
/// (`captures/claude-2.1.296-working-beside-background-agents-120x40-e.txt`)
/// is a turn: the registry's busy is the turn's, and a call in flight holds
/// the send. Nothing is typed into the running turn.
#[tokio::test]
async fn a_turn_running_beside_background_agents_is_mid_turn() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("working-background").await;
    b.screen_with(agent.id, "◯ general-purpose").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_main"));
    assert_eq!(b.watcher.compose_into(agent.id, "after this", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    assert!(b.svc.store.holds_compose(agent.id).unwrap());
}

/// Both sends are answered Queued at once with nothing typed and shown as
/// Queued on the pane's rows; once the call ends each goes into claude's
/// queue, in the order sent, and its held row is taken back.
#[tokio::test]
async fn sends_claude_cant_take_yet_are_held_shown_and_queued_in_order() {
    let b = board().await;
    quick(&b);
    let (agent, si) = working_with_a_call(&b).await;
    rows_open(&b, agent.id);
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(b.watcher.compose_into(agent.id, "first", &[]).await.expect("held"), Turn::During);
    assert_eq!(b.watcher.compose_into(agent.id, "second", &[]).await.expect("held"), Turn::During);
    assert_eq!(held_rows(agent.id), [("first".into(), QueuedState::Waiting), ("second".into(), QueuedState::Waiting)]);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    logs(&si, "QUEUED second").await;
    let log = si.log();
    assert!(log.find("QUEUED first").unwrap() < log.find("QUEUED second").unwrap(), "{log}");
    nothing_held(&b, agent.id).await;
    assert!(held_rows(agent.id).is_empty(), "{:?}", rows(agent.id));
    si.show("idle").await;
    si.submits(2).await;
    assert_eq!(si.submitted(), ["first", "second"], "{}", si.log());
}

/// The gate said hold, and before the send is held a second send finds the
/// call over: it waits its turn behind the first, never typed ahead of it.
#[tokio::test]
async fn a_send_racing_one_being_held_goes_after_it() {
    let b = board().await;
    quick(&b);
    let (agent, si) = working_with_a_call(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let (said, heard) = tokio::sync::oneshot::channel();
    let (go, wait) = tokio::sync::oneshot::channel();
    b.watcher.composed_held.lock().unwrap().pause_before_hold = Some((said, wait));
    let watcher = b.watcher.clone();
    let first = tokio::spawn(async move { watcher.compose_into(agent.id, "first", &[]).await });
    heard.await.unwrap();
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    let watcher = b.watcher.clone();
    let second = tokio::spawn(async move { watcher.compose_into(agent.id, "second", &[]).await });
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    go.send(()).unwrap();
    assert_eq!(first.await.unwrap().expect("held"), Turn::During);
    second.await.unwrap().expect("queued");
    logs(&si, "QUEUED first").await;
    logs(&si, "QUEUED second").await;
    let log = si.log();
    assert!(log.find("QUEUED first").unwrap() < log.find("QUEUED second").unwrap(), "{log}");
}

/// A command mid-turn waits for the turn to end: held, then run.
#[tokio::test]
async fn a_command_mid_turn_is_held_until_the_turn_ends() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(b.watcher.compose_into(agent.id, "/init", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    logs(&si, "COMMAND /init").await;
}

/// A draft left in the box, or someone typing, keeps a held message held:
/// never typed over the draft, never dropped, and sent once the box is clear.
#[tokio::test]
async fn a_held_message_waits_out_a_draft() {
    let b = board().await;
    quick(&b);
    let (agent, si) = working_with_a_call(&b).await;
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(b.watcher.compose_into(agent.id, "kept", &[]).await.expect("held"), Turn::During);
    // The draft is on the screen before the call ends, so from the first
    // retry the call no longer holds it, only the draft does. Ended first,
    // a retry could find the box empty before the draft was drawn and type
    // into it (a slow CI runner did, at a 100 ms retry).
    //
    // And no retry may straddle the two: one that read the box before the
    // draft was drawn, and asked about the call only after it ended, found
    // neither and typed over the draft (main's CI, run 38673266000; ov-547).
    // The pane's slot is held across the call's end, which waits out a try
    // in flight; every try after it reads the draft.
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    let slot = b.watcher.compose_slot(agent.id).await;
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    drop(slot);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    assert!(b.svc.store.holds_compose(agent.id).unwrap(), "dropped over a draft");
    let next = b.watcher.compose_into(agent.id, "next", &[]).await;
    assert_eq!(next.expect("behind it, not refused for the draft"), Turn::During);
    // The person clears it, the turn still running: each goes into claude's
    // queue, in order.
    si.show("working-cleared").await;
    logs(&si, "QUEUED kept").await;
    logs(&si, "QUEUED next").await;
    let log = si.log();
    assert!(log.find("QUEUED kept").unwrap() < log.find("QUEUED next").unwrap(), "{log}");
}

/// A message held past `GIVE_UP_MS` is given up: Withdrawn on the pane's
/// rows, with a notice saying so, and gone from the line.
#[tokio::test]
async fn a_message_held_too_long_says_it_wasnt_sent() {
    let b = board().await;
    quick(&b);
    let (agent, si) = working_with_a_call(&b).await;
    rows_open(&b, agent.id);
    assert_eq!(b.watcher.compose_into(agent.id, "too late", &[]).await.expect("held"), Turn::During);
    farcooler_store::testing::backdate_compose_holds(&b.svc.store, super::compose::held::GIVE_UP_MS);
    nothing_held(&b, agent.id).await;
    assert_eq!(held_rows(agent.id), [("too late".into(), QueuedState::Withdrawn)]);
    assert_eq!(not_sent(agent.id), ["Not sent: “too late”. Claude didn’t take it within 4 hours."]);
    nothing_typed(&si);
}

/// Held messages are kept in the store: a daemon started afresh finds them
/// and tells them, in order; one a restart caught mid-typing is never typed
/// again, and the pane says it may not have arrived.
#[tokio::test]
async fn held_messages_survive_a_restart() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    rows_open(&b, agent.id);
    let _hook = hook_on_submit(&b, &si);
    // As a daemon before this one left them: rows, and no teller.
    b.svc.store.hold_compose(agent.id, "typed before", &[], &[]).unwrap();
    farcooler_store::testing::claim_compose_holds(&b.svc.store);
    b.svc.store.hold_compose(agent.id, "one", &[], &[]).unwrap();
    b.svc.store.hold_compose(agent.id, "two", &[], &[]).unwrap();
    b.watcher.spawn_pumps();
    si.submits(2).await;
    assert_eq!(si.submitted(), ["one", "two"], "{}", si.log());
    nothing_held(&b, agent.id).await;
    assert_eq!(
        not_sent(agent.id),
        ["“typed before” may not have reached Claude: the runner restarted as it was typed. Check the terminal before sending it again."]
    );
}

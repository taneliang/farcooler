//! ov-823 (ov-816 review, N4 and N5): a waiting message rides out a read
//! that failed, and gives up on a run of them; a shell in the pane is claude
//! gone; a wait resumed against another conversation is given up.

use super::*;
use crate::watch::answer_wake::compose::fail_reads;

/// The words a run of reads that never came right ends the wait with.
const NOT_RUNNING: &str = "Not sent: “what did they find”. Claude isn’t running in this pane anymore.";

/// A wait with its bad reads cut to `cap`.
async fn waiting_with_cap(b: &Board, cap: Duration) -> (Terminal, StandIn) {
    let waiting = waiting_through(b, "shells-busy").await;
    b.watcher.with_timing(|t| t.torn_cap = cap);
    waiting
}

/// The registry file caught mid-write, as claude's own write leaves it.
fn tear(si: &StandIn) {
    std::fs::write(registry_of(si), "{\"pid\":").unwrap();
}

/// One read that failed mid-wait doesn't end it: the turn ends, and the
/// message goes in.
#[tokio::test]
async fn a_screen_read_that_fails_once_does_not_end_the_wait() {
    let b = board().await;
    let (agent, si) = waiting_with_cap(&b, Duration::from_secs(5)).await;
    fail_reads(agent.id, 4);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(waiting_rows(agent.id), ["what did they find"], "given up on a failed read: {:?}", not_sent(agent.id));
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Reads that keep failing do: after `torn_cap`, and not before.
#[tokio::test]
async fn screen_reads_that_keep_failing_end_the_wait_after_the_cap() {
    let b = board().await;
    let (agent, si) = waiting_with_cap(&b, Duration::from_millis(1_500)).await;
    let began = std::time::Instant::now();
    fail_reads(agent.id, usize::MAX);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(waiting_rows(agent.id), ["what did they find"], "given up before the cap");
    waits_over(&b).await;
    assert!(began.elapsed() >= Duration::from_millis(1_500), "gave up after {:?}", began.elapsed());
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), [NOT_RUNNING]);
}

/// A registry file caught mid-write at the turn's end is tried again.
#[tokio::test]
async fn a_torn_registry_read_does_not_end_the_wait() {
    let b = board().await;
    let (agent, si) = waiting_with_cap(&b, Duration::from_secs(5)).await;
    tear(&si);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(waiting_rows(agent.id), ["what did they find"], "given up on a torn read: {:?}", not_sent(agent.id));
    // claude's next write (a rename) heals it, the turn over.
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// A registry that stays torn ends it after the cap, in its own words.
#[tokio::test]
async fn a_registry_that_stays_torn_ends_the_wait_after_the_cap() {
    let b = board().await;
    let (agent, si) = waiting_with_cap(&b, Duration::from_millis(1_500)).await;
    let began = std::time::Instant::now();
    tear(&si);
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(waiting_rows(agent.id), ["what did they find"], "given up before the cap");
    waits_over(&b).await;
    assert!(began.elapsed() >= Duration::from_millis(1_500), "gave up after {:?}", began.elapsed());
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert_eq!(
        not_sent(agent.id),
        ["The runner couldn’t tell whether Claude was free, so “what did they find” is still in Claude’s box. Check the terminal."]
    );
}

/// claude quitting to a shell is claude gone, said at once, not a screen the
/// runner couldn't read, and not after the cap.
#[tokio::test]
async fn a_shell_in_the_pane_is_claude_gone() {
    let b = board().await;
    let (agent, si) = waiting_with_cap(&b, Duration::from_secs(8)).await;
    let began = std::time::Instant::now();
    std::fs::write(&si.control, "quit").unwrap();
    waits_over(&b).await;
    assert!(began.elapsed() < Duration::from_secs(6), "waited out the cap: {:?}", began.elapsed());
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), [NOT_RUNNING]);
}

/// Review N4: a wait picked up after a restart by a claude with the pid it
/// had but another conversation (a /clear, a resume, a pid come round again)
/// is given up, unentered.
#[tokio::test]
async fn a_wait_resumed_against_another_session_is_given_up() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "shells-busy").await;
    b.watcher.stop_waits_for_tests();
    tokio::time::sleep(Duration::from_millis(400)).await;
    farcooler_store::testing::rewrite_compose_waits(&b.svc.store, |wait| {
        let mut kept: serde_json::Value = serde_json::from_str(wait).unwrap();
        kept["session"] = "another-session".into();
        kept.to_string()
    });
    si.show("shells").await;
    b.watcher.spawn_pumps();
    waits_over(&b).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), ["Claude restarted while “what did they find” waited, so the runner didn’t send it. Check the terminal."]);
}

/// Why the first version of this lane failed on Linux CI: claude's hook can
/// be heard while the Enter's send is still returning, and a confirmation
/// counted from the return missed it ("didn't confirm"). It's counted from
/// the key. The return here takes 150 ms (the nextest processes are one per
/// test, so the pause can't reach another test).
#[tokio::test]
async fn a_hook_heard_before_the_enter_returns_still_confirms_it() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "shells-busy").await;
    crate::watch::answer_wake::compose::slow_return(Duration::from_millis(150));
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    waits_over(&b).await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

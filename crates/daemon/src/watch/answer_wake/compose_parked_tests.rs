//! ov-827: a message whose turn outlasts the wait's often-read window is not
//! left in claude's box for good. It parks, and is sent at the turn's end
//! under every rule a waiting message keeps (`compose::pending`, "Parked"):
//! two reads between turns, never into a dialog, never into a box that
//! changed. The watcher's sight of the turn ending wakes it; a restart keeps
//! it. Its row says it's waiting to send throughout.
//!
//! Each test's waits are the production ones scaled down and injected
//! (`Timing`): `READ_OFTEN` stands in for `TURN_BEGUN_WAIT`'s ten minutes,
//! and a turn several times as long for the owner's fifteen-minute one.

use super::*;

/// The stand-in for `TURN_BEGUN_WAIT`: how long the message reads the pane
/// every `wait_poll` before it parks.
const READ_OFTEN: Duration = Duration::from_millis(800);

/// The words a message given up because its box changed ends with.
const CHANGED: &str = "Claude’s box changed while “what did they find” waited, so the runner didn’t send it. Check the terminal.";

/// A message waiting out a turn `mode` claude began after its paste, with no
/// fence to be had, parked once `READ_OFTEN` is out, reading the pane every
/// `poll` unless the watcher wakes it, and kept for a minute: answered
/// Queued, the pane's rows open, and the log said it parked.
async fn parked_through(b: &Board, mode: &'static str, poll: Duration) -> (Terminal, StandIn, Sink, tracing::subscriber::DefaultGuard) {
    let (agent, si) = unfenceable_claude(b).await;
    quick_turns(b, &si);
    b.watcher.with_timing(|t| {
        t.wait_slows_after = Duration::from_millis(200);
        t.carry_agents_for = Duration::from_millis(200);
        t.turn_begun_wait = READ_OFTEN;
        t.parked_poll = poll;
        t.left_kept = Duration::from_secs(60);
        t.compose_confirm = crate::watch::Timing::SHORT;
    });
    drop(hook_on_submit(b, &si));
    rows_open(b, agent.id);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook(mode)));
    let (sink, guard) = logging();
    assert_eq!(compose_promptly(b, &agent, "what did they find").await.expect("queued"), Turn::During);
    logs_line(&sink, "a waiting message parked in the box until the turn ends").await;
    (agent, si, sink, guard)
}

/// Acceptance 1: a turn that runs on well past the often-read window (the
/// owner's fifteen minutes against the ten) leaves the message parked, never
/// given up and never entered, its row waiting to send; once the turn ends,
/// the plain Enter goes, once, after the turn, and nothing says it wasn't
/// sent. Before ov-827 it was given up at the window's end and left in the
/// box for good.
#[tokio::test]
async fn a_message_whose_turn_outlasts_the_wait_is_sent_when_it_ends() {
    let b = board().await;
    let (agent, si, sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    tokio::time::sleep(READ_OFTEN * 4).await;
    assert!(enters(&si).is_empty(), "no Enter during the turn: {}", si.log());
    assert!(not_sent(agent.id).is_empty(), "given up mid-turn: {:?}\n{}", not_sent(agent.id), logged(&sink));
    assert_eq!(waiting_rows(agent.id), ["what did they find"], "its row waits to send");
    assert_eq!(b.svc.store.compose_waits().unwrap().len(), 1, "kept for a restart");
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    let log = si.log();
    assert_eq!(enters(&si), ["ENTER"], "{log}");
    assert!(log.find("ENTER").unwrap() > log.rfind("MODE shells").unwrap(), "{log}");
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// Parked with an hour between its own reads, the message reads the pane
/// when the watcher sees the pane's activity move: the turn's end, read by
/// the watcher's own sample, sends it within seconds, not within the hour.
#[tokio::test]
async fn a_parked_message_is_woken_by_the_watcher_seeing_the_turn_end() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_secs(60 * 60)).await;
    // The watcher reads the turn running, then over.
    b.watcher.sample().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    si.show("shells").await;
    let ended = std::time::Instant::now();
    for _ in 0..40 {
        b.watcher.sample().await;
        if !si.submitted().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(si.submitted(), ["what did they find"], "never woken: {}", si.log());
    assert!(ended.elapsed() < Duration::from_secs(10), "sent only after {:?}", ended.elapsed());
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Acceptance 3: the turn ends with claude's box holding something else (a
/// person edited it straight in tmux, no input mark): nothing is sent, and
/// the row says the box changed.
#[tokio::test]
async fn a_box_edited_while_parked_gets_no_enter_and_says_so() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    si.show("draft:what did they find out").await;
    waits_over(&b).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), [CHANGED]);
    assert!(waiting_rows(agent.id).is_empty());
}

/// Acceptance 3, cleared: the box emptied while the turn runs, the message
/// parked: nothing is sent when the turn ends, and the row says it was
/// cleared.
#[tokio::test]
async fn a_box_cleared_while_parked_gets_no_enter_and_says_so() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    si.show("working-cleared").await;
    waits_over(&b).await;
    si.show("shells").await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), ["Claude’s box was cleared while “what did they find” waited, so the runner didn’t send it."]);
}

/// Acceptance 4: the turn ends with a permission dialog drawn, before
/// claude's registry says one is up (it still says idle): no Enter while it
/// is, the parked message reading it the while; answered from the phone (a
/// marked key, the box unchanged), and the turn over, the plain Enter goes,
/// after the dialog.
#[tokio::test]
async fn a_dialog_up_at_the_turns_end_gets_no_enter_until_it_goes() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    si.show("menu-lagging").await;
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    assert!(enters(&si).is_empty(), "an Enter on the dialog: {}", si.log());
    assert_eq!(waiting_rows(agent.id), ["what did they find"]);
    typed_marked(&b, &agent, "31").await;
    super::super::logs(&si, "ANSWERED 1").await;
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    let log = si.log();
    assert_eq!(enters(&si), ["ENTER"], "{log}");
    assert!(log.find("ENTER").unwrap() > log.rfind("MODE shells").unwrap(), "{log}");
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// A runner restarted while the message is parked, past what used to be its
/// whole wait: the next start picks it up, still waiting to send, and it's
/// sent once the turn ends, not given up as the runner having been down.
#[tokio::test]
async fn a_parked_message_is_sent_after_a_runner_restart() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    b.watcher.stop_waits_for_tests();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(b.svc.store.compose_waits().unwrap().len(), 1, "kept across the stop");
    // Queued well before the restart: past `turn_begun_wait`, inside `left_kept`.
    kept_with(&b, "queued_at", farcooler_store::testing::now_millis() - 20_000);
    b.watcher.spawn_pumps();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(not_sent(agent.id).is_empty(), "given up at the restart: {:?}", not_sent(agent.id));
    assert_eq!(waiting_rows(agent.id), ["what did they find"]);
    si.show("shells").await;
    super::super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Past `left_kept`, the message is given up as before: still in the box,
/// unentered, and the row says Claude stayed busy.
#[tokio::test]
async fn a_message_parked_past_its_limit_is_given_up() {
    let b = board().await;
    let (agent, si, _sink, _logging) = parked_through(&b, "working", Duration::from_millis(300)).await;
    b.watcher.with_timing(|t| t.left_kept = READ_OFTEN * 3);
    waits_over(&b).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert_eq!(not_sent(agent.id), ["Not sent: “what did they find”. Claude stayed busy, so it’s still in Claude’s box."]);
}

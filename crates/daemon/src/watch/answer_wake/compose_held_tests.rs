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

/// claude 2.1.309 with its turn over and background shells running, no
/// agent (`captures/claude-2.1.309-shells-*-59x41-e.txt`): its registry says
/// `shell`, and once the box holds the message the mode line keeps only
/// `· 2 shells`, with no `↓ to manage` and no agent rows. The owner's
/// message, curly apostrophes and all, goes in at once, Sent. Before ov-795
/// the read-back before the Enter saw no background work, took the
/// registry's `shell` for a turn, and left it as `paste_left`.
#[tokio::test]
async fn a_claude_beside_background_shells_takes_a_message_as_its_next_turn() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let text = "Btw to make things go faster, if there are multiple trains waiting to land, if it’s safe you can merge \
                them into the same train. Otherwise you’ll have a million trains queueing unnecessarily. Please update \
                the manager skill also with such best practices";
    let turn = b.watcher.compose_into(agent.id, text, &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), [text], "{}", si.log());
}

/// Beside the same shells, a box that holds more than the message still
/// refuses: someone typing straight into the terminal after the paste,
/// which leaves no input mark, puts more in the box before the Enter. The
/// count says only that shells run, never what the box holds.
#[tokio::test]
async fn a_box_holding_more_than_the_message_beside_background_shells_still_refuses() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let (svc, id) = (b.svc.clone(), agent.id);
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        // Off the test's runtime, which this hook blocks.
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async {
                    let unmarked = crate::runtime::Runtime { marks: None, ..svc.runtime() };
                    unmarked.send_bytes_hex(id, &" and mine".bytes().map(|c| format!("{c:02x}")).collect::<String>()).await.unwrap();
                    for _ in 0..750 {
                        if svc.screen(id).await.is_ok_and(|(screen, _, _)| screen.contains("find and mine")) {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    panic!("the typing never showed");
                });
            });
        });
    }));
    let sink = Sink::default();
    let refused = {
        let _logging = tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish());
        b.watcher.compose_into(agent.id, "what did they find", &[]).await
    };
    assert!(matches!(refused, Err(DomainError::Conflict { what: "paste_left" })), "{refused:?}");
    assert!(si.submitted().is_empty(), "{}", si.log());
    // Which check left it, and the box's shape, never its text (ov-795).
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("a paste was left in the box") && logged.contains("not_shown"), "{logged}");
    assert!(logged.contains("background=Shells") && logged.contains("held=\"holds\""), "{logged}");
    assert!(!logged.contains("find"), "the text is never logged: {logged}");
}

/// The same `busy` beside shells alone, seen at the gate: a turn, so the
/// message is held for its end, never typed into it.
#[tokio::test]
async fn a_busy_registry_beside_background_shells_holds_the_message() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells-busy").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_main"));
    assert_eq!(b.watcher.compose_into(agent.id, "after this", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    assert!(b.svc.store.holds_compose(agent.id).unwrap());
}

/// A log sink for one test's subscriber.
#[derive(Clone, Default)]
struct Sink(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
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
    let (agent, si) = idle_claude(&b).await;
    a_command_is_held_until_the_turn_ends(&b, &agent, &si).await;
}

/// The same with a stand-in slow to write its registry, as one is on a loaded
/// runner: every retry while the mode changes meets a registry being
/// written. Written in place, a retry read it torn, found no live claude,
/// and gave the command up as unconfirmable (ov-640: CI run 39108505933 saw
/// the stand-in go idle, working, idle, working, idle and never log it).
#[tokio::test]
async fn a_command_is_held_through_a_registry_being_written() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", "STAND_IN_SLOW_REGISTRY=0.3").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    super::compose_tests::hooked(&b);
    a_command_is_held_until_the_turn_ends(&b, &agent, &si).await;
}

/// Written in place, with the file empty for 0.3 s each time (the stand-in's
/// `STAND_IN_TORN_REGISTRY`): a retry that meets it reads a file that exists
/// but doesn't parse, which is claude mid-write, not no claude (ov-645). Red
/// before: that read was "no live entry", and the command was given up.
#[tokio::test]
async fn a_command_is_held_through_a_registry_written_in_place() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", "STAND_IN_TORN_REGISTRY=0.3").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    super::compose_tests::hooked(&b);
    a_command_is_held_until_the_turn_ends(&b, &agent, &si).await;
}

/// The registry file of the one claude stand-in `si` is.
fn registry_of(si: &StandIn) -> PathBuf {
    let sessions = si.control.parent().unwrap().join("config").join("sessions");
    std::fs::read_dir(sessions).unwrap().next().unwrap().unwrap().path()
}

/// A registry that stays torn is held only for `torn_cap` (10 s in the
/// daemon; here 1 s, which `Timing` lets a test set), then given up as
/// unconfirmable, so a format the runner doesn't know can't hold a message
/// for the 4 hours: still held at half the cap, gone after it, never typed.
#[tokio::test]
async fn a_registry_that_stays_torn_is_given_up_after_its_cap() {
    let b = board().await;
    quick(&b);
    b.watcher.with_timing(|t| t.torn_cap = Duration::from_secs(1));
    let (agent, si) = idle_claude(&b).await;
    rows_open(&b, agent.id);
    std::fs::write(registry_of(&si), "{\"pid\":").unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "stuck", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(b.svc.store.holds_compose(agent.id).unwrap(), "given up before the cap");
    nothing_held(&b, agent.id).await;
    assert_eq!(held_rows(agent.id), [("stuck".into(), QueuedState::Withdrawn)]);
    assert_eq!(not_sent(agent.id), ["Not sent: “stuck”. The runner couldn’t type it."]);
    nothing_typed(&si);
}

/// The cap counts one run of torn reads. After a message was given up at
/// the cap, a later message held through a fresh tear gets its own full cap,
/// not the old run's already-spent clock (the ov-645 review's regression).
#[tokio::test]
async fn a_given_up_run_does_not_shorten_the_next() {
    let b = board().await;
    quick(&b);
    b.watcher.with_timing(|t| t.torn_cap = Duration::from_secs(1));
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let file = registry_of(&si);
    let whole = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, "{\"pid\":").unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "first", &[]).await.expect("held"), Turn::During);
    nothing_held(&b, agent.id).await;
    std::fs::write(&file, &whole).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "direct", &[]).await.expect("sent"), Turn::Between);
    std::fs::write(&file, "{\"pid\":").unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "second", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(b.svc.store.holds_compose(agent.id).unwrap(), "given up on its first torn read");
}

/// A try that reads the registry whole ends the run: tears split by a
/// healthy stretch don't add up to the cap (cap 2 s; torn 1.2 s, whole
/// 0.3 s, torn 1.2 s is 2.4 s of tearing but never 2 s in a row).
#[tokio::test]
async fn a_whole_read_between_tears_restarts_the_cap() {
    let b = board().await;
    quick(&b);
    b.watcher.with_timing(|t| t.torn_cap = Duration::from_millis(2000));
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let file = registry_of(&si);
    let whole = std::fs::read_to_string(&file).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "/init", &[]).await.expect("held"), Turn::During);
    let ms = Duration::from_millis;
    std::fs::write(&file, "{\"pid\":").unwrap();
    tokio::time::sleep(ms(1200)).await;
    std::fs::write(&file, &whole).unwrap();
    tokio::time::sleep(ms(300)).await;
    std::fs::write(&file, "{\"pid\":").unwrap();
    tokio::time::sleep(ms(1200)).await;
    assert!(b.svc.store.holds_compose(agent.id).unwrap(), "the cap ran across a whole read");
}

/// A registry that tears after the gate, before the last look ahead of the
/// Enter, leaves the text in the box: no Enter on a turn that can't be ruled
/// out (`still_between`).
#[tokio::test]
async fn a_registry_torn_at_the_last_check_gets_no_enter() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("torn-on-paste").await;
    let sent = b.watcher.compose_into(agent.id, "one\ntwo", &[]).await;
    assert!(matches!(sent, Err(DomainError::Conflict { what: "paste_left" })), "{sent:?}");
    assert!(si.log().contains("PASTE ") && !si.log().contains("ENTER"), "{}", si.log());
}

/// A torn read that heals before the cap costs nothing, and a file that is
/// gone is no claude at once: refused, not held.
#[tokio::test]
async fn a_torn_registry_that_heals_is_typed_and_a_missing_one_is_refused() {
    let b = board().await;
    quick(&b);
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let file = registry_of(&si);
    let whole = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, &whole[..whole.len() / 2]).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "patient", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(&si);
    std::fs::write(&file, whole).unwrap();
    si.submits(1).await;
    assert_eq!(si.submitted(), ["patient"], "{}", si.log());
    nothing_held(&b, agent.id).await;
    std::fs::remove_file(&file).unwrap();
    let gone = b.watcher.compose_into(agent.id, "nobody", &[]).await;
    assert!(matches!(gone, Err(DomainError::Conflict { what: "unconfirmable" })), "{gone:?}");
}

async fn a_command_is_held_until_the_turn_ends(b: &Board, agent: &Terminal, si: &StandIn) {
    quick(b);
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(b, si);
    assert_eq!(b.watcher.compose_into(agent.id, "/init", &[]).await.expect("held"), Turn::During);
    tokio::time::sleep(SEVERAL_RETRIES).await;
    nothing_typed(si);
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    logs(si, "COMMAND /init").await;
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
    let slot = b.watcher.compose_slot_for_test(agent.id).await;
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

// ov-808: what a compose does when the read-back before its between-turns
// Enter finds claude's registry `busy` or its spinner drawn
// (`compose::enter_between`). Background agents a tall box hides are no
// turn, and the message is sent; a turn begun since the gate takes it as
// claude's own queue would, under the fence, or once it ends; only a turn
// still running past the wait with no fence to be had leaves it.

/// How long a turn begun is waited out here (`Timing::turn_begun_wait`).
const WAIT: Duration = Duration::from_millis(600);

/// Sends spaced sooner, and a turn begun waited out for `WAIT`.
fn quick_turns(b: &Board) {
    b.watcher.with_timing(|t| (t.told_spacing, t.turn_begun_wait) = (crate::watch::Timing::SHORT, WAIT));
}

/// What `compose_into` logged, run under a sink of its own.
async fn compose_logged(b: &Board, agent: &Terminal, text: &str) -> (Result<Turn>, String) {
    let sink = Sink::default();
    let result = {
        let _logging = tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish());
        b.watcher.compose_into(agent.id, text, &[]).await
    };
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    (result, logged)
}

/// A message about as long as the owner's two on build 3989 (ov-808, 303 and 276
/// characters), with a curly apostrophe as theirs have.
const OWNER: &str = "I think these glass round texts here are actually kinda weird. Aren’t these usually pills, i.e. not a \
                     round rect but the swipe to dismiss thing? Also could you check whether the image away to dismiss \
                     works, since it seemed off when I tried it on the phone earlier today and I would like it fixed soon \
                     please, before the next train goes.";

/// The owner's case, logged twice on build 3989 as `check=registry_busy
/// main=Idle background=Shells held=holds`: claude between turns beside
/// background agents and shells, on a pane short enough that once the
/// message is in its box claude drops the agent rows, leaving only `· 2
/// shells` (measured on 2.1.309 at 59x23,
/// `captures/claude-2.1.309-agents-shells-*-59x23-e.txt`). Its registry says
/// `busy` for the agents throughout. The gate saw the agents, so the
/// read-back before the Enter keeps them, and the message is Sent as its
/// next turn. Before ov-808 that `busy` beside what read as shells alone was
/// taken for a turn begun, and the message was left in the box.
#[tokio::test]
async fn a_message_beside_agents_its_box_hides_is_sent() {
    let b = board().await;
    quick_turns(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("agents-shells").await;
    let screen = b.screen_with(agent.id, "◯ general-purpose").await;
    assert_eq!(b.svc.registry().background_work("claude", &screen), farcooler_core::activity::Background::Agents);
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_call(SESSION, Some("toolu_bg"), Some("agent-bg"));
    let _hook = hook_on_submit(&b, &si);
    let (svc, id) = (b.svc.clone(), agent.id);
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        // The screen the read-back takes, as the log had it: Idle, the
        // agent rows gone, the shells' count left, the registry busy.
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let (screen, _, _) = rt.block_on(svc.screen(id)).unwrap();
                let registry = svc.registry();
                assert_eq!(registry.classify_main("claude", &screen), AgentActivity::Idle);
                assert_eq!(registry.background_work("claude", &screen), farcooler_core::activity::Background::Shells);
                assert!(!screen.contains("◯"), "{screen}");
            });
        });
    }));
    let (turn, logged) = compose_logged(&b, &agent, OWNER).await;
    assert!(matches!(turn, Ok(Turn::Between)), "{turn:?}\n{logged}\n{}", si.log());
    assert_eq!(si.submitted(), [OWNER], "{}", si.log());
    assert!(!logged.contains("a paste was left in the box"), "{logged}");
}

/// A turn claude began between the paste and the Enter, its registry `busy`
/// beside shells alone, its spinner not drawn: the session's hooks are
/// quiet, so the message goes in under the fence, as a mid-turn send does
/// (ov-808), not left in the box.
#[tokio::test]
async fn a_turn_begun_beside_background_shells_takes_the_message_under_the_fence() {
    let b = board().await;
    quick_turns(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("shells-busy")));
    let (turn, logged) = compose_logged(&b, &agent, "what did they find").await;
    assert!(turn.is_ok(), "{turn:?}\n{logged}\n{}", si.log());
    assert_eq!(si.submitted(), ["what did they find"], "{}", si.log());
    assert!(!logged.contains("a paste was left in the box"), "{logged}");
}

/// The same with the turn's spinner drawn by the read-back: claude queues
/// the message, under the fence, and it's answered Queued.
#[tokio::test]
async fn a_turn_drawn_before_the_enter_queues_the_message() {
    let b = board().await;
    quick_turns(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    let (turn, logged) = compose_logged(&b, &agent, "what did they find").await;
    assert_eq!(turn.expect("queued"), Turn::During, "{logged}\n{}", si.log());
    assert!(si.log().contains("QUEUED what did they find"), "{}", si.log());
}

/// A turn begun with a call in flight, so no fence can be had (a dialog
/// drawn as the key lands would take it as Yes): the message waits in the
/// box, and goes in between turns once claude's registry says the turn is
/// over, within the wait. The owner's orchestrator, which no hook is heard
/// from (ov-711), has no fence ever, and is sent this way.
#[tokio::test]
async fn a_turn_that_ends_within_the_wait_takes_the_message_between_turns() {
    let b = board().await;
    quick_turns(&b);
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(10));
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let (shower, asks) = (si.clone(), b.svc.hooks().asks().clone());
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        shower.show_in_hook("shells-busy");
        asks.mark_tool_starting(SESSION, Some("toolu_main"));
    }));
    let ender = si.clone();
    let ends = tokio::spawn(async move {
        while !ender.log().contains("MODE shells-busy") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(ender.submitted().is_empty(), "no Enter while the turn ran: {}", ender.log());
        ender.show("shells").await;
    });
    let (turn, logged) = compose_logged(&b, &agent, "what did they find").await;
    ends.await.unwrap();
    assert_eq!(turn.expect("sent"), Turn::Between, "{logged}\n{}", si.log());
    assert_eq!(si.submitted(), ["what did they find"], "{}", si.log());
}

/// The one case that still leaves it: a turn begun with a call in flight
/// that is still running when the wait ends. No fence, no turn's end, so
/// nothing safe to press; left, `registry_busy` logged with the screen's
/// shape, after the wait and not before.
#[tokio::test]
async fn a_turn_still_running_with_a_call_past_the_wait_leaves_the_paste() {
    let b = board().await;
    quick_turns(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let (shower, asks) = (si.clone(), b.svc.hooks().asks().clone());
    let began = Arc::new(std::sync::Mutex::new(None));
    let at = began.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        shower.show_in_hook("shells-busy");
        asks.mark_tool_starting(SESSION, Some("toolu_main"));
        *at.lock().unwrap() = Some(std::time::Instant::now());
    }));
    let (refused, logged) = compose_logged(&b, &agent, "what did they find").await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "paste_left" })), "{refused:?}");
    assert!(began.lock().unwrap().unwrap().elapsed() >= WAIT, "left before the wait was out");
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert!(logged.contains("registry_busy") && logged.contains("background=Shells"), "{logged}");
    assert!(!logged.contains("find"), "the text is never logged: {logged}");
}

/// A `◯ ` in the message itself is no agent row: beside shells alone, with
/// the box holding `◯ general-purpose …`, a turn begun before the Enter
/// with a call in flight is still a turn by the registry's `busy`, waited
/// out and left, never sent between turns. Only rows under the mode line,
/// or the gate's, say agents run (ov-795 re-review, ov-808).
#[tokio::test]
async fn an_agent_glyph_in_the_message_is_no_background_agent() {
    let b = board().await;
    quick_turns(&b);
    let (agent, si) = idle_claude(&b).await;
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let (shower, asks) = (si.clone(), b.svc.hooks().asks().clone());
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        shower.show_in_hook("shells-busy");
        asks.mark_tool_starting(SESSION, Some("toolu_main"));
    }));
    let (refused, logged) = compose_logged(&b, &agent, "◯ general-purpose  Sleeper 1 is it done").await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "paste_left" })), "{refused:?}");
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert!(logged.contains("registry_busy") && logged.contains("background=Shells"), "{logged}");
}

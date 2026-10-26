//! ov-808: what a compose does when the read-back before its between-turns
//! Enter finds claude's registry `busy` or its spinner drawn
//! (`compose::enter_between`, `compose::pending`). Background agents a tall
//! box hides are no turn, and the message is sent; a turn begun since the
//! gate takes it under the fence when the hooks give one; with no fence the
//! send is answered Queued at once and its Enter waits out the turn off the
//! typing lock, going in only after two reads between turns, and given up,
//! unentered, when Stop is pressed or the wait runs out.
//!
//! The stand-in logs `ENTER fenced` for an Enter pressed while the fence is
//! held (`Watcher::mark_fence_in`), and `ENTER` for one without.

use super::*;
use crate::watch::answer_wake::interrupt::Key;

/// How long a turn begun is waited out here (`Timing::turn_begun_wait`).
const WAIT: Duration = Duration::from_millis(1_500);

/// Sends spaced sooner, a turn begun waited out for `WAIT`, and every fenced
/// Enter said to `si`.
fn quick_turns(b: &Board, si: &StandIn) {
    b.watcher.with_timing(|t| (t.told_spacing, t.turn_begun_wait, t.held_retry) = (crate::watch::Timing::SHORT, WAIT, Duration::from_millis(100)));
    b.watcher.mark_fence_in(PathBuf::from(format!("{}.fence", si.control.display())));
}

/// A log sink this thread's tasks write to until the guard is dropped.
fn logging() -> (Sink, tracing::subscriber::DefaultGuard) {
    let sink = Sink::default();
    let guard = tracing::subscriber::set_default(tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).finish());
    (sink, guard)
}

fn logged(sink: &Sink) -> String {
    String::from_utf8(sink.0.lock().unwrap().clone()).unwrap()
}

/// Wait, up to 15 seconds, for `sink` to hold `needle`.
async fn logs_line(sink: &Sink, needle: &str) -> String {
    for _ in 0..750 {
        let said = logged(sink);
        if said.contains(needle) {
            return said;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never logged {needle}: {}", logged(sink));
}

/// The stand-in's Enters: `ENTER` lines, fenced or not.
fn enters(si: &StandIn) -> Vec<String> {
    si.log().lines().filter(|l| l.starts_with("ENTER")).map(str::to_string).collect()
}

/// A claude stand-in its hooks were heard from, but no turn boundary yet:
/// no witness can be had (`HookAsks::quiet_mid_turn`), as for the owner's
/// orchestrator, which has none ever (ov-711); Stop is still pressable.
async fn unfenceable_claude(b: &Board) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.svc.hooks().asks().heard(SESSION, false);
    (agent, si)
}

/// `compose_into`, which must answer at once, not wait out the turn.
async fn compose_promptly(b: &Board, agent: &Terminal, text: &str) -> Result<Turn> {
    let began = std::time::Instant::now();
    let turn = b.watcher.compose_into(agent.id, text, &[]).await;
    assert!(began.elapsed() < WAIT, "the send waited out the turn: {:?}", began.elapsed());
    turn
}

/// A message about as long as the owner's two on build 3989 (ov-808, 303 and
/// 276 characters), with a curly apostrophe as theirs have.
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
/// next turn, by the plain Enter. Before ov-808 that `busy` beside what read
/// as shells alone was taken for a turn begun, and the message was left.
#[tokio::test]
async fn a_message_beside_agents_its_box_hides_is_sent() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    quick_turns(&b, &si);
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
    let (sink, _logging) = logging();
    let turn = b.watcher.compose_into(agent.id, OWNER, &[]).await;
    assert!(matches!(turn, Ok(Turn::Between)), "{turn:?}\n{}\n{}", logged(&sink), si.log());
    assert_eq!(si.submitted(), [OWNER], "{}", si.log());
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    assert!(!logged(&sink).contains("a paste was left in the box"), "{}", logged(&sink));
}

/// A turn claude began between the paste and the Enter, its registry `busy`
/// beside shells alone, its spinner not drawn: the session's hooks are
/// quiet, so the message goes in under the fence, as a mid-turn send does.
#[tokio::test]
async fn a_turn_begun_beside_background_shells_takes_the_message_under_the_fence() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    quick_turns(&b, &si);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("shells-busy")));
    let turn = b.watcher.compose_into(agent.id, "what did they find", &[]).await;
    assert!(turn.is_ok(), "{turn:?}\n{}", si.log());
    assert_eq!(si.submitted(), ["what did they find"], "{}", si.log());
    assert_eq!(enters(&si), ["ENTER fenced"], "{}", si.log());
}

/// The same with the turn's spinner drawn by the read-back: claude queues
/// the message, under the fence, and it's answered Queued.
#[tokio::test]
async fn a_turn_drawn_before_the_enter_queues_the_message_under_the_fence() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    quick_turns(&b, &si);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    let turn = b.watcher.compose_into(agent.id, "what did they find", &[]).await;
    assert_eq!(turn.expect("queued"), Turn::During, "{}", si.log());
    assert!(si.log().contains("QUEUED what did they find"), "{}", si.log());
    assert_eq!(enters(&si), ["ENTER fenced"], "{}", si.log());
}

/// The spinner drawn and no fence to be had: answered Queued at once, no
/// Enter while the turn runs, and once the wait is out the message is left,
/// logged `not_idle` with the gate's kind, and the pane's rows say so.
#[tokio::test]
async fn a_turn_drawn_with_no_fence_gets_no_enter_and_is_left() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    rows_open(&b, agent.id);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    let (sink, _logging) = logging();
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("check=\"not_idle\"") && said.contains("gate=Shells"), "{said}");
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert!(not_sent(agent.id).iter().any(|n| n.contains("Claude stayed busy, so it’s still in Claude’s box")), "{:?}", not_sent(agent.id));
}

/// A turn begun with no fence to be had, its registry `busy` beside shells:
/// answered Queued at once; no Enter while the turn runs; once the registry
/// says it's over, read so twice, the plain Enter, and claude takes it.
#[tokio::test]
async fn a_turn_that_ends_within_the_wait_takes_the_message_between_turns() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(10));
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("shells-busy")));
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(enters(&si).is_empty(), "no Enter while the turn ran: {}", si.log());
    si.show("shells").await;
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
}

/// A registry `busy` beside shells still there when the wait is out:
/// left, after the wait, `registry_busy` logged, never the text.
#[tokio::test]
async fn a_turn_still_running_past_the_wait_leaves_the_paste() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    rows_open(&b, agent.id);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("shells-busy")));
    let (sink, _logging) = logging();
    let began = std::time::Instant::now();
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(began.elapsed() >= WAIT, "left before the wait was out");
    assert!(said.contains("registry_busy") && said.contains("background=Shells"), "{said}");
    assert!(!said.contains("find"), "the text is never logged: {said}");
    assert!(enters(&si).is_empty(), "{}", si.log());
    let rows = not_sent(agent.id);
    assert!(rows.iter().any(|n| n == "Not sent: “what did they find”. Claude stayed busy, so it’s still in Claude’s box."), "{rows:?}");
}

/// A `◯ ` in the message itself is no agent row: beside shells alone, a turn
/// begun before the Enter is still a turn by the registry's `busy`, waited
/// out and left, never sent between turns. Only rows under the mode line,
/// or the gate's, say agents run (ov-795 re-review, ov-808).
#[tokio::test]
async fn an_agent_glyph_in_the_message_is_no_background_agent() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("shells-busy")));
    let (sink, _logging) = logging();
    let turn = compose_promptly(&b, &agent, "◯ general-purpose  Sleeper 1 is it done").await;
    assert_eq!(turn.expect("queued"), Turn::During);
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("registry_busy") && said.contains("background=Shells"), "{said}");
    assert!(enters(&si).is_empty(), "{}", si.log());
}

/// Review S2: beside agents the gate saw, the registry can't say a turn
/// runs, only the screen can. One frame that reads between turns mid-turn
/// (here, a quarter second of it) is no turn's end: the plain Enter needs
/// two reads `idle_twice` apart with nothing else between, a second here so
/// a loaded runner's late sleep can't stretch the frame past it. No Enter.
#[tokio::test]
async fn one_stray_idle_frame_mid_turn_gets_no_enter() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    b.watcher.with_timing(|t| (t.idle_twice, t.turn_begun_wait) = (Duration::from_secs(1), Duration::from_secs(3)));
    si.show("agents-shells").await;
    b.screen_with(agent.id, "◯ general-purpose").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working-background")));
    let (sink, _logging) = logging();
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(Duration::from_millis(300)).await;
    si.show("agents-shells").await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    si.show("working-background").await;
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("check=\"not_idle\"") && said.contains("gate=Agents"), "{said}");
    assert!(enters(&si).is_empty(), "{}", si.log());
}

/// Review S1: Stop pressed while a message waits out the turn acts in its
/// own time, never behind the wait, as the wait holds the box only for each
/// try; and the wait then gives the message up, with no Enter, since the
/// person stopped the turn it waited on.
#[tokio::test]
async fn stop_during_the_wait_acts_at_once_and_the_wait_gives_up_unentered() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(6));
    rows_open(&b, agent.id);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    let (sink, _logging) = logging();
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let began = std::time::Instant::now();
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped");
    assert!(began.elapsed() < Duration::from_secs(5), "Stop waited: {:?}", began.elapsed());
    assert!(si.log().lines().any(|l| l == "ESC"), "{}", si.log());
    let said = logs_line(&sink, "check=\"stopped\"").await;
    assert!(said.contains("a paste was left in the box"), "{said}");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    let rows = not_sent(agent.id);
    assert!(rows.iter().any(|n| n == "Not sent: “what did they find”. You stopped Claude, so it’s still in Claude’s box."), "{rows:?}");
}

/// Type `keys` into `agent`'s pane straight, as someone at the terminal
/// would, leaving no input mark.
async fn typed_unmarked(b: &Board, agent: &Terminal, keys: &str) {
    let unmarked = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    unmarked.send_bytes_hex(agent.id, &keys.bytes().map(|c| format!("{c:02x}")).collect::<String>()).await.unwrap();
}

/// A message waiting out a turn beside shells, with no fence to be had:
/// answered Queued, and the pane's rows open.
async fn waiting(b: &Board) -> (Terminal, StandIn) {
    waiting_through(b, "shells-busy").await
}

/// Review F3, ov-816: typing in the box in Far Cooler's terminal while the
/// message waits gives it up, unentered, and the rows say someone typed,
/// never that it didn't land.
#[tokio::test]
async fn typing_in_the_box_during_the_wait_gives_it_up_as_typed() {
    let b = board().await;
    let (sink, _logging) = logging();
    let (agent, si) = waiting(&b).await;
    typed_marked(&b, &agent, "20616e64206d696e65").await;
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("check=\"not_shown\""), "{said}");
    waits_over(&b).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    let want = "Someone typed in Claude’s terminal while “what did they find” waited, so the runner didn’t send it. Check the terminal.";
    assert_eq!(not_sent(agent.id), [want]);
}

/// ov-816: a key typed in Far Cooler's terminal that leaves the box as it
/// was (a dialog's answer, drawn and gone between two slow reads) doesn't
/// end the wait, but no Enter goes until nobody has typed for
/// `TYPED_WITHIN_MS`, as a send's gate waits.
#[tokio::test]
async fn a_key_that_leaves_the_box_holds_the_enter_until_nobody_types() {
    let b = board().await;
    let (agent, si) = waiting(&b).await;
    crate::runtime::mark_input(b.svc.root_dir(), agent.id);
    let marked = std::time::Instant::now();
    si.show("shells").await;
    super::logs(&si, "SUBMIT what did they find").await;
    let quiet = Duration::from_millis(super::interrupt::TYPED_WITHIN_MS as u64);
    assert!(marked.elapsed() >= quiet - Duration::from_millis(100), "entered {:?} after a key", marked.elapsed());
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Review F3: the box changed straight in the terminal while the message
/// waits (no input mark): given up, unentered, as the box changing.
#[tokio::test]
async fn the_box_changing_during_the_wait_gives_it_up_as_changed() {
    let b = board().await;
    let (sink, _logging) = logging();
    let (agent, si) = waiting(&b).await;
    typed_unmarked(&b, &agent, " and mine").await;
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("check=\"not_shown\""), "{said}");
    waits_over(&b).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    let rows = not_sent(agent.id);
    let want = "Claude’s box changed while “what did they find” waited, so the runner didn’t send it. Check the terminal.";
    assert!(rows.iter().any(|n| n == want), "{rows:?}");
}

/// Review F3: Send Now while the message waits is refused, as it is for any
/// box holding text (`interrupt::pressable`), so it can't give the wait up
/// or send it; the wait goes on and enters it once the turn ends.
#[tokio::test]
async fn send_now_during_the_wait_is_refused_and_the_wait_goes_on() {
    let b = board().await;
    let (agent, si) = waiting(&b).await;
    let refused = b.watcher.press(agent.id, Key::SendNow).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "draft" })), "{refused:?}");
    assert!(!si.log().contains("SENDNOW"), "{}", si.log());
    si.show("shells").await;
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Review F2: a second send while the first waits in the box is held behind
/// it and answered Queued, never refused as a draft; once the turn ends the
/// first is entered, then the second is typed and entered, in order.
#[tokio::test]
async fn a_second_send_during_the_wait_is_held_behind_it() {
    let b = board().await;
    let (agent, si) = waiting(&b).await;
    let second = b.watcher.compose_into(agent.id, "and the second", &[]).await;
    assert_eq!(second.expect("held"), Turn::During);
    assert!(b.svc.store.holds_compose(agent.id).unwrap());
    si.show("shells").await;
    super::logs(&si, "SUBMIT and the second").await;
    assert_eq!(si.submitted(), ["what did they find", "and the second"], "{}", si.log());
}

// ---- ov-816: waits long enough for the owner's orchestrator's turns ----

/// A message waiting out a turn `mode` claude began after its paste, with
/// no fence to be had, for 10 s: answered Queued, and the pane's rows open.
async fn waiting_through(b: &Board, mode: &'static str) -> (Terminal, StandIn) {
    let (agent, si) = unfenceable_claude(b).await;
    quick_turns(b, &si);
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(10));
    drop(hook_on_submit(b, &si));
    rows_open(b, agent.id);
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook(mode)));
    assert_eq!(compose_promptly(b, &agent, "what did they find").await.expect("queued"), Turn::During);
    (agent, si)
}

/// Wait, up to 15 seconds, for every wait to have ended: its row in the
/// store goes only then (`pending::Settle`).
async fn waits_over(b: &Board) {
    for _ in 0..750 {
        if b.svc.store.compose_waits().unwrap().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a wait never ended");
}

/// The Queued rows a wait shows, waiting still (`held:pending:<token>`).
fn waiting_rows(terminal: Uuid) -> Vec<String> {
    held_rows(terminal).into_iter().filter(|(_, state)| *state == QueuedState::Waiting).map(|(text, _)| text).collect()
}

/// A key someone types in Far Cooler's terminal, input-marked as the Mac's
/// and the phone's keys are (`Runtime::send_bytes_hex`).
async fn typed_marked(b: &Board, agent: &Terminal, hex: &str) {
    b.svc.runtime().send_bytes_hex(agent.id, hex).await.unwrap();
}

/// The stand-in's process, as the gate proves it (`START <pid>`).
fn stand_in_pid(si: &StandIn) -> i32 {
    si.log().lines().find_map(|l| l.strip_prefix("START ")).and_then(|pid| pid.trim().parse().ok()).expect("started")
}

/// Prerequisite 1: a permission dialog drawn while the message waits is
/// waited out, not given up on, and the key that answers it (marked, as
/// the phone's is) is no typing in the box. Neither Enter goes while it's
/// up: the one Enter is the plain one, after the turn ends.
#[tokio::test]
async fn a_dialog_during_the_wait_is_waited_out_and_its_answer_is_no_typing() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    si.show("menu").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    typed_marked(&b, &agent, "31").await;
    super::logs(&si, "ANSWERED 1").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    si.show("working").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(enters(&si).is_empty(), "no Enter while the dialog or the turn ran: {}", si.log());
    assert_eq!(waiting_rows(agent.id), ["what did they find"]);
    si.show("shells").await;
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    let log = si.log();
    assert!(log.find("ENTER").unwrap() > log.rfind("MODE shells").unwrap(), "{log}");
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
}

/// Prerequisite 1, at the first try: a dialog drawn between the paste and
/// the Enter, in a claude whose hooks would give a fence, is waited out and
/// answered Queued, never refused, and the fenced Enter never tried on it.
#[tokio::test]
async fn a_dialog_drawn_before_the_enter_is_waited_out() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    quick_turns(&b, &si);
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(10));
    si.show("shells").await;
    b.screen_with(agent.id, "2 shells · ↓ to manage").await;
    let _hook = hook_on_submit(&b, &si);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("menu")));
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    si.show("shells").await;
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
}

/// A dialog still up when the wait runs out: the rows say Claude was still
/// asking, never that it stayed busy or that the message is in its box.
#[tokio::test]
async fn a_dialog_up_past_the_wait_is_said_so() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    b.watcher.with_timing(|t| t.turn_begun_wait = WAIT);
    si.show("menu").await;
    waits_over(&b).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    let want = "Not sent: “what did they find”. Claude was still asking you something, so the runner didn’t send it. Check the terminal.";
    assert_eq!(not_sent(agent.id), [want]);
}

/// Prerequisite 2: a wait reads claude's process environment never (the
/// config directory is the gate's), and past `wait_slows_after` reads the
/// pane every `wait_poll`, not every `PASTE_POLL`.
#[tokio::test]
async fn a_long_wait_reads_claudes_process_once_and_slows() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "shells-busy").await;
    b.watcher.with_timing(|t| (t.wait_slows_after, t.wait_poll) = (Duration::from_millis(300), Duration::from_secs(1)));
    let pid = stand_in_pid(&si);
    let (env, tries) = (super::mid_turn::env_reads(pid), super::compose::tries(agent.id));
    tokio::time::sleep(Duration::from_millis(3_600)).await;
    let (env, tries) = (super::mid_turn::env_reads(pid) - env, super::compose::tries(agent.id) - tries);
    assert_eq!(env, 0, "the wait read claude's environment {env} times");
    assert!((2..=8).contains(&tries), "the wait read the pane {tries} times in 3.6 s");
    assert!(enters(&si).is_empty(), "{}", si.log());
}

/// The gate saw background agents; a turn ran, and once it's over the
/// screen reads between turns beside shells alone, a tall box hiding the
/// agent rows, the registry `busy`: for the agents, or for a turn their
/// end began (ov-808 re-review, 3c).
async fn agents_hidden_after_a_turn(b: &Board, carry: Duration) -> (Terminal, StandIn) {
    let (agent, si) = unfenceable_claude(b).await;
    quick_turns(b, &si);
    b.watcher.with_timing(|t| (t.turn_begun_wait, t.carry_agents_for) = (Duration::from_secs(4), carry));
    rows_open(b, agent.id);
    si.show("agents-shells").await;
    b.screen_with(agent.id, "◯ general-purpose").await;
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working-background")));
    assert_eq!(compose_promptly(b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    assert!(enters(&si).is_empty(), "{}", si.log());
    si.show("agents-shells").await;
    (agent, si)
}

/// Prerequisite 3, while the gate's agents are fresh: `busy` beside them is
/// theirs, and the message is entered between turns.
#[tokio::test]
async fn the_gates_agents_are_trusted_while_fresh() {
    let b = board().await;
    let (_agent, si) = agents_hidden_after_a_turn(&b, Duration::from_secs(30)).await;
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
}

/// Prerequisite 3, past `carry_agents_for`: the gate's agents count for
/// nothing, only rows on the screen do, so a `busy` beside shells alone is
/// a turn and the message waits, never entered, until the wait runs out.
#[tokio::test]
async fn the_gates_agents_are_not_trusted_past_their_time() {
    let b = board().await;
    let (sink, _logging) = logging();
    let (agent, si) = agents_hidden_after_a_turn(&b, Duration::from_millis(500)).await;
    let said = logs_line(&sink, "a paste was left in the box").await;
    assert!(said.contains("check=\"registry_busy\"") && said.contains("gate=Agents") && said.contains("background=Shells"), "{said}");
    assert!(enters(&si).is_empty(), "{}", si.log());
    assert!(not_sent(agent.id).iter().any(|n| n.contains("Claude stayed busy")), "{:?}", not_sent(agent.id));
}

/// Prerequisite 4: the box cleared straight in tmux mid-turn is said at
/// once, not when the turn ends, and in words of its own.
#[tokio::test]
async fn a_box_cleared_mid_turn_is_said_at_once() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    b.watcher.with_timing(|t| t.turn_begun_wait = Duration::from_secs(30));
    let began = std::time::Instant::now();
    si.show("working-cleared").await;
    waits_over(&b).await;
    // The transcript is read for `compose_confirm` (5 s) first, in case it
    // was sent by hand; the turn would run 30.
    assert!(began.elapsed() < Duration::from_secs(12), "said only after {:?}", began.elapsed());
    assert!(enters(&si).is_empty(), "{}", si.log());
    let want = "Claude’s box was cleared while “what did they find” waited, so the runner didn’t send it.";
    assert_eq!(not_sent(agent.id), [want]);
    assert!(waiting_rows(agent.id).is_empty());
}

/// Prerequisite 4: the message submitted from the Mac while it waits (a
/// marked Return, claude queues it) is no "not sent": its Queued row goes
/// to claude's own, and the runner presses nothing.
#[tokio::test]
async fn a_message_submitted_from_the_mac_during_the_wait_is_not_said_unsent() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    typed_marked(&b, &agent, "0d").await;
    super::logs(&si, "QUEUED what did they find").await;
    waits_over(&b).await;
    assert_eq!(enters(&si), ["ENTER"], "only the Mac's: {}", si.log());
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// Prerequisite 4: the same, Return pressed straight in tmux, which leaves
/// no input mark: the box empties mid-turn, and the transcript says why.
#[tokio::test]
async fn a_message_submitted_straight_in_tmux_during_the_wait_is_not_said_unsent() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    typed_unmarked(&b, &agent, "\r").await;
    super::logs(&si, "QUEUED what did they find").await;
    waits_over(&b).await;
    assert_eq!(enters(&si), ["ENTER"], "only the one typed: {}", si.log());
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// Prerequisite 4, N3: a runner stopped mid-wait keeps the wait in the
/// store, and the next start picks it up: still Queued, and entered once
/// the turn ends.
#[tokio::test]
async fn a_wait_cut_short_by_a_restart_is_picked_up_again() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "shells-busy").await;
    b.watcher.stop_waits_for_tests();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(b.svc.store.compose_waits().unwrap().len(), 1, "kept across the stop");
    si.show("shells").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(enters(&si).is_empty(), "nothing waits while the runner is down: {}", si.log());
    b.watcher.spawn_pumps();
    super::logs(&si, "SUBMIT what did they find").await;
    assert_eq!(enters(&si), ["ENTER"], "{}", si.log());
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// Prerequisite 4, N3: sent by hand while the runner was down, the message
/// is never entered again, and never said unsent.
#[tokio::test]
async fn a_message_sent_while_the_runner_was_down_is_not_entered_again() {
    let b = board().await;
    let (agent, si) = waiting_through(&b, "working").await;
    b.watcher.stop_waits_for_tests();
    tokio::time::sleep(Duration::from_millis(400)).await;
    typed_unmarked(&b, &agent, "\r").await;
    super::logs(&si, "QUEUED what did they find").await;
    b.watcher.spawn_pumps();
    waits_over(&b).await;
    si.show("shells").await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(enters(&si), ["ENTER"], "only the one typed: {}", si.log());
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// The card's acceptance (ov-816): a message sent to a claude no hook is
/// heard from (the owner's orchestrator, ov-711), whose turn began after
/// the paste and runs `turn`, raising a permission dialog partway that's
/// answered from the phone, is sent when the turn ends: the plain Enter,
/// once, after the turn, never into the dialog.
async fn a_long_turn_with_a_dialog(turn: Duration, timing: impl FnOnce(&mut crate::watch::Timing)) {
    let b = board().await;
    b.watcher.with_timing(timing);
    let agent = b.agent("Orchestrator", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.watcher.mark_fence_in(PathBuf::from(format!("{}.fence", si.control.display())));
    rows_open(&b, agent.id);
    let shower = si.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || shower.show_in_hook("working")));
    let began = std::time::Instant::now();
    assert_eq!(compose_promptly(&b, &agent, "what did they find").await.expect("queued"), Turn::During);
    tokio::time::sleep(turn * 2 / 5).await;
    si.show("menu").await;
    tokio::time::sleep(turn / 5).await;
    typed_marked(&b, &agent, "31").await;
    super::logs(&si, "ANSWERED 1").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    si.show("working").await;
    tokio::time::sleep(turn.saturating_sub(began.elapsed())).await;
    assert!(enters(&si).is_empty(), "no Enter during the turn: {}", si.log());
    assert_eq!(waiting_rows(agent.id), ["what did they find"]);
    si.show("idle").await;
    super::logs(&si, "SUBMIT what did they find").await;
    let log = si.log();
    assert_eq!(enters(&si), ["ENTER"], "{log}");
    assert!(log.find("ENTER").unwrap() > log.rfind("MODE idle").unwrap(), "{log}");
    waits_over(&b).await;
    assert!(not_sent(agent.id).is_empty(), "{:?}", not_sent(agent.id));
    assert!(waiting_rows(agent.id).is_empty());
}

/// The acceptance, its turn six seconds, the wait's reads slowing and the
/// gate's agents untrusted after one.
#[tokio::test]
async fn a_message_sent_during_a_long_turn_with_a_dialog_is_sent_when_it_ends() {
    let s = Duration::from_secs(1);
    a_long_turn_with_a_dialog(6 * s, |t| (t.told_spacing, t.wait_slows_after, t.carry_agents_for) = (crate::watch::Timing::SHORT, s, s)).await;
}

/// The acceptance as the card has it: a five-minute turn, every wait the
/// daemon's own. Run by hand (`--ignored`): it takes five minutes.
#[tokio::test]
#[ignore = "five minutes long: the card's acceptance at production timing"]
async fn a_message_sent_during_a_five_minute_turn_with_a_dialog_is_sent_when_it_ends() {
    a_long_turn_with_a_dialog(Duration::from_secs(5 * 60), |_| {}).await;
}

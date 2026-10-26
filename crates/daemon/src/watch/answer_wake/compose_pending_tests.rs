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
    b.watcher.with_timing(|t| (t.told_spacing, t.turn_begun_wait) = (crate::watch::Timing::SHORT, WAIT));
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
    assert!(not_sent(agent.id).iter().any(|n| n.contains("left in Claude’s box")), "{:?}", not_sent(agent.id));
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
/// two reads `IDLE_TWICE` apart with nothing else between. No Enter.
#[tokio::test]
async fn one_stray_idle_frame_mid_turn_gets_no_enter() {
    let b = board().await;
    let (agent, si) = unfenceable_claude(&b).await;
    quick_turns(&b, &si);
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
    assert!(not_sent(agent.id).iter().any(|n| n.contains("You stopped Claude")), "{:?}", not_sent(agent.id));
}

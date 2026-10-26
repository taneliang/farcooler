//! ov-808: what a compose does when the read-back before its between-turns
//! Enter finds claude's registry `busy` or its spinner drawn
//! (`compose::enter_between`). Background agents a tall box hides are no
//! turn, and the message is sent; a turn begun since the gate takes it as
//! claude's own queue would, under the fence, or once it ends; only a turn
//! still running past the wait with no fence to be had leaves it.

use super::compose_held_tests::Sink;
use super::compose_tests::{hook_on_submit, idle_claude};
use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// How long a turn begun is waited out here (`Timing::turn_begun_wait`).
const WAIT: Duration = Duration::from_millis(600);

/// Sends spaced sooner, and a turn begun waited out for `WAIT`.
fn quick(b: &Board) {
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
    quick(&b);
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
    quick(&b);
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
    quick(&b);
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
    quick(&b);
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
    quick(&b);
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
    quick(&b);
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

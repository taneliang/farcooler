//! ov-556, ov-557: a message is never pasted into a draft. The gate reads the
//! box, then does slow reads (claude's registry, a witness); a draft typed
//! straight into the terminal in that gap leaves no input mark, so the first
//! paste reads the box once more, last (`compose::paste_checked`). On the
//! board and stand-in of `tests`, through the `before_paste` hook, which
//! draws the draft just where the gap is.

use super::compose_held_tests::logs;
use super::compose_upload_tests::served;
use super::compose_tests::{hook_on_submit, idle_claude};
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// A hook for `before_paste` that types `draft` into the stand-in's box, as a
/// person typing at the raw terminal would, and returns once the stand-in has
/// drawn it and tmux shows it: the paste that follows finds it there.
/// `fired` is set when it ran.
pub(super) fn draw_a_draft(b: &Board, si: &StandIn, agent: Uuid, draft: &str, fired: Arc<AtomicBool>) -> Box<dyn FnOnce() + Send> {
    let (svc, control, log) = (b.svc.clone(), si.control.clone(), si.log.clone());
    let (mode, draft) = (format!("draft:{draft}"), draft.to_string());
    Box::new(move || {
        std::fs::write(&control, &mode).unwrap();
        // Off the test's runtime, which this hook blocks: a runtime of its
        // own reads the pane until the draft is on it.
        std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async {
                    for _ in 0..750 {
                        let drawn = std::fs::read_to_string(&log).unwrap_or_default().contains(&format!("MODE {mode}\n"));
                        if drawn && svc.screen(agent).await.is_ok_and(|(screen, _, _)| screen.contains(&draft)) {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    panic!("the draft never showed");
                });
            });
        });
        fired.store(true, Ordering::SeqCst);
    })
}

/// Wait, up to 15 seconds, until `fired` is set.
async fn fired(fired: &AtomicBool) {
    for _ in 0..750 {
        if fired.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the hook never ran");
}

/// A held message passes the whole gate, and a draft appears in the box
/// before its first paste: nothing is pasted, and the message stays held
/// (ov-556), rather than the draft getting the message in it and the hold
/// being given up.
#[tokio::test]
async fn a_draft_drawn_just_before_the_first_paste_keeps_a_held_message_held() {
    let b = board().await;
    b.watcher.with_timing(|t| (t.held_retry, t.told_spacing) = (Duration::from_millis(100), crate::watch::Timing::SHORT));
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(b.watcher.compose_into(agent.id, "kept", &[]).await.expect("held"), Turn::During);
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, agent.id, "half a thought", ran.clone()));
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    fired(&ran).await;
    // A try in flight ends before the slot is ours; the draft is on the
    // screen for every one after it, so none can paste.
    drop(b.watcher.compose_slot_for_test(agent.id).await);
    assert!(!si.log().contains("PASTE"), "pasted into the draft: {}", si.log());
    assert!(!si.log().contains("QUEUED"), "{}", si.log());
    let hold = b.svc.store.first_compose_hold(agent.id).unwrap().expect("given up, not held");
    assert_eq!(hold.text, "kept");
    // The person sends the draft away; the message goes in after it.
    si.show("working-cleared").await;
    logs(&si, "QUEUED kept").await;
}

/// The same for a send that isn't held: refused as a draft, and nothing is
/// typed, for a draft drawn after the gate read the box empty.
#[tokio::test]
async fn a_draft_drawn_just_before_a_sends_first_paste_refuses_it_untyped() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, agent.id, "half a thought", ran.clone()));
    let result = b.watcher.compose_into(agent.id, "hello\nthere", &[]).await;
    assert!(ran.load(Ordering::SeqCst));
    assert_eq!(super::tell_tests::refused_with(result), "draft");
    nothing_typed(&si);
}

/// `terminal send --if-empty` (ov-557) is a `terminal.compose` over the
/// socket: with text already in the box, the runner refuses as `draft` and
/// types nothing, no Return included; with the box clear, it goes in and is
/// Sent. The Mac's Suggest Themes says the refusal with its undelivered
/// sentence.
#[tokio::test]
async fn a_send_if_empty_with_text_in_the_box_types_nothing() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let (_socket, session) = served(&b).await;
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    match session.compose_with_files(agent.id, "Suggest themes for this board", &[], &[]).await {
        Err(farcooler_client::session::SessionError::Refused { what, .. }) => assert_eq!(what, "draft"),
        other => panic!("{other:?}"),
    }
    nothing_typed(&si);
    // The person clears it (`draft:` with nothing in it is the empty box).
    si.show("draft:").await;
    let mut cleared = false;
    for _ in 0..750 {
        if !b.svc.screen(agent.id).await.is_ok_and(|(screen, _, _)| screen.contains("half a thought")) {
            cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(cleared, "the draft never cleared: {}", si.log());
    assert!(!session.compose_with_files(agent.id, "Suggest themes for this board", &[], &[]).await.expect("sent"), "Sent");
    assert_eq!(si.submitted(), ["Suggest themes for this board"], "{}", si.log());
}

/// `terminal tell` pastes past the same final look (ov-556): a draft drawn
/// after its gate read the box empty gets nothing pasted, and is refused as
/// `draft`.
#[tokio::test]
async fn a_draft_drawn_just_before_a_tells_paste_refuses_it_untyped() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, orchestrator.id, "half a thought", ran.clone()));
    let result = b.watcher.tell_into(orchestrator.id, "land ov-214", crate::watch::answer_wake::route::Author::Person).await;
    assert!(ran.load(Ordering::SeqCst));
    assert_eq!(super::tell_tests::refused_with(result), "draft");
    nothing_typed(&si);
}

/// An answer wake pastes past the same final look (ov-556): a draft drawn
/// after its gate read the box empty gets nothing pasted, the answer is not
/// claimed or settled: it stays pending for a later pass.
#[tokio::test]
async fn a_draft_drawn_just_before_an_answers_paste_makes_it_wait() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let ran = Arc::new(AtomicBool::new(false));
    *b.watcher.before_paste.lock().unwrap() = Some(draw_a_draft(&b, &si, agent.id, "half a thought", ran.clone()));
    b.answer("Drill in");
    b.pump().await;
    assert!(ran.load(Ordering::SeqCst), "the hook never ran");
    nothing_typed(&si);
    assert_eq!(b.pending().len(), 1, "settled or claimed: {:?}", b.pending());
}

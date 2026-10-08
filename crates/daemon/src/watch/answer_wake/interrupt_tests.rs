//! Stop and Send Now (ov-368): one Esc, or ctrl+x ctrl+s, pressed in a
//! working claude past every guard, once, and confirmed; and each guard
//! refusing with nothing pressed. The stand-in stops its turn on a lone Esc
//! and runs its queue on ctrl+x ctrl+s, as claude 2.1.290 does, logging
//! `ESC` and `SENDNOW`. On the board and tmux server of `tests`.

use super::interrupt::{Key, LOCKOUT, interrupted_since, sent_from_queue_since, waiting_in_queue};
use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

fn refused(result: Result<()>) -> &'static str {
    match result {
        Err(e) => e.what(),
        Ok(()) => panic!("it was pressed"),
    }
}

/// A claude stand-in working on a turn, its hooks heard from.
async fn working_claude(b: &Board) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.svc.hooks().asks().heard(SESSION, false);
    b.svc.hooks().asks().turn_bounded(SESSION);
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    (agent, si)
}

fn pressed_count(si: &StandIn, key: &str) -> usize {
    si.log().lines().filter(|l| *l == key).count()
}

/// Type `text` and Enter into the stand-in as a person at the terminal would,
/// unmarked, so it's no one typing in Far Cooler: working, it's queued.
async fn queue(b: &Board, agent: &Terminal, si: &StandIn, text: &str) {
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    let paste: String = crate::pastes::encode_paste(true, text).iter().map(|b| format!("{b:02x}")).collect();
    runtime.send_bytes_hex(agent.id, &paste).await.unwrap();
    runtime.send_bytes_hex(agent.id, "0d").await.unwrap();
    for _ in 0..750 {
        if si.log().contains(&format!("QUEUED {text}")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never queued: {}", si.log());
}

fn transcript(si: &StandIn) -> std::path::PathBuf {
    let config = si.control.parent().unwrap().join("config").join("projects");
    let project = std::fs::read_dir(&config).unwrap().next().unwrap().unwrap().path();
    project.join("stand-in.jsonl")
}

/// Working: one Esc, logged once, and confirmed by the interrupted record.
#[tokio::test]
async fn one_esc_stops_a_working_claude_and_is_confirmed() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped");
    assert_eq!(pressed_count(&si, "ESC"), 1, "{}", si.log());
    assert!(interrupted_since(&transcript(&si), 0), "the stand-in recorded it");
}

/// Between turns there's nothing to stop: no Esc, which on a draft would be
/// the first of the two that clear it.
#[tokio::test]
async fn nothing_is_pressed_between_turns() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    si.show("idle").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "idle");
    assert_eq!(refused(b.watcher.press(agent.id, Key::SendNow).await), "idle");
    assert!(!si.log().contains("ESC") && !si.log().contains("SENDNOW"), "{}", si.log());
}

/// A dialog the registry says is up (`waiting`), drawn or not yet, and one
/// the screen shows while the registry still says busy: an Esc would answer
/// it No. Nothing.
#[tokio::test]
async fn nothing_is_pressed_on_a_dialog() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    si.show("menu").await;
    b.screen_with(agent.id, "Tab to amend").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt", "the registry's waiting");
    si.show("working-waiting").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt", "not drawn yet");
    si.show("working-menu").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt", "the screen's dialog");
    assert_eq!(refused(b.watcher.press(agent.id, Key::SendNow).await), "prompt");
    assert!(!si.log().contains("ESC") && !si.log().contains("SENDNOW"), "{}", si.log());
}

/// An ask held on the pane is a dialog claude is waiting to draw.
#[tokio::test]
async fn nothing_is_pressed_while_an_ask_is_held() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let _held = b.svc.hooks().asks().hold(agent.id);
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// A key typed in the pane in the last three seconds: the person is there.
#[tokio::test]
async fn nothing_is_pressed_just_after_someone_typed() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, now_millis().to_string()).unwrap();
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "typing");
    assert!(!si.log().contains("ESC"), "{}", si.log());
    std::fs::write(&mark, (now_millis() - 3_000).to_string()).unwrap();
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped, three seconds on");
}

/// A second key inside the lockout is refused, never pressed: two Esc close
/// together clear claude's box.
#[tokio::test]
async fn never_twice_inside_the_lockout() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped");
    si.show("working").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "too_soon");
    assert_eq!(refused(b.watcher.press(agent.id, Key::SendNow).await), "too_soon");
    assert_eq!(pressed_count(&si, "ESC"), 1, "{}", si.log());
    tokio::time::sleep(LOCKOUT).await;
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped again, past the lockout");
    assert_eq!(pressed_count(&si, "ESC"), 2, "{}", si.log());
}

/// A gate begun a moment ago may have a dialog coming: waited out, and the
/// dialog it drew then stops the key.
#[tokio::test]
async fn a_dialog_a_gate_announced_is_waited_for() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    b.svc.hooks().asks().heard(SESSION, true);
    let control = si.control.clone();
    let drawn = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(control, "working-menu").unwrap();
    });
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    drawn.join().unwrap();
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// So may a call begun a moment ago, whose `PermissionRequest` hasn't come.
#[tokio::test]
async fn a_dialog_a_new_call_may_raise_is_waited_for() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    let control = si.control.clone();
    let drawn = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(control, "working-menu").unwrap();
    });
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    drawn.join().unwrap();
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// A dialog drawn while the key waited on the fence: every check runs again
/// under it, and nothing is pressed.
#[tokio::test]
async fn a_dialog_drawn_as_the_fence_is_taken_stops_the_key() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let control = si.control.clone();
    let log = si.log.clone();
    *b.watcher.before_key.lock().unwrap() = Some(Box::new(move || {
        let before = std::fs::read_to_string(&log).unwrap_or_default().matches("MODE working-menu").count();
        std::fs::write(&control, "working-menu").unwrap();
        for _ in 0..750 {
            if std::fs::read_to_string(&log).unwrap_or_default().matches("MODE working-menu").count() > before {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }));
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// Draw claude's dialog in the stand-in `after` from now, as one announced
/// by a gate or a call would be, on a thread of its own.
fn dialog_after(si: &StandIn, after: Duration) -> std::thread::JoinHandle<()> {
    let control = si.control.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        std::fs::write(control, "working-menu").unwrap();
    })
}

/// A gate heard after the first checks, as the fence is taken: waited out
/// under the fence, and the dialog it drew stops the key (review F1).
#[tokio::test]
async fn a_gate_heard_after_the_first_checks_is_waited_out_under_the_fence() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let asks = b.svc.hooks().asks().clone();
    let si_control = StandIn { control: si.control.clone(), log: si.log.clone() };
    let drawn = std::sync::Arc::new(std::sync::Mutex::new(None));
    let handle = drawn.clone();
    *b.watcher.before_key.lock().unwrap() = Some(Box::new(move || {
        asks.heard(SESSION, true);
        *handle.lock().unwrap() = Some(dialog_after(&si_control, Duration::from_millis(300)));
    }));
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    drawn.lock().unwrap().take().unwrap().join().unwrap();
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// So is a call marked after the first checks, before the fence.
#[tokio::test]
async fn a_call_begun_after_the_first_checks_is_waited_out_under_the_fence() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let asks = b.svc.hooks().asks().clone();
    let si_control = StandIn { control: si.control.clone(), log: si.log.clone() };
    let drawn = std::sync::Arc::new(std::sync::Mutex::new(None));
    let handle = drawn.clone();
    *b.watcher.before_key.lock().unwrap() = Some(Box::new(move || {
        asks.mark_tool_starting(SESSION, Some("toolu_1"));
        *handle.lock().unwrap() = Some(dialog_after(&si_control, Duration::from_millis(300)));
    }));
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "prompt");
    drawn.lock().unwrap().take().unwrap().join().unwrap();
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// An agent calling tools as fast as it can: each new call waits on the
/// fence, so the wait ends and the key goes (review F3).
#[tokio::test]
async fn stop_lands_while_calls_keep_coming() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let asks = b.svc.hooks().asks().clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (calls, until) = (asks.clone(), stop.clone());
    let caller = std::thread::spawn(move || {
        let mut n = 0;
        while !until.load(std::sync::atomic::Ordering::SeqCst) && n < 100 {
            calls.mark_tool_starting(SESSION, Some(&format!("toolu_{n}")));
            n += 1;
            std::thread::sleep(Duration::from_millis(150));
        }
    });
    let pressed = b.watcher.press(agent.id, Key::Interrupt).await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    caller.join().unwrap();
    pressed.expect("stopped");
    assert_eq!(pressed_count(&si, "ESC"), 1, "{}", si.log());
}

/// A gate heard again and again keeps a dialog maybe coming: past the time
/// the fence may be held, its own word, never `prompt` (review F3).
#[tokio::test]
async fn a_dialog_that_keeps_maybe_coming_is_settling() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let asks = b.svc.hooks().asks().clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (gates, until) = (asks.clone(), stop.clone());
    let gating = std::thread::spawn(move || {
        while !until.load(std::sync::atomic::Ordering::SeqCst) {
            gates.heard(SESSION, true);
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let pressed = b.watcher.press(agent.id, Key::Interrupt).await;
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    gating.join().unwrap();
    assert_eq!(refused(pressed), "settling");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// A confirmed Stop ends the main thread's calls in flight, which no hook
/// would (measured): a mid-turn send isn't held `busy` for the rest of the
/// next turn. A subagent's is left (review F5).
#[tokio::test]
async fn a_confirmed_stop_ends_the_calls_it_killed() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    let asks = b.svc.hooks().asks().clone();
    asks.mark_tool_starting(SESSION, Some("toolu_main"));
    asks.mark_call(SESSION, Some("toolu_sub"), Some("a1"));
    b.watcher.press(agent.id, Key::Interrupt).await.expect("stopped");
    assert_eq!(pressed_count(&si, "ESC"), 1, "{}", si.log());
    assert_eq!(asks.calls_for_tests(SESSION), ["toolu_sub"]);
}

/// Checks under the fence that run past `UNDER_FENCE` (a loaded runner):
/// `settling`, nothing pressed, so a late key never outlives the fence.
#[tokio::test]
async fn checks_under_the_fence_that_overrun_are_settling() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    b.watcher.slow_recheck_ms.store(3_500, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "settling");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// A session this daemon never heard a hook from has no fence: nothing.
#[tokio::test]
async fn a_session_never_heard_from_is_unconfirmable() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "unconfirmable");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// codex: none of its keys was measured.
#[tokio::test]
async fn codex_is_unsupported() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    si.show("working").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::Interrupt).await), "unsupported");
    assert!(!si.log().contains("ESC"), "{}", si.log());
}

/// Send Now: the queued messages go now, once, confirmed by claude running
/// them from its queue.
#[tokio::test]
async fn send_now_sends_what_waits_in_the_queue() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::SendNow).await), "nothing_queued");
    assert!(!si.log().contains("SENDNOW"), "{}", si.log());
    queue(&b, &agent, &si, "and the docs").await;
    queue(&b, &agent, &si, "then land it").await;
    b.screen_with(agent.id, "Press up to edit queued messages").await;
    b.watcher.press(agent.id, Key::SendNow).await.expect("sent now");
    assert_eq!(pressed_count(&si, "SENDNOW"), 1, "{}", si.log());
    assert_eq!(si.submitted(), ["and the docs", "then land it"], "{}", si.log());
}

/// Send Now over a draft: claude would submit the draft too, so nothing.
#[tokio::test]
async fn send_now_never_goes_over_a_draft() {
    let b = board().await;
    let (agent, si) = working_claude(&b).await;
    queue(&b, &agent, &si, "and the docs").await;
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    runtime.send_bytes_hex(agent.id, &"half".bytes().map(|b| format!("{b:02x}")).collect::<String>()).await.unwrap();
    b.screen_with(agent.id, "half").await;
    assert_eq!(refused(b.watcher.press(agent.id, Key::SendNow).await), "draft");
    assert!(!si.log().contains("SENDNOW"), "{}", si.log());
}

/// What the transcript says: a message waits from its `enqueue` until a
/// `dequeue`, an absorbing `remove` or a `popAll`; claude's own notices
/// aren't a person's; and what confirms each key.
#[test]
fn the_transcript_says_what_waits_and_what_was_taken() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let write = |lines: &[&str]| std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let enqueue = |text: &str| format!(r#"{{"type":"queue-operation","operation":"enqueue","content":"{text}"}}"#);
    write(&[&enqueue("a")]);
    assert!(waiting_in_queue(&path));
    write(&[&enqueue("a"), r#"{"type":"queue-operation","operation":"dequeue"}"#]);
    assert!(!waiting_in_queue(&path));
    write(&[&enqueue("a"), &enqueue("b"), r#"{"type":"queue-operation","operation":"remove","content":"a"}"#]);
    assert!(waiting_in_queue(&path), "b waits");
    write(&[&enqueue("a"), r#"{"type":"queue-operation","operation":"popAll"}"#]);
    assert!(!waiting_in_queue(&path));
    write(&[&enqueue("<task-notification>done</task-notification>")]);
    assert!(!waiting_in_queue(&path), "claude's own");

    write(&[r#"{"type":"user","message":{"role":"user","content":"[Request interrupted by user]"}}"#]);
    assert!(interrupted_since(&path, 0));
    let tool = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
    write(&[tool]);
    assert!(interrupted_since(&path, 0));
    assert!(!interrupted_since(&path, std::fs::metadata(&path).unwrap().len()), "only past the mark");
    write(&[r#"{"type":"user","message":{"role":"user","content":"[Request interrupted] is what I typed"}}"#]);
    assert!(!interrupted_since(&path, 0));

    write(&[r#"{"type":"queue-operation","operation":"remove","content":"a","reason":"absorbed_mid_turn"}"#]);
    assert!(sent_from_queue_since(&path, 0));
    write(&[r#"{"type":"queue-operation","operation":"remove","content":"a"}"#]);
    assert!(!sent_from_queue_since(&path, 0), "taken back to the box, not sent");
    write(&[r#"{"type":"user","message":{"role":"user","content":"a"},"promptSource":"queued"}"#]);
    assert!(sent_from_queue_since(&path, 0));
    write(&[r#"{"type":"user","message":{"role":"user","content":"a"},"promptSource":"typed"}"#]);
    assert!(!sent_from_queue_since(&path, 0));
}

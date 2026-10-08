//! Bring Here (ov-369): the draft in claude's box read with its line breaks
//! and nothing typed; cleared only while it reads as read, a row at a time;
//! put back when anything else shows; and each refusal with no key pressed.
//! The stand-in draws claude 2.1.292's box (`STAND_IN_DRAFT`): wrapped at
//! the pane's width less 4, its cursor after the text, a tall box windowed,
//! ctrl+u and ctrl+y logged as `CLEARROW` and `PUTBACK`. On the board and
//! tmux server of `tests`.

use super::*;

fn refused(result: Result<(String, bool)>) -> &'static str {
    match result {
        Err(e) => e.what(),
        Ok(ok) => panic!("it went through: {ok:?}"),
    }
}

fn keys(si: &StandIn) -> usize {
    si.log().lines().filter(|l| *l == "CLEARROW" || *l == "PUTBACK").count()
}

/// A claude stand-in drawing Bring Here's box, `env` added, its box holding
/// `draft`.
async fn drafted(b: &Board, env: &str, draft: &str) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", &format!("STAND_IN_DRAFT=1 {env}")).await;
    si.show(&format!("draft:{draft}")).await;
    // Read as printed: a moved cursor's reverse-video cell sits inside the
    // last line's escapes, which a raw match would never find.
    let last = draft.lines().last().unwrap_or_default().to_string();
    for _ in 0..600 {
        let screen = b.svc.screen(agent.id).await.expect("a screen").0;
        if farcooler_core::composer::printed(&screen).contains(&last) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    (agent, si)
}

/// The box as the daemon reads it now.
async fn box_now(b: &Board, agent: &Terminal) -> farcooler_core::composer::Composer {
    let (screen, _, _) = b.svc.screen(agent.id).await.unwrap();
    farcooler_core::composer::read("claude", &screen)
}

/// Read: the text with its line breaks, nothing typed.
#[tokio::test]
async fn a_read_answers_the_draft_and_types_nothing() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "Fix the login bug first.\n\nThen the tests").await;
    let read = b.watcher.bring_draft(agent.id, None).await.expect("read");
    assert_eq!(read, ("Fix the login bug first.\n\nThen the tests".to_string(), false));
    assert_eq!(keys(&si), 0, "{}", si.log());
    assert!(matches!(box_now(&b, &agent).await, farcooler_core::composer::Composer::Holds(_)));
}

/// Clear: the box emptied a row at a time, nothing put back, and the
/// clear remembered for the typing window.
#[tokio::test]
async fn a_clear_empties_the_box_it_read() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "first line\nsecond line\nthird").await;
    let (text, _) = b.watcher.bring_draft(agent.id, None).await.expect("read");
    let cleared = b.watcher.bring_draft(agent.id, Some(text.clone())).await.expect("cleared");
    assert_eq!(cleared, (text, true));
    assert_eq!(box_now(&b, &agent).await, farcooler_core::composer::Composer::Empty);
    assert!(si.log().contains("CLEARROW") && !si.log().contains("PUTBACK"), "{}", si.log());
    assert!(!si.log().contains("ESC") && !si.log().contains("ENTER"), "{}", si.log());
}

/// An empty box: nothing to bring, nothing typed.
#[tokio::test]
async fn an_empty_box_brings_nothing() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", "STAND_IN_DRAFT=1").await;
    assert_eq!(b.watcher.bring_draft(agent.id, None).await.expect("read"), (String::new(), false));
    assert_eq!(b.watcher.bring_draft(agent.id, Some("x".into())).await.expect("read"), (String::new(), false));
    assert_eq!(keys(&si), 0, "{}", si.log());
}

/// A box that no longer reads as the client read it is never cleared.
#[tokio::test]
async fn a_changed_box_is_left_alone() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "what I typed").await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some("what I typed before".into())).await), "changed");
    assert_eq!(keys(&si), 0, "{}", si.log());
}

/// A key typed through a client in the last three seconds: the person is
/// there. Three seconds on, it clears.
#[tokio::test]
async fn a_key_just_now_holds_the_clear() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "half a thought").await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, now_millis().to_string()).unwrap();
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some("half a thought".into())).await), "typing");
    assert_eq!(keys(&si), 0, "{}", si.log());
    std::fs::write(&mark, (now_millis() - 3_000).to_string()).unwrap();
    b.watcher.bring_draft(agent.id, Some("half a thought".into())).await.expect("cleared, three seconds on");
}

/// A key typed mid-clear stops it, and ctrl+y puts the draft back.
#[tokio::test]
async fn a_key_mid_clear_puts_it_back() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "one\ntwo\nthree").await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    // After the first ctrl+u, before the second.
    *b.watcher.before_clear_key.lock().unwrap() = Some(Box::new(move |n| {
        let mark = mark.clone();
        Box::pin(async move {
            if n == 1 {
                std::fs::write(&mark, now_millis().to_string()).unwrap();
            }
        })
    }));
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some("one\ntwo\nthree".into())).await), "typing");
    assert!(si.log().contains("PUTBACK"), "{}", si.log());
    assert!(farcooler_core::composer::holds_exactly(&box_now(&b, &agent).await, "one two three"));
}

/// A box claude windows at fewer rows than this daemon expects (a later
/// claude): the hidden rows scroll in as the shown ones go, which isn't the
/// draft read, so it's all put back and refused.
#[tokio::test]
async fn hidden_rows_scrolling_in_put_it_back() {
    let b = board().await;
    let (agent, si) = drafted(&b, "STAND_IN_WINDOW=2", "line one\nline two\nline three\nline four").await;
    let (text, _) = b.watcher.bring_draft(agent.id, None).await.expect("read");
    assert_eq!(text, "line three\nline four", "only the window shows");
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some(text)).await), "too_tall");
    assert!(si.log().contains("PUTBACK"), "{}", si.log());
    assert!(farcooler_core::composer::holds_exactly(&box_now(&b, &agent).await, "line three line four"));
}

/// A box as tall as claude draws one may be hiding rows: refused before
/// any key.
#[tokio::test]
async fn a_box_at_claudes_height_is_refused() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", "STAND_IN_DRAFT=1").await;
    let (_, _, rows) = b.svc.screen(agent.id).await.unwrap();
    let most = farcooler_core::composer::draft::max_rows(rows);
    assert!(most >= 1, "a pane {rows} rows tall");
    let draft: Vec<String> = (1..=most + 3).map(|n| format!("row {n}")).collect();
    si.show(&format!("draft:{}", draft.join("\n"))).await;
    b.screen_with(agent.id, &format!("row {}", most + 3)).await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, None).await), "too_tall");
    assert_eq!(keys(&si), 0, "{}", si.log());
}

/// A collapsed paste or an image: the screen hasn't got its content.
#[tokio::test]
async fn a_placeholder_is_refused() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "[Pasted text #1 +3 lines] and more").await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, None).await), "pasted");
    assert_eq!(keys(&si), 0, "{}", si.log());
}

/// claude's cursor moved back into the text: a ctrl+u would leave what's
/// after it, so nothing is read.
#[tokio::test]
async fn a_moved_cursor_is_refused() {
    let b = board().await;
    let (agent, si) = drafted(&b, "STAND_IN_CURSOR_BACK=4", "a draft to edit").await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, None).await), "cursor");
    assert_eq!(keys(&si), 0, "{}", si.log());
}

/// A dialog, and codex: nothing read, nothing typed.
#[tokio::test]
async fn a_dialog_and_codex_are_refused() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "a draft").await;
    si.show("menu").await;
    b.screen_with(agent.id, "Tab to amend").await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, None).await), "prompt");
    assert_eq!(keys(&si), 0, "{}", si.log());
    let codex = b.agent("Agent 3", "codex").await;
    let cx = b.stand_in(&codex, "codex", "codex").await;
    cx.show("draft:a draft").await;
    b.screen_with(codex.id, "a draft").await;
    assert_eq!(refused(b.watcher.bring_draft(codex.id, None).await), "unsupported");
}

/// Mid-turn, the same: no Esc, the turn left running.
#[tokio::test]
async fn a_draft_mid_turn_is_cleared_without_stopping_the_turn() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in_with(&agent, "claude", "claude", "STAND_IN_DRAFT=1").await;
    si.show("working").await;
    let runtime = crate::runtime::Runtime { marks: None, ..b.svc.runtime() };
    runtime.send_bytes_hex(agent.id, &"next: the logout path".bytes().map(|c| format!("{c:02x}")).collect::<String>()).await.unwrap();
    b.screen_with(agent.id, "logout path").await;
    let (text, _) = b.watcher.bring_draft(agent.id, None).await.expect("read");
    assert_eq!(text, "next: the logout path");
    b.watcher.bring_draft(agent.id, Some(text)).await.expect("cleared");
    assert!(!si.log().contains("ESC"), "{}", si.log());
    assert_eq!(std::fs::read_to_string(&si.control).unwrap(), "working");
}

/// After a clear, keys from before it no longer hold a send: they made the
/// draft the composer holds now. A key after it does.
#[tokio::test]
async fn keys_before_a_clear_no_longer_hold_a_send() {
    let b = board().await;
    let (agent, _si) = drafted(&b, "", "typed a moment ago").await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, (now_millis() - 3_000).to_string()).unwrap();
    assert!(b.watcher.typed_lately(agent.id, now_millis()), "three seconds ago holds a send");
    b.watcher.bring_draft(agent.id, Some("typed a moment ago".into())).await.expect("cleared");
    assert!(!b.watcher.typed_lately(agent.id, now_millis()), "the keys that made the draft");
    std::fs::write(&mark, (now_millis() + 1).to_string()).unwrap();
    assert!(b.watcher.typed_lately(agent.id, now_millis() + 2), "a key after the clear");
}

/// Keys typed straight into tmux (no client mark) between a read and the
/// next ctrl+u would go with the row: the box is read again first, and the
/// clear stops, with what it took put back.
#[tokio::test]
async fn keys_typed_between_a_read_and_the_next_key_are_not_deleted() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "one\ntwo\nthree").await;
    let id = agent.id;
    let svc = b.svc.clone();
    *b.watcher.before_clear_key.lock().unwrap() = Some(Box::new(move |n| {
        let runtime = crate::runtime::Runtime { marks: None, ..svc.runtime() };
        let svc = svc.clone();
        Box::pin(async move {
            if n == 1 {
                runtime.send_bytes_hex(id, "7a7a").await.unwrap();
                for _ in 0..600 {
                    let screen = svc.screen(id).await.expect("a screen").0;
                    if farcooler_core::composer::printed(&screen).contains("zz") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        })
    }));
    // The typed "zz" is in the box, so the put-back can't be proven whole.
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some("one\ntwo\nthree".into())).await), "partly");
    assert_eq!(si.log().lines().filter(|l| *l == "CLEARROW").count(), 1, "no second ctrl+u: {}", si.log());
    let (screen, _, _) = b.svc.screen(agent.id).await.unwrap();
    assert!(farcooler_core::composer::printed(&screen).contains("zz"), "the typed keys are still there");
}

/// Hidden rows that read the same once whitespace is squeezed (here, all
/// alike) scroll in as the shown ones go: the box keeps drawing as many
/// rows, so it isn't the draft being cleared.
#[tokio::test]
async fn hidden_rows_that_look_alike_scrolling_in_put_it_back() {
    let b = board().await;
    let (agent, si) = drafted(&b, "STAND_IN_WINDOW=2", "x\nx\nx\nx").await;
    let (text, _) = b.watcher.bring_draft(agent.id, None).await.expect("read");
    assert_eq!(text, "x\nx");
    assert_eq!(refused(b.watcher.bring_draft(agent.id, Some(text)).await), "too_tall");
    assert!(si.log().contains("PUTBACK"), "{}", si.log());
}

/// A dialog that comes up mid-clear: ctrl+y isn't pressed there.
#[tokio::test]
async fn a_dialog_mid_clear_is_not_pasted_into() {
    let b = board().await;
    let (agent, si) = drafted(&b, "", "one\ntwo\nthree").await;
    let (control, log, id, svc) = (si.control.clone(), si.log.clone(), agent.id, b.svc.clone());
    *b.watcher.before_clear_key.lock().unwrap() = Some(Box::new(move |n| {
        let show = StandIn { control: control.clone(), log: log.clone() };
        let svc = svc.clone();
        Box::pin(async move {
            if n == 1 {
                show.show("menu").await;
                for _ in 0..300 {
                    if svc.screen(id).await.expect("a screen").0.contains("Tab to amend") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        })
    }));
    let word = refused(b.watcher.bring_draft(agent.id, Some("one\ntwo\nthree".into())).await);
    assert_eq!(word, "partly");
    assert!(!si.log().contains("PUTBACK"), "{}", si.log());
}

/// Vim's insert mode gives ctrl+y another meaning: refused before any key.
#[tokio::test]
async fn vim_insert_mode_is_refused() {
    let b = board().await;
    let (agent, si) = drafted(&b, "STAND_IN_VIM=INSERT", "a draft").await;
    assert_eq!(refused(b.watcher.bring_draft(agent.id, None).await), "unfamiliar");
    assert_eq!(keys(&si), 0, "{}", si.log());
}

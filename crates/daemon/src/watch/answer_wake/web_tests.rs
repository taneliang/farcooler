//! A web pane is never typed to (ov-435, review 1 M7). Its process only
//! holds a rectangle the Mac draws a page into, so a message, a composed
//! prompt, a draft or a keypress pasted at it would land in a program that
//! isn't there. Each of these panes is made to look like a typable agent in
//! every other way (an adopted orchestrator whose process is a claude), so
//! that only `PaneMode::is_client_drawn` stands between it and the keys. On
//! the board and tmux server of `tests`.

use super::interrupt::Key;
use super::*;

#[tokio::test]
async fn nothing_is_typed_into_a_web_pane_whatever_it_runs() {
    let b = board().await;
    let orchestrator = b.adopted_shell().await;
    let si = b.stand_in(&orchestrator, "claude", "claude").await;
    b.doing(orchestrator.id, AgentActivity::Idle).await;
    let now = b.svc.store.get_terminal(orchestrator.id).unwrap();
    b.svc
        .store
        .set_pane_mode(now.id, now.resource_version, farcooler_store::models::PaneMode::Web, None, false)
        .unwrap();

    assert_eq!(super::tell_tests::refused_with(b.watcher.tell_into(orchestrator.id, "hello").await), "not_an_agent");
    let composed = b.watcher.compose_into(orchestrator.id, "hello", &[]).await;
    assert_eq!(composed.expect_err("composed into a page").what(), "not_an_agent");
    let drafted = b.watcher.draft_into(orchestrator.id, "About ov-1: ", false).await;
    match drafted {
        Err(e) => assert_eq!(e.what(), "not_pasteable"),
        Ok(_) => panic!("drafted into a page"),
    }
    let pressed = b.watcher.press(orchestrator.id, Key::Interrupt).await;
    assert_eq!(pressed.expect_err("pressed in a page").what(), "not_an_agent");
    nothing_typed(&si);
}

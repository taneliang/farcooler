//! What marks a pane as someone typing, through the input channel the Mac's
//! terminal writes to (`Runtime::input_channel`), and what a message composed
//! for claude mid-turn makes of it (ov-407). On the board and tmux server of
//! `tests`, whose helpers these use.

use super::compose_tests::idle_claude;
use super::tell_tests::refused_with;
use super::*;

/// The lines a click and two notches of the wheel over claude's pane send,
/// SGR-encoded as claude asks: press, release, and both notches in one.
const GESTURES: &str = "1b5b3c303b34313b31334d\n1b5b3c303b34313b31336d\n1b5b3c36343b34313b31334d1b5b3c36343b34313b31334d\n";
/// What the emulator answers claude's questions with as its prompt comes up.
const REPLIES: &str = "1b5b3f35751b5b3f36631b5b3f323032363b322479\n";

/// A click, the wheel and the emulator's replies leave no mark, from the
/// Mac's channel or a phone's `terminal.write`, so a message composed
/// mid-turn is queued; a key from either does, and holds the next one as
/// someone typing. Marking every line, as before, refuses the first; never
/// marking lets a later one through.
#[tokio::test]
async fn a_click_is_not_typing_and_a_key_is() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let runtime = b.svc.runtime();
    let mark = || crate::runtime::last_input(b.svc.root_dir(), agent.id);
    runtime.input_from(agent.id, format!("{GESTURES}{REPLIES}").as_bytes()).await.expect("forwarded");
    b.svc.send_bytes(agent.id, b"\x1b[<64;41;13M\x1b[?6c").await.expect("written");
    assert_eq!(mark(), None, "a gesture or a reply marked the pane");
    assert_eq!(b.watcher.compose_into(agent.id, "after the click", &[]).await.expect("queued"), Turn::During);
    assert!(si.log().contains("QUEUED after the click"), "{}", si.log());
    runtime.input_from(agent.id, "78\n".as_bytes()).await.expect("forwarded");
    assert!(mark().is_some(), "a key on the Mac left no mark");
    assert_eq!(refused_with(b.watcher.compose_into(agent.id, "over the key", &[]).await), "typing");
    std::fs::remove_file(crate::runtime::input_mark(b.svc.root_dir(), agent.id)).unwrap();
    b.svc.send_bytes(agent.id, b"y").await.expect("written");
    assert!(mark().is_some(), "a key on a phone left no mark");
    assert_eq!(refused_with(b.watcher.compose_into(agent.id, "over the key", &[]).await), "typing");
    assert!(!si.log().contains("over the key"), "{}", si.log());
}

//! `terminal.compose` (ov-367): a native composer's message typed into
//! claude's box and submitted, Sent once its `UserPromptSubmit` is heard,
//! Queued once its `enqueue` record is written; and every refusal. The
//! stand-in draws claude 2.1.290's placeholders and command popup; a test
//! says what claude's hook would, from what the stand-in logs it submitted.
//! On the board and tmux server of `tests`, whose helpers these use.

use super::compose::{Composition, composition, normalized};
use super::tell_tests::refused_with;
use super::*;

/// The session every claude stand-in names in its registry.
const SESSION: &str = "stand-in";

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
    0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0, 0, 0, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0, 0,
    0x03, 0x01, 0x01, 0, 0x18, 0xdd, 0x8d, 0xb0, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The session's hooks heard from, a turn boundary since the daemon started.
fn hooked(b: &Board) {
    b.svc.hooks().asks().heard(SESSION, false);
    b.svc.hooks().asks().turn_bounded(SESSION);
}

/// claude's `UserPromptSubmit`, as `hook_ingress` records it, for each prompt
/// the stand-in submits or command it runs, each a turn of its own; and, as
/// claude 2.1.290 does, for each message it queues mid-turn, at once and
/// naming the running turn. What the stand-in logged, its line breaks back.
/// Stops when the test ends.
pub(super) fn hook_on_submit(b: &Board, si: &StandIn) -> tokio::task::JoinHandle<()> {
    let asks = b.svc.hooks().asks().clone();
    let log = si.log.clone();
    tokio::spawn(async move {
        let mut seen = 0;
        loop {
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let lines: Vec<&str> = said.lines().filter(|l| ["SUBMIT ", "COMMAND ", "QUEUED "].iter().any(|k| l.starts_with(k))).collect();
            for (n, line) in lines.iter().enumerate().skip(seen) {
                let (kind, prompt) = line.split_once(' ').unwrap_or_default();
                let turn = if kind == "QUEUED" { "running".to_string() } else { format!("turn-{n}") };
                if kind == "QUEUED" {
                    asks.saw_turn(SESSION, &turn);
                }
                asks.prompted(SESSION, &prompt.replace("\\n", "\n"), Some(&turn));
            }
            seen = lines.len();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
}

async fn idle_claude(b: &Board) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    hooked(b);
    (agent, si)
}

fn refused(result: Result<Turn>) -> &'static str {
    refused_with(result)
}

/// Three lines, idle: pasted once, read back as typed, submitted, and Sent
/// once claude's hook names the prompt.
#[tokio::test]
async fn multi_line_text_is_sent_once_the_hook_names_it() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let turn = b.watcher.compose_into(agent.id, "fix the flaky test\r\nin tests.rs\n\nthen land it\n", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between);
    assert_eq!(si.log().matches("PASTE ").count(), 1, "one paste: {}", si.log());
    assert_eq!(si.submitted(), ["fix the flaky test\\nin tests.rs\\n\\nthen land it"], "{}", si.log());
}

/// The same, with no hook ever saying claude took it: no Sent.
#[tokio::test]
async fn a_send_no_hook_confirms_is_unconfirmed() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "one\ntwo", &[]).await), "unconfirmed");
    si.submits(1).await;
    assert_eq!(si.submitted(), ["one\\ntwo"], "it was typed: {}", si.log());
}

/// A paste past claude's threshold shows as `[Pasted text #N +K lines]`:
/// read back as that, submitted whole, and confirmed by the whole text.
#[tokio::test]
async fn a_long_paste_is_read_back_as_its_placeholder() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let five = "step 1\nstep 2\nstep 3\nstep 4\nstep 5";
    assert_eq!(b.watcher.compose_into(agent.id, five, &[]).await.expect("sent"), Turn::Between);
    b.screen_with(agent.id, "step 1").await;
    let long = "word ".repeat(200);
    assert_eq!(b.watcher.compose_into(agent.id, &long, &[]).await.expect("sent"), Turn::Between);
    si.submits(2).await;
    assert_eq!(si.submitted(), [five.replace('\n', "\\n"), long.trim_end().to_string()], "{}", si.log());
}

/// An image: written to the runner's paste directory, its path pasted on its
/// own and read back as `[Image #N]`, then the text; claude's prompt names
/// the image by that placeholder.
#[tokio::test]
async fn an_image_goes_in_first_as_its_placeholder() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let image = ("image/png".to_string(), PNG.to_vec());
    let turn = b.watcher.compose_into(agent.id, "what color is it", &[image]).await;
    assert_eq!(turn.expect("sent"), Turn::Between);
    assert_eq!(si.submitted(), ["[Image #1] what color is it"], "{}", si.log());
    let pastes = crate::paths::pastes_dir_in(b.svc.root_dir()).unwrap();
    let written: Vec<_> = std::fs::read_dir(pastes).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file()).collect();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(std::fs::read(written[0].path()).unwrap(), PNG);
}

/// A prompt command: its name alone first, and on only when claude's popup
/// highlights exactly it; then its arguments. Alone, Enter runs the
/// highlighted command.
#[tokio::test]
async fn a_prompt_command_goes_in_when_the_popup_highlights_it() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let turn = b.watcher.compose_into(agent.id, "/init focus on the tests", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between);
    assert!(si.log().contains("PASTE /init\n"), "the name alone first: {}", si.log());
    assert_eq!(si.submitted(), ["/init focus on the tests"], "{}", si.log());
    b.screen_with(agent.id, "/init focus").await;
    assert_eq!(b.watcher.compose_into(agent.id, "/init", &[]).await.expect("ran"), Turn::Between);
    assert!(si.log().contains("COMMAND /init"), "{}", si.log());
}

/// A command the popup highlights as another (claude's best match): no
/// Enter, the name left in the box.
#[tokio::test]
async fn a_command_the_popup_reads_as_another_is_not_entered() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "/i", &[]).await), "paste_left");
    assert!(si.log().contains("PASTE /i"), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

/// A panel command, or one of claude's own that acts at once, is the
/// terminal's: nothing typed. So is a command mid-turn, and a shell escape.
#[tokio::test]
async fn a_panel_command_is_handed_off_untyped() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    for panel in ["/model", "/usage", "/config", "/resume", "/agents", "/status", "/cost", "/clear", "/compact now"] {
        assert_eq!(refused(b.watcher.compose_into(agent.id, panel, &[]).await), "handoff", "{panel}");
    }
    assert_eq!(refused(b.watcher.compose_into(agent.id, "!ls", &[]).await), "command");
    assert_eq!(refused(b.watcher.compose_into(agent.id, "/tmp/x is full", &[]).await), "command");
    nothing_typed(&si);
}

/// A draft in the box is refused (R-28); so is a dialog, and a picker the
/// box reader doesn't know: nothing typed.
#[tokio::test]
async fn a_draft_or_a_dialog_is_refused_untyped() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("draft:half a thought").await;
    b.screen_with(agent.id, "half a thought").await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello\nthere", &[]).await), "draft");
    si.show("menu").await;
    b.screen_with(agent.id, "Tab to amend").await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello\nthere", &[]).await), "prompt");
    si.show("picker").await;
    b.screen_with(agent.id, "Select model").await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello\nthere", &[]).await), "unfamiliar");
    nothing_typed(&si);
}

/// Working: the message goes into claude's own queue through the fence, and
/// is Queued once the `enqueue` record holds it, whole, though claude's hook
/// names it at once as it does a turn's prompt. It reaches claude when the
/// turn ends.
#[tokio::test]
async fn busy_is_queued_once_the_enqueue_record_holds_it() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    let five = "later 1\nlater 2\nlater 3\nlater 4\nlater 5";
    assert_eq!(b.watcher.compose_into(agent.id, five, &[]).await.expect("queued"), Turn::During);
    assert!(si.log().contains("QUEUED later 1\\nlater 2"), "{}", si.log());
    assert!(si.submitted().is_empty(), "{}", si.log());
    assert_eq!(refused(b.watcher.compose_into(agent.id, "/init", &[]).await), "busy", "a command waits for the turn");
    si.show("idle").await;
    si.submits(1).await;
    assert_eq!(si.submitted(), [five.replace('\n', "\\n")], "{}", si.log());
}

/// Working with no `enqueue` record written: no Queued, and no Sent for the
/// hook that names the running turn.
#[tokio::test]
async fn busy_with_no_enqueue_record_is_unconfirmed() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working-quiet").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    let _hook = hook_on_submit(&b, &si);
    assert_eq!(refused(b.watcher.compose_into(agent.id, "a\nb", &[]).await), "unconfirmed");
    assert!(si.log().contains("QUEUED a\\nb"), "{}", si.log());
}

/// codex is refused: none of its drawn forms was measured.
#[tokio::test]
async fn codex_is_unsupported() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "unsupported");
    nothing_typed(&si);
}

/// What's typed: line breaks as LF, other controls written out, trailing
/// whitespace and leading blank lines gone; a command split from its
/// arguments; images sniffed.
#[test]
fn a_composition_is_checked_before_anything_is_typed() {
    assert_eq!(normalized("\n \nfix\r\nit\u{1b}[31m\u{202e}\t \n\n"), "fix\nit\\u{1b}[31m\\u{202e}");
    assert_eq!(normalized("  indented\nkept"), "  indented\nkept");
    let init = composition("/init focus\non tests", &[]).unwrap();
    assert_eq!(init, Composition { text: " focus\non tests".into(), command: Some("/init".into()), images: vec![] });
    assert_eq!(composition(" \n", &[]).unwrap_err().what(), "text");
    let not_png = ("image/png".to_string(), b"GIF8".to_vec());
    assert_eq!(composition("x", &[not_png]).unwrap_err().what(), "image");
    let png = ("image/png".to_string(), PNG.to_vec());
    assert_eq!(composition("/init", std::slice::from_ref(&png)).unwrap_err().what(), "images");
    assert_eq!(composition("", &[png]).unwrap().images.len(), 1, "an image alone is a message");
    assert_eq!(composition(&"x".repeat(100_001), &[]).unwrap_err().what(), "too_long");
}

/// A prompt with an image is a list of blocks in claude's transcript, its
/// text in a `text` block (2.1.290): it's read for that text. claude's own
/// line saying where the image came from (`isMeta`) is no prompt.
#[test]
fn a_prompt_with_an_image_is_read_from_its_text_block() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Image #4] what color"},{"type":"image","source":{}}]},"imagePasteIds":[4]}"#,
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"[Image: source: /x.png]"}]}}"#,
    ];
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    assert_eq!(super::mid_turn::recorded_as(&path, 0, "[Image #4]  what color", false), Some("user"));
    assert_eq!(super::mid_turn::recorded_as(&path, 0, "[Image: source: /x.png]", false), None);
}

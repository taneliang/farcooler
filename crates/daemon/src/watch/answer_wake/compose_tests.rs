//! `terminal.compose` (ov-367): a native composer's message typed into
//! claude's box and submitted, Sent once its `UserPromptSubmit` is heard,
//! Queued once its `enqueue` record is written; and every refusal. The
//! stand-in draws claude 2.1.290's placeholders and command popup; a test
//! says what claude's hook would, from what the stand-in logs it submitted.
//! On the board and tmux server of `tests`, whose helpers these use.

use super::compose::{Composition, composition, composition_with, normalized};
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

pub(super) async fn idle_claude(b: &Board) -> (Terminal, StandIn) {
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

/// Working with no `enqueue` record written: claude's hook naming the
/// running turn is what says it was queued, never a Sent. With no hook
/// either, unconfirmed.
#[tokio::test]
async fn busy_with_only_the_hook_is_queued_and_with_nothing_unconfirmed() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working-quiet").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::Working).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "a\nb", &[]).await), "unconfirmed");
    assert!(si.log().contains("QUEUED a\\nb"), "{}", si.log());
    let _hook = hook_on_submit(&b, &si);
    b.screen_with(agent.id, "ress up to edit queued messages").await;
    assert_eq!(b.watcher.compose_into(agent.id, "c\nd", &[]).await.expect("queued"), Turn::During);
}

/// A backslash before Enter is a line break to claude, never a send: refused
/// untyped, a command's arguments too. codex sends it as typed
/// (`compose_codex_tests`).
#[tokio::test]
async fn a_backslash_at_the_end_is_refused_for_claude() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "see C:\\ \n", &[]).await), "backslash");
    assert_eq!(refused(b.watcher.compose_into(agent.id, "/init the docs\\", &[]).await), "backslash");
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
    // Behind leading spaces, still a command or a shell escape.
    assert_eq!(composition("  /init focus", &[]).unwrap().command.as_deref(), Some("/init"));
    assert_eq!(composition(" /model", &[]).unwrap_err().what(), "handoff");
    assert_eq!(composition("\t!ls", &[]).unwrap_err().what(), "command");
    assert_eq!(composition("  plain", &[]).unwrap().text, "  plain", "a prompt keeps its indent");
    assert_eq!(composition(" \n", &[]).unwrap_err().what(), "text");
    let not_png = ("image/png".to_string(), b"GIF8".to_vec());
    assert_eq!(composition("x", &[not_png]).unwrap_err().what(), "image");
    let png = ("image/png".to_string(), PNG.to_vec());
    assert_eq!(composition("/init", std::slice::from_ref(&png)).unwrap_err().what(), "images");
    assert_eq!(composition("", &[png]).unwrap().images.len(), 1, "an image alone is a message");
    assert_eq!(composition(&"x".repeat(100_001), &[]).unwrap_err().what(), "too_long");
    // A backslash at the end is the agent's to judge (`a_backslash_at_the_end_…`).
    assert!(composition("see C:\\ \n", &[]).is_ok());
    assert!(composition("a \\ in the middle", &[]).is_ok());
    // Every image together, past what one compose takes, uploaded first
    // (ov-393): what one request carries is held before this.
    let cap = farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES;
    let padded = |n: usize| ("image/png".to_string(), [PNG, &vec![0u8; n - PNG.len()]].concat());
    assert!(composition("x", &[padded(cap / 2), padded(cap / 2)]).is_ok());
    assert_eq!(composition("x", &[padded(cap / 2), padded(cap / 2 + 1)]).unwrap_err().what(), "images_too_large");
}

/// A compose's files (ov-454): their paths go before the text once it's
/// checked, so a path's `/` is never taken for a command; a message of files
/// alone is their paths; and a command carries none.
#[test]
fn a_composes_file_paths_go_before_its_checked_text() {
    let files = [std::path::PathBuf::from("/r/pastes/compose-01-notes.txt"), std::path::PathBuf::from("/r/a b/compose-02-x.csv")];
    let composed = composition_with("summarize these", &[], &files).unwrap();
    assert_eq!(composed.text, r#"/r/pastes/compose-01-notes.txt "/r/a b/compose-02-x.csv" summarize these"#);
    assert_eq!(composed.command, None);
    assert_eq!(composition_with("", &[], &files[..1]).unwrap().text, "/r/pastes/compose-01-notes.txt");
    assert_eq!(composition_with("/init", &[], &files[..1]).unwrap_err().what(), "files");
    assert_eq!(composition_with("!ls", &[], &files[..1]).unwrap_err().what(), "command");
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

/// Working, with the screen saying idle, as claude 2.1.290's does after a
/// long paste (`paste again to expand` where `esc to interrupt` was): its
/// registry says busy, so the send goes in under the mid-turn checks. A call
/// in flight refuses it, nothing typed; with none, it's Queued.
#[tokio::test]
async fn a_turn_the_screen_misses_is_read_from_claudes_registry() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    si.show("working-hidden").await;
    b.screen_with(agent.id, "paste again to expand").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    b.svc.hooks().asks().mark_tool_starting(SESSION, Some("toolu_1"));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "after the tool", &[]).await), "busy");
    nothing_typed(&si);
    b.svc.hooks().asks().tool_ended(SESSION, Some("toolu_1"));
    assert_eq!(b.watcher.compose_into(agent.id, "after the tool", &[]).await.expect("queued"), Turn::During);
    assert!(si.log().contains("QUEUED after the tool"), "{}", si.log());
}

/// The watcher reading no agent for a sample (claude running a hook holds
/// the pane's foreground): a send goes on when claude's registry says it's
/// working, through the mid-turn checks; not when it says idle, and not
/// over someone typing.
#[tokio::test]
async fn a_watcher_that_lost_the_agent_defers_to_claudes_registry() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    b.doing(agent.id, AgentActivity::None).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "busy", "the registry says idle");
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    b.doing(agent.id, AgentActivity::None).await;
    crate::runtime::mark_input(b.svc.root_dir(), agent.id);
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "typing");
    nothing_typed(&si);
    let long_ago = now_millis() - QUIET_WATCHED_MS - 1_000;
    std::fs::write(crate::runtime::input_mark(b.svc.root_dir(), agent.id), long_ago.to_string()).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "now", &[]).await.expect("queued"), Turn::During);
    assert!(si.log().contains("QUEUED now"), "{}", si.log());
}

/// claude begins a turn of its own while the text is pasted, between
/// turns: the last check before the unfenced Enter sees it on the screen
/// (its registry lagging), or in its registry when the screen doesn't show
/// it. No Enter; the text is left in the box.
#[tokio::test]
async fn a_turn_begun_during_the_paste_gets_no_enter() {
    for mode in ["working-lagging-on-paste", "working-hidden-on-paste"] {
        let b = board().await;
        let (agent, si) = idle_claude(&b).await;
        si.show(mode).await;
        assert_eq!(refused(b.watcher.compose_into(agent.id, "one\ntwo", &[]).await), "paste_left", "{mode}");
        assert!(si.log().contains("PASTE ") && !si.log().contains("ENTER"), "{mode}: {}", si.log());
    }
}

/// A key typed after the box read back, before the Enter: no Enter.
#[tokio::test]
async fn a_key_typed_after_the_read_back_gets_no_enter() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let (root, id) = (b.svc.root_dir().to_path_buf(), agent.id);
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || crate::runtime::mark_input(&root, id)));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "one\ntwo", &[]).await), "paste_left");
    assert!(si.log().contains("PASTE ") && !si.log().contains("ENTER"), "{}", si.log());
}

/// Bracketed paste turned off between the gate and the first paste (a
/// shell taking the pane would): the paste is refused, nothing typed, so a
/// multi-line text's lines never run.
#[tokio::test]
async fn bracketed_paste_off_before_the_paste_gets_nothing() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let (control, log) = (si.control.clone(), si.log.clone());
    *b.watcher.before_paste.lock().unwrap() = Some(Box::new(move || {
        std::fs::write(&control, "nobracket").unwrap();
        for _ in 0..750 {
            if std::fs::read_to_string(&log).unwrap_or_default().contains("MODE nobracket") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
    }));
    let refusal = refused(b.watcher.compose_into(agent.id, "rm -rf build\nls", &[]).await);
    assert!(["unfamiliar", "unproven"].contains(&refusal), "{refusal}");
    nothing_typed(&si);
}

/// A send the person makes waits three seconds after a key, not the fifteen
/// an automatic send does (ov-407): four seconds on, it goes; one second
/// on, it's held.
#[tokio::test]
async fn a_persons_send_holds_three_seconds_after_a_key() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    b.doing(agent.id, AgentActivity::None).await;
    si.show("working").await;
    b.screen_with(agent.id, "esc to interrupt").await;
    let mark = crate::runtime::input_mark(b.svc.root_dir(), agent.id);
    std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
    std::fs::write(&mark, (now_millis() - 1_000).to_string()).unwrap();
    assert_eq!(refused(b.watcher.compose_into(agent.id, "now", &[]).await), "typing");
    nothing_typed(&si);
    std::fs::write(&mark, (now_millis() - 4_000).to_string()).unwrap();
    assert_eq!(b.watcher.compose_into(agent.id, "now", &[]).await.expect("queued"), Turn::During);
    // The automatic path keeps its window.
    assert!(b.watcher.typed_lately(agent.id, now_millis()), "an answer wake still waits");
}

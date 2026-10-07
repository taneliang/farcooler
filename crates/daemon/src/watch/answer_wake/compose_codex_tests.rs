//! `terminal.compose` into codex (ov-416): a message typed into codex's box
//! between turns, read back as codex 0.153.4 draws it, and Sent once the
//! rollout codex holds records it; and each refusal, with nothing typed. The
//! stand-in draws codex's placeholders and writes its rollout, its head a
//! real one's (`codex-tui-0.153.4`). On the board and tmux server of `tests`.

use std::io::Write;
use std::path::PathBuf;

use super::tell_tests::refused_with;
use super::*;

/// A real 0.153.4 rollout from the sandbox the measurements were made in.
const ROLLOUT: &str = include_str!(
    "../../../../core/fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T16-04-08-01a1189c-23ea-7190-b6bf-1d9ac1940ade.jsonl"
);

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
    0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0, 0, 0, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0, 0,
    0x03, 0x01, 0x01, 0, 0x18, 0xdd, 0x8d, 0xb0, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Where the stand-in keeps its rollout, under a codex home that isn't
/// `~/.codex` (as a `CODEX_HOME` puts it): the real rollout's head, its
/// `session_meta`, and nothing else yet.
fn rollout(b: &Board) -> PathBuf {
    let path = b.dir.path().join("codex-home/sessions/2026/10/07/rollout-2026-10-07T16-04-08-01a1189c-23ea-7190-b6bf-1d9ac1940ade.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{}\n", ROLLOUT.lines().next().unwrap())).unwrap();
    path
}

/// An idle codex stand-in that records what it takes in `rollout`, holding
/// it open from the start, or (`late`) only from its first message, as a
/// fresh codex does.
async fn idle_codex(b: &Board, rollout: &std::path::Path, late: bool) -> (Terminal, StandIn) {
    let agent = b.agent("Agent 2", "codex").await;
    let mut env = format!("STAND_IN_ROLLOUT='{}'", rollout.display());
    if late {
        std::fs::rename(rollout, rollout.with_extension("jsonl.head")).unwrap();
        env.push_str(" STAND_IN_ROLLOUT_LATE=1");
    }
    let si = b.stand_in_with(&agent, "codex", "codex", &env).await;
    b.doing(agent.id, AgentActivity::Idle).await;
    (agent, si)
}

/// The real rollout's first `task_started`, dated after any process a test
/// starts, so it's that process's turn running (`codex_turn::said`).
fn a_turn_starts() -> String {
    let line = ROLLOUT.lines().find(|l| l.contains(r#""payload":{"type":"task_started""#)).unwrap();
    let at = line.find(r#""timestamp":""#).unwrap() + r#""timestamp":""#.len();
    format!("{}2099-01-01T00:00:00.000Z{}\n", &line[..at], &line[at + "2026-10-07T23:04:12.345Z".len()..])
}

fn refused(result: Result<Turn>) -> &'static str {
    refused_with(result)
}

/// Lines with a blank one between, idle: pasted once, read back, sent, and
/// Sent once the rollout records the message.
#[tokio::test]
async fn multi_line_text_is_sent_once_the_rollout_records_it() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let turn = b.watcher.compose_into(agent.id, "fix the flaky test\r\n\nthen land it\n", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.log().matches("PASTE ").count(), 1, "one paste: {}", si.log());
    assert_eq!(si.submitted(), ["fix the flaky test\\n\\nthen land it"], "{}", si.log());
}

/// A backslash at the end, which claude would take as a line break: codex
/// 0.153.4 sends it as typed.
#[tokio::test]
async fn a_backslash_at_the_end_is_sent() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    assert_eq!(b.watcher.compose_into(agent.id, "look in C:\\", &[]).await.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), ["look in C:\\"], "{}", si.log());
}

/// Typed and entered, but no record of it in the rollout: not Sent.
#[tokio::test]
async fn a_send_the_rollout_never_records_is_unconfirmed() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "one\ntwo", &[]).await), "unconfirmed");
    si.submits(1).await;
    assert_eq!(si.submitted(), ["one\\ntwo"], "it was typed: {}", si.log());
}

/// Past 1,000 characters codex shows `[Pasted Content N chars]`: read back
/// as that, the count exact, and confirmed by the whole text.
#[tokio::test]
async fn a_long_paste_is_read_back_as_its_placeholder() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let long = "word ".repeat(220);
    assert_eq!(b.watcher.compose_into(agent.id, &long, &[]).await.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), [long.trim_end().to_string()], "{}", si.log());
}

/// An image: its path pasted alone, read back as `[Image #1]`, then the
/// text; the rollout's message carries the image.
#[tokio::test]
async fn an_image_goes_in_first_as_its_placeholder() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let image = ("image/png".to_string(), PNG.to_vec());
    let turn = b.watcher.compose_into(agent.id, "what color is it", &[image]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert_eq!(si.submitted(), ["[Image #1]  what color is it"], "{}", si.log());
    let recorded = farcooler_core::session_log::codex_prompts::prompts_from(&path, 0);
    assert_eq!(recorded.iter().map(|p| p.images).collect::<Vec<_>>(), [1], "{recorded:?}");
}

/// A codex with no rollout yet opens one at its first message: the send is
/// confirmed from the file it holds after the Enter.
#[tokio::test]
async fn a_fresh_codex_is_confirmed_by_the_rollout_it_opens() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, true).await;
    assert!(!path.exists());
    let turn = b.watcher.compose_into(agent.id, "hello", &[]).await;
    assert_eq!(turn.expect("sent"), Turn::Between, "{}", si.log());
    assert!(path.exists());
}

/// What codex would take as something other than a message, or can't be
/// read back, is refused before anything is typed: a command opens a popup
/// or a panel; a last word starting `@` or `$` opens a picker; `!` runs a
/// shell; a text taller than its box can't be read back.
#[tokio::test]
async fn what_codex_wouldn_t_send_is_refused_untyped() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let tall = (1..=60).map(|n| format!("row {n}")).collect::<Vec<_>>().join("\n");
    for (text, why) in [
        ("/init focus on tests", "handoff"),
        ("/model", "handoff"),
        ("look at @src/main.rs", "picker"),
        ("use $skill-creator", "picker"),
        ("!ls", "command"),
        (tall.as_str(), "too_tall"),
    ] {
        assert_eq!(refused(b.watcher.compose_into(agent.id, text, &[]).await), why, "{text}");
    }
    nothing_typed(&si);
}

/// A turn running, as the screen says or as the rollout says while the
/// screen reads idle: `busy`, nothing typed. codex would take the Enter as a
/// steer, under approvals no hook fences.
#[tokio::test]
async fn a_codex_mid_turn_is_busy() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    si.show("working").await;
    b.doing(agent.id, AgentActivity::Working).await;
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "busy");
    si.show("idle").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(a_turn_starts().as_bytes()).unwrap();
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "busy");
    nothing_typed(&si);
}

/// A turn begun between the read-back and the Enter, as the rollout says:
/// no Enter, the text left in the box.
#[tokio::test]
async fn a_turn_begun_before_the_enter_leaves_the_paste() {
    let b = board().await;
    let path = rollout(&b);
    let (agent, si) = idle_codex(&b, &path, false).await;
    let start = a_turn_starts();
    let held = path.clone();
    *b.watcher.before_enter.lock().unwrap() = Some(Box::new(move || {
        std::fs::OpenOptions::new().append(true).open(&held).unwrap().write_all(start.as_bytes()).unwrap();
    }));
    assert_eq!(refused(b.watcher.compose_into(agent.id, "hello", &[]).await), "paste_left");
    assert!(si.log().contains("PASTE hello"), "{}", si.log());
    assert!(!si.log().contains("ENTER"), "{}", si.log());
}

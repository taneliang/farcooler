//! The messages a person sent codex, as its rollout records them: how a
//! message typed into codex's box is confirmed taken (ov-416).
//!
//! codex-cli 0.153.4 writes each as an `event_msg/item_completed` whose item
//! is a `UserMessage`: one `local_image` part per image (its path), then one
//! `text` part, the box's text with each image's `[Image #N]` and every
//! collapsed paste's whole text (measured in a sandbox; the fixture is
//! `codex-tui-0.153.4/rollout-2026-10-07T16-04-08-…`). The
//! `response_item` user message isn't read: it also carries codex's own
//! `<environment_context>` and `<image>` wrappers.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// The most read past `from`: a send is confirmed within seconds of its
/// Enter, and a turn's records in that time are far less.
const READ_LIMIT: u64 = 16 * 1024 * 1024;

/// One message as codex took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Its text, as the rollout has it.
    pub text: String,
    /// How many images it carried.
    pub images: usize,
    /// When the rollout dates it (milliseconds since the epoch), if it does.
    pub at_ms: Option<i64>,
}

/// The messages recorded in the rollout at `path` from byte `from` on, in
/// order. A line not yet ended is left for the next read; anything that
/// can't be read is nothing.
pub fn prompts_from(path: &Path, from: u64) -> Vec<Prompt> {
    let Ok(mut file) = File::open(path) else { return Vec::new() };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(READ_LIMIT).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let ended = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |at| at + 1);
    String::from_utf8_lossy(&bytes[..ended]).lines().filter_map(prompt).collect()
}

fn prompt(line: &str) -> Option<Prompt> {
    // Most lines aren't one: skip them before parsing.
    if !line.contains("\"UserMessage\"") {
        return None;
    }
    let record: Value = serde_json::from_str(line).ok()?;
    let payload = record.get("payload")?;
    if record.get("type")?.as_str()? != "event_msg" || payload.get("type")?.as_str()? != "item_completed" {
        return None;
    }
    let item = payload.get("item")?;
    if item.get("type")?.as_str()? != "UserMessage" {
        return None;
    }
    let parts = item.get("content")?.as_array()?;
    let kind = |part: &Value| part.get("type").and_then(Value::as_str).map(str::to_string);
    let images = parts.iter().filter(|p| kind(p).as_deref() == Some("local_image")).count();
    let text = parts
        .iter()
        .filter(|p| kind(p).as_deref() == Some("text"))
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let at_ms = record.get("timestamp").and_then(Value::as_str).and_then(super::claude::parse_iso8601_millis);
    Some(Prompt { text, images, at_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLLOUT: &str = include_str!(
        "../../fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T16-04-08-01a1189c-23ea-7190-b6bf-1d9ac1940ade.jsonl"
    );

    /// Every message of a real session, with its images, and none of the
    /// `/vim` commands between them; from a byte on, only those after it.
    #[test]
    fn a_real_rollout_s_messages_are_read() {
        let dir = std::env::temp_dir().join(format!("fc-codex-prompts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, ROLLOUT).unwrap();
        let all = prompts_from(&path, 0);
        let texts: Vec<(&str, usize)> = all.iter().map(|p| (&p.text[..p.text.len().min(20)], p.images)).collect();
        assert_eq!(
            texts,
            [("[Image #1]  describe", 1), ("vim paste test", 0), ("vim insert test", 0), ("[Image #1]  yyyyyyyy", 1)]
        );
        assert_eq!(all[3].text, format!("[Image #1]  {}", "y".repeat(1200)), "a collapsed paste, whole");
        let later = ROLLOUT.find("vim insert test").unwrap();
        let from = ROLLOUT[..later].rfind('\n').unwrap() as u64 + 1;
        assert_eq!(prompts_from(&path, from).len(), 2);
        let last = ROLLOUT.rfind("[Image #1]  yyy").unwrap();
        let cut = last + ROLLOUT[last..].find('\n').unwrap();
        std::fs::write(&path, &ROLLOUT[..cut]).unwrap();
        assert_eq!(prompts_from(&path, 0).len(), 3, "a line half written is not read");
        assert!(prompts_from(&dir.join("none.jsonl"), 0).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

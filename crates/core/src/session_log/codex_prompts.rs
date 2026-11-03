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

/// A complete rollout line and its byte offset. The offset can be persisted
/// with a receipt so its evidence can be found again after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionedPrompt {
    pub offset: u64,
    pub prompt: Prompt,
}

/// Read complete messages at or after a previously captured line boundary.
/// Incomplete and malformed lines are never evidence of receipt.
pub fn positioned_prompts_from(path: &Path, from: u64) -> Vec<PositionedPrompt> {
    let Ok(mut file) = File::open(path) else { return Vec::new() };
    positioned_prompts_in(&mut file, from)
}

/// Read from the already-opened file, so receipt callers can check the
/// identity of the same handle that supplied the evidence.
pub fn positioned_prompts_in(file: &mut File, from: u64) -> Vec<PositionedPrompt> {
    if file.seek(SeekFrom::Start(from)).is_err() { return Vec::new() }
    let mut bytes = Vec::new();
    if file.take(READ_LIMIT).read_to_end(&mut bytes).is_err() { return Vec::new() }
    let mut offset = from;
    let mut found = Vec::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if !line.ends_with(b"\n") { break }
        if let Ok(text) = std::str::from_utf8(line)
            && let Some(prompt) = prompt(text.trim_end_matches('\n'))
        {
            found.push(PositionedPrompt { offset, prompt });
        }
        offset = offset.saturating_add(line.len() as u64);
    }
    found
}

/// Find an exact, image-free prompt at or after an Enter. The caller must
/// first verify that `file` is the pinned rollout for the same session; this
/// function makes no claim about file or thread identity on its own.
pub fn exact_prompt_in(file: &mut File, from: u64, text: &str, entered_at: i64) -> Option<PositionedPrompt> {
    if text.is_empty() { return None }
    positioned_prompts_in(file, from).into_iter().find(|item| {
        item.prompt.images == 0 && item.prompt.text == text && item.prompt.at_ms.is_some_and(|at| at >= entered_at)
    })
}

/// Read the main Codex thread identity from this same open rollout handle.
/// A subagent or incomplete header is never evidence for a pane's session.
pub fn main_thread_in(file: &mut File) -> Option<String> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut bytes).ok()?;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if !line.ends_with(b"\n") { break }
        let Ok(record) = serde_json::from_slice::<Value>(line) else { continue };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") { continue }
        let payload = record.get("payload")?;
        if payload.get("source").and_then(|source| source.get("subagent")).is_some() { return None }
        let id = payload.get("id")?.as_str()?;
        uuid::Uuid::parse_str(id).ok()?;
        return Some(id.to_string());
    }
    None
}

/// Match a prompt only in the already-open file whose device and inode were
/// pinned before submission. The caller must separately prove the Codex
/// thread still names this rollout; a matching path alone is not that proof.
#[cfg(unix)]
pub fn exact_prompt_in_pinned_file(
    file: &mut File,
    device: u64,
    inode: u64,
    from: u64,
    text: &str,
    entered_at: i64,
) -> Option<PositionedPrompt> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().ok()?;
    if metadata.dev() != device || metadata.ino() != inode || metadata.len() < from { return None }
    exact_prompt_in(file, from, text, entered_at)
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

    #[test]
    fn positioned_messages_name_complete_rollout_lines() {
        let dir = crate::scratch::dir("codex-prompt-offsets");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, ROLLOUT).unwrap();
        let found = positioned_prompts_from(&path, 0);
        assert_eq!(found.len(), 4);
        for item in &found {
            let remainder = &ROLLOUT[item.offset as usize..];
            assert!(remainder.starts_with('{'));
            assert_eq!(prompts_from(&path, item.offset)[0], item.prompt);
        }
        let last = found.last().unwrap().offset as usize;
        let line = &ROLLOUT[last..];
        let end = line.find('\n').unwrap();
        std::fs::write(&path, &ROLLOUT[..last + end]).unwrap();
        assert_eq!(positioned_prompts_from(&path, u64::try_from(last).unwrap()), []);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thread_identity_is_read_from_the_open_main_rollout() {
        let dir = crate::scratch::dir("codex-receipt-thread");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, ROLLOUT).unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(main_thread_in(&mut file).as_deref(), Some("01a1189c-23ea-7190-b6bf-1d9ac1940ade"));
        std::fs::write(&path, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"01a1189c-23ea-7190-b6bf-1d9ac1940ade\",\"source\":{\"subagent\":{}}}}\n").unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(main_thread_in(&mut file), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_receipt_requires_matching_complete_text_and_timestamp() {
        let dir = crate::scratch::dir("codex-exact-receipt");
        let path = dir.join("rollout.jsonl");
        std::fs::write(&path, ROLLOUT).unwrap();
        let item = positioned_prompts_from(&path, 0).into_iter().find(|p| p.prompt.text == "vim paste test").unwrap();
        let at = item.prompt.at_ms.unwrap();
        let mut file = File::open(&path).unwrap();
        assert_eq!(exact_prompt_in(&mut file, item.offset, "vim paste test", at), Some(item.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata().unwrap();
            assert_eq!(exact_prompt_in_pinned_file(&mut file, metadata.dev(), metadata.ino(), item.offset, "vim paste test", at), Some(item.clone()));
            assert_eq!(exact_prompt_in_pinned_file(&mut file, metadata.dev(), metadata.ino().wrapping_add(1), item.offset, "vim paste test", at), None);
        }

        assert_eq!(exact_prompt_in(&mut file, item.offset, "vim paste test", at + 1), None);
        assert_eq!(exact_prompt_in(&mut file, item.offset, "vim paste tes", at), None);
        assert_eq!(exact_prompt_in(&mut file, item.offset + 1, "vim paste test", at), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every message of a real session, with its images, and none of the
    /// `/vim` commands between them; from a byte on, only those after it.
    #[test]
    fn a_real_rollout_s_messages_are_read() {
        let dir = crate::scratch::dir("codex-prompts");
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

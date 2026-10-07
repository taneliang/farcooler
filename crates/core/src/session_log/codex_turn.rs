//! Whether codex is between turns, from its rollout's last turn boundary
//! (ov-378): the busy and idle a send into a codex pane is gated on, as
//! claude's is on its session registry (`registry_turn` in the daemon).
//!
//! codex writes `event_msg/task_started` when a turn begins, and
//! `task_complete` or `turn_aborted` when it ends, every turn: a prompt, a
//! steer's turn, a Tab-queued one, a `!cmd`, a denied approval. Measured on
//! codex-cli 0.153.4 (`.claude/agent/reports/codex-projection/design.md`):
//! the record is on disk within about 40 ms of the screen changing, whether
//! or not codex's hooks run (they run only once trusted).
//!
//! Read backwards from the end of the file, so a 90 MB rollout costs what its
//! last turn's records cost, up to `SCAN_LIMIT`. Fails closed: a last line
//! with no newline yet is `Writing` (it may be a `task_started`), and no
//! boundary found is `Unknown`, never `Closed`. Which process writes the file
//! is the caller's to prove.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

/// How far back a scan reads before giving up. A turn whose records since
/// its start outweigh this (a long run of large tool outputs) reads as
/// `Unknown`.
pub const SCAN_LIMIT: u64 = 16 * 1024 * 1024;

/// The read size, backwards.
const CHUNK: usize = 256 * 1024;

/// A line up to this long is parsed whole. A longer one (a tool's output,
/// the model's history, or a `task_complete` whose `last_agent_message` is a
/// long final reply: 4 of 1,355 on this Mac, the largest 21 KB) is read by
/// its head alone (`boundary_by_head`).
const BOUNDARY_LINE_MAX: usize = 16 * 1024;

/// How much of a long line's head is kept: its record type and its
/// payload's type come first, before any free text.
const HEAD: usize = 512;

/// What a rollout's last turn boundary says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloutTurn {
    /// `task_started` is the last boundary: a turn is running, begun at
    /// `started_ms` (the record's own time). A turn begun before the process
    /// now writing the file started is a dead process's (`codex resume`
    /// writes nothing until the next prompt): that's the caller's to judge.
    Open { started_ms: Option<i64> },
    /// `task_complete` or `turn_aborted` is: between turns.
    Closed,
    /// The file ends partway through a line codex is still writing.
    Writing,
    /// No boundary within `SCAN_LIMIT`, or the file can't be read.
    Unknown,
}

/// The last turn boundary in the rollout at `path`.
pub fn last_turn(path: &Path) -> RolloutTurn {
    last_turn_in(path, CHUNK, SCAN_LIMIT)
}

/// `last_turn`, with the read size and the bound given, for tests.
pub fn last_turn_in(path: &Path, chunk: usize, limit: u64) -> RolloutTurn {
    let Ok(mut file) = File::open(path) else { return RolloutTurn::Unknown };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return RolloutTurn::Unknown };
    if len == 0 {
        return RolloutTurn::Unknown;
    }
    let mut last = [0u8; 1];
    if file.seek(SeekFrom::Start(len - 1)).is_err() || file.read_exact(&mut last).is_err() {
        return RolloutTurn::Unknown;
    }
    if last[0] != b'\n' {
        return RolloutTurn::Writing;
    }
    // Bytes `[0, end)` are unread; `line` holds the start of the line that
    // runs on past them, read so far: all of it, or once it is `long`, only
    // its first `HEAD` bytes, which the next read backwards replaces.
    let mut end = len - 1;
    let mut line: Vec<u8> = Vec::new();
    let mut long = false;
    while end > 0 && len - end <= limit {
        let start = end.saturating_sub(chunk as u64);
        let mut buf = vec![0u8; (end - start) as usize];
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut buf).is_err() {
            return RolloutTurn::Unknown;
        }
        let mut hi = buf.len();
        while let Some(newline) = buf[..hi].iter().rposition(|&b| b == b'\n') {
            prepend(&mut line, &mut long, &buf[newline + 1..hi]);
            if let Some(found) = boundary(&line, long) {
                return found;
            }
            line.clear();
            long = false;
            hi = newline;
        }
        prepend(&mut line, &mut long, &buf[..hi]);
        end = start;
    }
    if end == 0 {
        if let Some(found) = boundary(&line, long) {
            return found;
        }
    }
    RolloutTurn::Unknown
}

/// Put `before` ahead of what is held of a line, keeping it whole up to
/// `BOUNDARY_LINE_MAX` and past that only its head.
fn prepend(line: &mut Vec<u8>, long: &mut bool, before: &[u8]) {
    if before.is_empty() {
        return;
    }
    let mut joined = Vec::with_capacity(before.len() + line.len().min(HEAD));
    joined.extend_from_slice(before);
    joined.extend_from_slice(line);
    if joined.len() > BOUNDARY_LINE_MAX {
        *long = true;
    }
    if *long {
        joined.truncate(HEAD);
    }
    *line = joined;
}

/// The boundary `line` is, if it is one: parsed whole, or for a line too
/// long to parse (`long`, only its head held), read from its head.
fn boundary(line: &[u8], long: bool) -> Option<RolloutTurn> {
    if !contains(line, b"event_msg") {
        return None;
    }
    if long {
        return boundary_by_head(line);
    }
    let record: Value = serde_json::from_slice(line).ok()?;
    if record.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let started_ms = || record.get("timestamp").and_then(Value::as_str).and_then(super::claude::parse_iso8601_millis);
    match record.pointer("/payload/type").and_then(Value::as_str)? {
        "task_started" => Some(RolloutTurn::Open { started_ms: started_ms() }),
        "task_complete" | "turn_aborted" => Some(RolloutTurn::Closed),
        _ => None,
    }
}

/// A long line's boundary, from the head codex writes before any free text:
/// `{"timestamp":…,"ordinal":N,"type":"event_msg","payload":{"type":"…"`.
/// Only an end is read this way. A `task_started` is never long, and one
/// that somehow were reads as no boundary, which errs toward `Unknown`.
fn boundary_by_head(head: &[u8]) -> Option<RolloutTurn> {
    const KIND: &[u8] = br#""type":"event_msg","payload":{"type":""#;
    // The record's own type must be the first `"type"` in the line: only
    // the timestamp and the ordinal come before it.
    let first = head.windows(8).position(|w| w == br#""type":""#)?;
    let after = head[first..].strip_prefix(KIND)?;
    [&br#"task_complete""#[..], br#"turn_aborted""#].iter().any(|end| after.starts_with(end)).then_some(RolloutTurn::Closed)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
#[path = "codex_turn_tests.rs"]
mod tests;

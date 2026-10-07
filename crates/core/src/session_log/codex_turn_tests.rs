use std::io::Write;
use std::path::PathBuf;

use super::*;

const MAIN: &str = include_str!("../../fixtures/session-logs/codex-tui-0.153.4/rollout-2026-10-07T12-22-14-01a117d0-fd5c-7fe3-8f0f-5f2afdf7906d.jsonl");

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("fc-codex-turn-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn file(&self, bytes: &[u8]) -> PathBuf {
        let path = self.0.join("rollout-test.jsonl");
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a line says, if it is a boundary: the truth the scan is held to.
fn truth(line: &str) -> Option<RolloutTurn> {
    let record: Value = serde_json::from_str(line).unwrap();
    if record["type"] != "event_msg" {
        return None;
    }
    match record["payload"]["type"].as_str() {
        Some("task_started") => Some(RolloutTurn::Open {
            started_ms: record["timestamp"].as_str().and_then(crate::session_log::claude::parse_iso8601_millis),
        }),
        Some("task_complete" | "turn_aborted") => Some(RolloutTurn::Closed),
        _ => None,
    }
}

/// The real session cut after every line, read with a chunk small enough
/// that boundaries straddle reads: open exactly while a turn runs.
#[test]
fn every_cut_of_a_real_rollout_reads_as_its_last_boundary() {
    let dir = Scratch::new("cuts");
    let mut text = String::new();
    let mut want = RolloutTurn::Unknown;
    let (mut open, mut closed) = (0, 0);
    for line in MAIN.lines() {
        text.push_str(line);
        text.push('\n');
        want = truth(line).unwrap_or(want);
        let path = dir.file(text.as_bytes());
        for chunk in [97, 4096, CHUNK] {
            assert_eq!(last_turn_in(&path, chunk, SCAN_LIMIT), want, "chunk {chunk} after {line:.120}");
        }
        open += usize::from(matches!(want, RolloutTurn::Open { started_ms: Some(_) }));
        closed += usize::from(want == RolloutTurn::Closed);
    }
    assert!(open > 30 && closed > 15, "{open} open, {closed} closed");
}

/// A line codex is partway through writing is not a closed turn.
#[test]
fn a_half_written_last_line_is_writing() {
    let dir = Scratch::new("half");
    let cut = MAIN.rfind("{\"timestamp\"").unwrap() + 40;
    assert_eq!(last_turn(&dir.file(&MAIN.as_bytes()[..cut])), RolloutTurn::Writing);
}

/// A turn's big tool output after its start is skipped, not parsed, and the
/// start behind it is still found.
#[test]
fn a_huge_output_after_the_start_still_reads_open() {
    let dir = Scratch::new("huge");
    let start = MAIN.lines().rfind(|l| matches!(truth(l), Some(RolloutTurn::Open { .. }))).unwrap();
    let mut text = format!("{}\n{start}\n", MAIN.lines().next().unwrap());
    let big = "x".repeat(3 * 1024 * 1024);
    text.push_str(&format!(r#"{{"timestamp":"2026-10-07T19:28:23.100Z","type":"response_item","payload":{{"type":"function_call_output","call_id":"c","output":"{big}"}}}}"#));
    text.push('\n');
    let path = dir.file(text.as_bytes());
    assert_eq!(last_turn(&path), truth(start).unwrap());
    // Past the bound, it says nothing rather than guess.
    assert_eq!(last_turn_in(&path, CHUNK, 1024 * 1024), RolloutTurn::Unknown);
}

/// A boundary word in a tool's output, or in a prompt, is not a boundary.
#[test]
fn only_an_event_msg_record_is_a_boundary() {
    let dir = Scratch::new("words");
    let mut file = MAIN.lines().take_while(|l| !l.contains("task_started")).collect::<Vec<_>>().join("\n");
    file.push('\n');
    let mut path = dir.file(file.as_bytes());
    assert_eq!(last_turn(&path), RolloutTurn::Unknown, "no turn yet");
    let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(f, r#"{{"timestamp":"2026-10-07T19:28:23.100Z","type":"response_item","payload":{{"type":"function_call_output","call_id":"c","output":"{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_complete\"}}}}"}}}}"#).unwrap();
    assert_eq!(last_turn(&path), RolloutTurn::Unknown, "an output that quotes a boundary");
    writeln!(f, r#"{{"timestamp":"2026-10-07T19:28:23.200Z","type":"response_item","payload":{{"type":"task_started","turn_id":"t","note":"not an event_msg"}}}}"#).unwrap();
    assert_eq!(last_turn(&path), RolloutTurn::Unknown, "a history item, whatever its payload says");
    path = dir.file(b"");
    assert_eq!(last_turn(&path), RolloutTurn::Unknown, "empty");
    assert_eq!(last_turn(&dir.0.join("missing.jsonl")), RolloutTurn::Unknown, "missing");
}

/// A turn's end whose `last_agent_message` is a long final reply (review 1,
/// H1: 4 real ones over 16 KB, the largest 21 KB) still ends it, read from
/// its head; and so does a long `turn_aborted`.
#[test]
fn a_long_turn_end_still_closes_the_turn() {
    let dir = Scratch::new("long-end");
    let lines: Vec<&str> = MAIN.lines().collect();
    let last_end = lines.iter().rposition(|l| truth(l) == Some(RolloutTurn::Closed)).unwrap();
    let reply = "a long final reply, then a question? ".repeat(1_000);
    for kind in ["task_complete", "turn_aborted"] {
        let end = format!(
            r#"{{"timestamp":"2026-10-07T19:28:42.832Z","ordinal":120,"type":"event_msg","payload":{{"type":"{kind}","turn_id":"t","last_agent_message":"{reply}","duration_ms":437}}}}"#
        );
        assert!(end.len() > 32 * 1024);
        let text = lines[..last_end].join("\n") + "\n" + &end + "\n";
        for chunk in [97, 4096, CHUNK] {
            assert_eq!(last_turn_in(&dir.file(text.as_bytes()), chunk, SCAN_LIMIT), RolloutTurn::Closed, "{kind}, chunk {chunk}");
        }
    }
    // A long line holding an end's shape anywhere but at its own head is
    // not one. (Inside a string it can't be: JSON escapes its quotes.)
    let quoted = format!(
        r#"{{"timestamp":"2026-10-07T19:28:42.832Z","type":"response_item","payload":{{"type":"function_call_output","meta":{{"type":"event_msg","payload":{{"type":"task_complete"}}}},"output":"{reply}"}}}}"#
    );
    let start = lines.iter().rposition(|l| matches!(truth(l), Some(RolloutTurn::Open { .. }))).unwrap();
    let text = lines[..=start].join("\n") + "\n" + &quoted + "\n";
    assert_eq!(last_turn(&dir.file(text.as_bytes())), truth(lines[start]).unwrap());
}

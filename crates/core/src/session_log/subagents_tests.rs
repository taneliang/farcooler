//! The description a subagent's meta file carries (ov-509), read beside its
//! transcript the way a pane's follower reads both.

use std::io::Write;

use super::SubagentLogs;
use crate::session_log::usage::LogUsage;

const TRANSCRIPT: &str = include_str!("../../fixtures/session-logs/claude-subagent-transcript.jsonl");
const META: &str = include_str!("../../fixtures/session-logs/claude-subagent-transcript.meta.json");
const AGENT: &str = "a0000000000000001";

fn session(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = crate::scratch::dir(&format!("subagents-{name}"));
    let dir = root.join("session").join("subagents");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(root.join("session.jsonl"), "").unwrap();
    (root, dir)
}

fn run_of(usage: &mut LogUsage) -> Option<String> {
    let turns = usage.take();
    turns.into_iter().find(|t| t.key.ends_with(AGENT)).and_then(|t| t.description)
}

/// The meta file lands a moment after the transcript's first lines: the run
/// is handed out with no name, then again with it.
#[test]
fn a_meta_file_that_lands_late_still_names_the_run() {
    let (root, dir) = session("late");
    let mut logs = SubagentLogs::default();
    let mut usage = LogUsage::default();
    let transcript = dir.join(format!("agent-{AGENT}.jsonl"));
    std::fs::write(&transcript, TRANSCRIPT).unwrap();
    logs.follow(&root.join("session.jsonl"), true, &mut usage);
    assert_eq!(run_of(&mut usage), None, "no meta file yet");

    std::fs::write(dir.join(format!("agent-{AGENT}.meta.json")), META).unwrap();
    // The transcript grows, as a running subagent's does.
    let mut file = std::fs::OpenOptions::new().append(true).open(&transcript).unwrap();
    writeln!(file, "{}", TRANSCRIPT.lines().last().unwrap()).unwrap();
    drop(file);
    logs.follow(&root.join("session.jsonl"), false, &mut usage);
    assert_eq!(run_of(&mut usage).as_deref(), Some("Sleep 200 then report"));
    std::fs::remove_dir_all(root).unwrap();
}

/// A meta file that is not JSON, or names nothing, is no description.
#[test]
fn a_meta_file_that_says_nothing_names_nothing() {
    for body in ["not json", r#"{"description":"  "}"#, r#"{"agentType":"general-purpose"}"#] {
        let (root, dir) = session("blank");
        std::fs::write(dir.join(format!("agent-{AGENT}.jsonl")), TRANSCRIPT).unwrap();
        std::fs::write(dir.join(format!("agent-{AGENT}.meta.json")), body).unwrap();
        let mut usage = LogUsage::default();
        SubagentLogs::default().follow(&root.join("session.jsonl"), true, &mut usage);
        assert_eq!(run_of(&mut usage), None, "{body}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

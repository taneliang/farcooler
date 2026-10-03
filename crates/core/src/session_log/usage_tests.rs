//! Every test reads a recorded log from `fixtures/session-logs/` (see its
//! README); where a test needs a shape no recording has, it says which line
//! it changed and why.

use super::*;

fn fixture(name: &str) -> Vec<String> {
    let path = format!("{}/fixtures/session-logs/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap().lines().map(str::to_string).collect()
}

fn claude(lines: &[String]) -> Vec<LoggedTurn> {
    let mut usage = LogUsage::default();
    lines.iter().for_each(|l| usage.claude_line(l));
    usage.take()
}

fn codex(lines: &[String]) -> Vec<LoggedTurn> {
    let mut usage = LogUsage::default();
    lines.iter().for_each(|l| usage.codex_line(l));
    usage.take()
}

/// Drop `message.usage` from line `n`.
fn without_usage(mut lines: Vec<String>, n: usize) -> Vec<String> {
    let mut record: Value = serde_json::from_str(&lines[n]).unwrap();
    record["message"].as_object_mut().unwrap().remove("usage");
    lines[n] = record.to_string();
    lines
}

/// The complete turn's two model calls: `msg_…Fm` written twice (thinking,
/// then tool_use) with the same usage, and `msg_…Ru`.
const OPUS_TURN: TokenCounts =
    TokenCounts { input: 4, output: 868, cache_read: 55595, cache_write: 11792, cache_write_1h: 11792 };

#[test]
fn a_claude_turn_counts_each_model_call_once() {
    let turns = claude(&fixture("claude-complete-turn.jsonl"));
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    assert_eq!(turn.key, "claude-log:00000000-0000-4000-8000-000000000003");
    assert_eq!(turn.models, vec![(Some("claude-opus-5".to_string()), OPUS_TURN)]);
    assert_eq!(turn.state, UsageState::Reported);
    assert_eq!(turn.active_ms, Some(14681), "claude's own durationMs");
    assert_eq!(turn.started_at_ms.zip(turn.ended_at_ms).map(|(s, e)| e - s), Some(87686));
}

/// Two lines that are one streamed message, read as two, would put this
/// turn's output at 1,188 rather than 868.
#[test]
fn a_message_repeated_across_streaming_lines_is_one_call() {
    let lines = fixture("claude-complete-turn.jsonl");
    let mut doubled = lines.clone();
    doubled.insert(2, lines[1].clone());
    assert_eq!(claude(&doubled)[0].models[0].1.output, 868);
}

/// A log that rotates is followed into its next file by the same fold, and
/// `Tail` re-reads a replaced file from its start: the turn that spans the
/// two is still one turn, counted once.
#[test]
fn a_turn_split_across_a_log_rotation_is_one_turn() {
    let lines = fixture("claude-complete-turn.jsonl");
    let mut usage = LogUsage::default();
    lines[..3].iter().for_each(|l| usage.claude_line(l));
    assert!(usage.take().is_empty(), "nothing is finished mid-turn");
    // The next file, read whole: the start and the first call again, then the rest.
    lines.iter().for_each(|l| usage.claude_line(l));
    let turns = usage.take();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].models[0].1, OPUS_TURN);
    // And the same file read a third time reports nothing new.
    lines.iter().for_each(|l| usage.claude_line(l));
    assert!(usage.take().is_empty());
}

/// A call whose line lost its `usage` leaves the turn's count a floor, said
/// as such; a turn with no usage at all is not reported, never zero.
#[test]
fn a_missing_usage_field_is_partial_or_not_reported() {
    let partial = claude(&without_usage(fixture("claude-complete-turn.jsonl"), 4));
    assert_eq!(partial[0].state, UsageState::Partial);
    assert_eq!(partial[0].models[0].1.output, 320);

    let none = claude(&without_usage(without_usage(without_usage(fixture("claude-complete-turn.jsonl"), 1), 2), 4));
    assert_eq!(none[0].state, UsageState::NotReported);
    assert!(none[0].models.is_empty());
}

/// Attached after the turn began, the fold has half a turn and records none.
#[test]
fn a_turn_whose_start_was_not_read_is_not_recorded() {
    assert!(claude(&fixture("claude-complete-turn.jsonl")[1..]).is_empty());
    assert!(claude(&fixture("claude-task-list.jsonl")).is_empty(), "this excerpt starts mid-turn");
}

/// An interrupted turn writes no `turn_duration`; the next prompt ends it,
/// with what it spent.
#[test]
fn an_interrupted_turn_is_closed_by_the_next() {
    let mut lines = fixture("claude-complete-turn.jsonl");
    lines.truncate(5);
    let mut next: Value = serde_json::from_str(&lines[0]).unwrap();
    next["uuid"] = "00000000-0000-4000-8000-000000000099".into();
    lines.push(next.to_string());
    let turns = claude(&lines);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].models[0].1, OPUS_TURN);
    assert_eq!(turns[0].active_ms, Some(87617), "from its start to its last line");
}

/// Two turns of codex 0.153's auto-reviewer, each the difference in the
/// session's running total, cached input split out.
#[test]
fn codex_turns_are_the_difference_in_the_running_total() {
    let turns = codex(&fixture("codex-token-counts.jsonl"));
    let tokens = |input, cache_read, output| TokenCounts { input, output, cache_read, ..Default::default() };
    let model = Some("codex-auto-review".to_string());
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].models, vec![(model.clone(), tokens(2209, 4864, 154))]);
    assert_eq!(turns[1].models, vec![(model, tokens(2883, 4864, 60))]);
    assert_eq!((turns[0].active_ms, turns[1].active_ms), (Some(5250), Some(4071)));
    assert_eq!(turns[1].key, "codex-log:00000000-0000-4000-8000-000000000044");
}

/// The rollout re-read whole after a rotation, partway into the first turn:
/// two turns, each once, with the same counts.
#[test]
fn a_codex_rollout_read_again_across_a_rotation_counts_each_turn_once() {
    let lines = fixture("codex-token-counts.jsonl");
    let mut usage = LogUsage::default();
    lines[..3].iter().for_each(|l| usage.codex_line(l));
    lines.iter().for_each(|l| usage.codex_line(l));
    lines.iter().for_each(|l| usage.codex_line(l));
    let turns = usage.take();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[1].models[0].1.input, 2883);
}

#[test]
fn an_older_codex_rollout_reads_the_same_way() {
    let turns = codex(&fixture("codex-complete-turn.jsonl"));
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].models[0].0.as_deref(), Some("gpt-5.5"));
    assert_eq!(turns[0].models[0].1, TokenCounts { input: 6631, output: 6, cache_read: 4992, ..Default::default() });
}

/// A codex turn that wrote no `token_count` reported nothing.
#[test]
fn a_codex_turn_without_a_token_count_is_not_reported() {
    let turns = codex(&fixture("codex-item-completed-turn.jsonl"));
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].state, UsageState::NotReported);
    assert!(turns[0].models.is_empty());
}

/// A real subagent transcript (`claude-subagent-transcript.jsonl`): five calls
/// over eight assistant lines, one of them written first with 2 output tokens
/// and then with 658. Summed by line, output would read 688.
const SUBAGENT: TokenCounts =
    TokenCounts { input: 10, output: 681, cache_read: 110128, cache_write: 31610, cache_write_1h: 0 };

#[test]
fn a_subagents_transcript_counts_each_call_once_as_its_own_entry() {
    let mut usage = LogUsage::default();
    fixture("claude-subagent-transcript.jsonl").iter().for_each(|l| usage.subagent_line("a1", l));
    let runs = usage.take();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].subagent);
    assert_eq!(runs[0].key, "claude-log:agent:a1");
    assert_eq!(runs[0].models, vec![(Some("claude-opus-5".to_string()), SUBAGENT)]);
    assert_eq!(runs[0].active_ms, Some(207_676), "first line to last");
}

/// A subagent still running is handed out again as it grows, whole, under
/// the same key; one that wrote nothing new is not.
#[test]
fn a_running_subagent_is_handed_out_again_as_it_grows() {
    let lines = fixture("claude-subagent-transcript.jsonl");
    let mut usage = LogUsage::default();
    lines[..4].iter().for_each(|l| usage.subagent_line("a1", l));
    let early = usage.take();
    assert_eq!(early[0].models[0].1.cache_write, 25414 + 1435);
    lines[4..].iter().for_each(|l| usage.subagent_line("a1", l));
    let later = usage.take();
    assert_eq!((later.len(), later[0].key.as_str()), (1, "claude-log:agent:a1"));
    assert_eq!(later[0].models[0].1, SUBAGENT);
    assert!(usage.take().is_empty());
}

/// The follower finds the transcripts beside the session it follows.
#[test]
fn a_sessions_subagent_files_are_followed() {
    let dir = std::env::temp_dir().join(format!("farcooler-subagents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let session = dir.join("00000000-0000-4000-8000-000000000001.jsonl");
    std::fs::write(&session, fixture("claude-complete-turn.jsonl").join("\n") + "\n").unwrap();
    let agents = dir.join("00000000-0000-4000-8000-000000000001/subagents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("agent-a1.jsonl"), fixture("claude-subagent-transcript.jsonl").join("\n") + "\n").unwrap();
    std::fs::write(agents.join("agent-a1.meta.json"), "{}").unwrap();

    let mut usage = LogUsage::default();
    let mut follower = super::super::subagents::SubagentLogs::default();
    follower.follow(&session, false, &mut usage);
    let runs = usage.take();
    assert_eq!(runs.len(), 1, "the transcript, not its meta file");
    assert_eq!(runs[0].models[0].1, SUBAGENT);
    let _ = std::fs::remove_dir_all(&dir);
}

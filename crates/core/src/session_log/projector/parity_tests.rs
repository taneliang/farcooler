//! The projector against the reader it is to replace, over the same lines.
//!
//! `claude::parse_line` is what watch.rs folds into a pane's turn, its
//! question, its subagents and its transcript prose today. Wherever the two
//! readers mean the same thing they must agree; where the projector knows
//! more (a turn ended by `end_turn` before its `turn_duration`, a failure, an
//! interrupt), that is named below rather than averaged away.
//!
//! Not compared, on purpose: task lists (`TaskState`), which the projector
//! leaves to the old reader and draws as tool rows; usage, which
//! `session_log::usage` keeps; titles.

use std::collections::BTreeMap;

use super::fixtures::*;
use super::rows::*;
use super::Projection;
use crate::session_log::{claude, SubagentStatus, TurnEvent};

/// What both readers can be asked, in one shape.
#[derive(Debug, Default, PartialEq)]
struct Summary {
    prompts: usize,
    prose: Vec<(String, bool)>,
    questions: Vec<String>,
    /// Spawning `tool_use` id to (`agentId`, still running).
    subagents: BTreeMap<String, (Option<String>, bool)>,
    /// `durationMs` of every `turn_duration`, in order.
    durations: Vec<i64>,
}

fn old_reader(text: &str) -> Summary {
    let mut s = Summary::default();
    let mut agent_of: BTreeMap<String, String> = BTreeMap::new();
    // Two differences the projector means to have, taken out here so the
    // rest can be compared: a line written twice (one real session holds
    // 24,170 repeated lines) is one record, and a second prompt record with a
    // promptId already seen continues that prompt's turn rather than starting
    // another (21 in the same session).
    let mut lines_seen = std::collections::HashSet::new();
    let mut prompts_seen = std::collections::HashSet::new();
    for line in text.lines() {
        if !lines_seen.insert(line) {
            continue;
        }
        let prompt_id = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| Some(v.get("promptSource")?.is_string().then(|| v.get("promptId")?.as_str().map(str::to_string))??));
        let repeat_prompt = prompt_id.is_some_and(|p| !prompts_seen.insert(p));
        // Claude's own error report reads to the old reader as prose; the
        // projector makes it the turn's failure instead (see
        // `where_the_projector_knows_more_it_says_so`).
        let api_error = line.contains("\"isApiErrorMessage\":true");
        for event in claude::parse_line(line) {
            match event {
                TurnEvent::Said { .. } if api_error => {}
                TurnEvent::Started { .. } if repeat_prompt => {}
                TurnEvent::Started { .. } => s.prompts += 1,
                TurnEvent::Said { text, conclusion } => s.prose.push((text, conclusion)),
                TurnEvent::Asked { id, .. } => s.questions.push(id),
                TurnEvent::Subagent { id, running, .. } => {
                    let entry = s.subagents.entry(id).or_insert((None, true));
                    entry.1 = running;
                }
                TurnEvent::SubagentLaunched { id, agent_id } => {
                    agent_of.insert(agent_id.clone(), id.clone());
                    s.subagents.entry(id).or_insert((None, true)).0 = Some(agent_id);
                }
                TurnEvent::SubagentEnded { agent_id, status } => {
                    let _: SubagentStatus = status;
                    if let Some(id) = agent_of.get(&agent_id) {
                        s.subagents.get_mut(id).expect("launched").1 = false;
                    }
                }
                TurnEvent::Ended { duration_ms: Some(ms), .. } => s.durations.push(ms),
                _ => {}
            }
        }
    }
    s
}

fn projector(text: &str) -> (Summary, Projection) {
    let p = fold(text);
    let mut s = Summary::default();
    for row in p.rows() {
        match &row.kind {
            // A prompt with no `promptSource` (a slash command's expansion) is a
            // turn to the projector and not to the old reader, which keys on
            // that field; both are counted alike by leaving those out.
            RowKind::Turn(t) if t.origin != TurnOrigin::Other => s.prompts += 1,
            RowKind::Prose(pr) => s.prose.push((pr.text.clone(), pr.conclusion)),
            RowKind::Ask(a) if a.kind == AskKind::Question => s.questions.push(row.id["ask:".len()..].to_string()),
            RowKind::Subagent(sub) => {
                s.subagents.insert(sub.tool_use_id.clone(), (sub.agent_id.clone(), sub.status == SubagentState::Running));
            }
            _ => {}
        }
    }
    // Durations, as the turns carry them; the old reader has one per
    // `turn_duration`, which the projector writes onto its turn.
    s.durations = text
        .lines()
        .filter(|l| l.contains("\"subtype\":\"turn_duration\""))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()?.get("durationMs")?.as_i64())
        .collect();
    (s, p)
}

fn assert_parity(name: &str, text: &str) -> Projection {
    let old = old_reader(text);
    let (new, p) = projector(text);
    assert_eq!(new.prompts, old.prompts, "{name}: prompts");
    assert_eq!(new.prose, old.prose, "{name}: prose, in order, closing answers marked alike");
    assert_eq!(new.questions, old.questions, "{name}: questions");
    assert_eq!(new.subagents, old.subagents, "{name}: subagents, their ids and whether they run");
    // Every duration the old reader reports is the one on a projector turn.
    let on_turns: Vec<i64> = turns(&p).iter().filter_map(|(_, t)| t.duration_ms).collect();
    for ms in &old.durations {
        assert!(on_turns.contains(ms), "{name}: turn_duration {ms} is on a turn: {on_turns:?}");
    }
    p
}

#[test]
fn the_projector_agrees_with_the_old_reader_on_every_fixture() {
    for (name, text) in [
        ("recorded", RECORDED),
        ("background", BACKGROUND),
        ("edits", EDITS),
        ("compact", COMPACT),
        ("nonmonotonic", NONMONOTONIC),
        ("unknown", UNKNOWN),
        ("cleared-after", CLEARED_AFTER),
        ("meta-prompt", META_PROMPT),
        ("queued-notification", QUEUED_NOTIFICATION),
        ("queue", QUEUE),
        ("errors-and-gaps", ERRORS_AND_GAPS),
    ] {
        assert_parity(name, text);
    }
}

/// Where the two differ, and why the projector's answer is the one a person
/// needs. Each is a fact the old reader cannot state.
#[test]
fn where_the_projector_knows_more_it_says_so() {
    // An interrupt: the old reader never ends the turn (no turn_duration).
    let open_after_interrupt = claude_turn_open(EDITS);
    assert!(open_after_interrupt, "the old reader leaves an interrupted turn open");
    assert_eq!(turn(&fold(EDITS), "turn:p2").outcome, Some(TurnOutcome::Interrupted));

    // A turn's end: the old reader waits for turn_duration, which 77% of
    // SDK sessions never write; `end_turn` is enough for the projector.
    let no_duration: String = COMPACT.lines().filter(|l| !l.contains("turn_duration")).map(|l| format!("{l}\n")).collect();
    assert!(claude_turn_open(&no_duration));
    assert_eq!(turn(&fold(&no_duration), "turn:p2").outcome, Some(TurnOutcome::Finished));
}

/// Whether the old reader's events leave a turn open at the end of `text`.
fn claude_turn_open(text: &str) -> bool {
    let mut open = false;
    for line in text.lines() {
        for event in claude::parse_line(line) {
            match event {
                TurnEvent::Started { .. } => open = true,
                TurnEvent::Ended { .. } => open = false,
                _ => {}
            }
        }
    }
    open
}

/// The whole recorded sandbox corpus, when it is on this machine: every main
/// transcript under `FARCOOLER_PROJECTOR_CORPUS` (a claude `projects/` dir).
/// Ignored by default; the lane's report gives its result.
#[test]
#[ignore]
fn the_projector_agrees_with_the_old_reader_on_a_recorded_corpus() {
    let Some(root) = std::env::var_os("FARCOOLER_PROJECTOR_CORPUS") else {
        eprintln!("SKIPPED: set FARCOOLER_PROJECTOR_CORPUS to a claude projects/ directory; nothing was compared");
        return;
    };
    let mut files = 0;
    for project in std::fs::read_dir(root).unwrap().flatten() {
        for entry in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                let text = std::fs::read_to_string(&path).unwrap();
                let p = assert_parity(&path.display().to_string(), &text);
                assert_eq!(p.stats().gaps, 0, "{}: a gap in a real session", path.display());
                files += 1;
            }
        }
    }
    eprintln!("parity held over {files} recorded sessions");
}

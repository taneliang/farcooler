//! Claude's transcript: `~/.claude/projects/<slug>/<session-uuid>.jsonl`.
//!
//! See `docs/agent-session-logs.md` for what every field named here was
//! observed to mean -- this file reads only the fields that document names,
//! and nothing else, so a field claude adds tomorrow breaks nothing today.

use serde_json::Value;

use super::{TaskStatus, TurnEvent, TurnOutcome};

/// Parse one line of a claude transcript into the shared turn vocabulary.
///
/// Returns a `Vec` rather than an `Option` because one line can carry more
/// than one fact -- see `turn_end` for the case that forces this. Most lines
/// still produce zero or one event; the collection exists for the rare line
/// that produces two, not to invite parsers to invent more.
///
/// Returns an empty `Vec` for anything this parser does not recognize: a line
/// that is not JSON, a record `type` the reference doc does not name, or a
/// shape that has drifted since the doc was written. An unrecognized line is
/// meant to fall through to the screen-scraping layer below, not to error --
/// these are private formats with no compatibility promise.
pub fn parse_line(line: &str) -> Vec<TurnEvent> {
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match record.get("type").and_then(Value::as_str) {
        Some("user") => user_record(&record),
        Some("assistant") => assistant_record(&record),
        Some("system") => turn_end(&record),
        Some("ai-title") => title(&record).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// A `user` record is either the human starting a turn or a tool coming back.
///
/// Both are asked, rather than one or the other: the two shapes have never
/// been seen on one line, but the whole reason `parse_line` returns a `Vec` is
/// that choosing between two facts on one line is how stage 2 lost its turn
/// ends, and there is nothing to gain from making that choice again here.
fn user_record(record: &Value) -> Vec<TurnEvent> {
    let mut events: Vec<TurnEvent> = turn_start(record).into_iter().collect();
    events.extend(tool_result(record));
    events
}

/// A turn start is `type: "user"` carrying `promptSource`. A tool result is
/// ALSO `type: "user"` -- it is told apart by having no `promptSource` at
/// all (`docs/agent-session-logs.md`, "Turn starts"). Checking PRESENCE
/// rather than value is deliberate and must stay that way: five values have
/// been counted on one machine (`typed`, `sdk`, `system`, `queued`,
/// `suggestion_accepted` -- see the reference doc for the counts), the brief's
/// own example named only the first, and a list is a closed set that the next
/// value claude invents falls straight out of. A turn that never starts is
/// worse than one started by a value nobody predicted.
fn turn_start(record: &Value) -> Option<TurnEvent> {
    record.get("promptSource")?;
    Some(TurnEvent::Started { at_ms: at_ms(record) })
}

/// What an assistant line said, and what it did: every `text` block in its
/// content, and the first `tool_use`.
///
/// One `assistant` line can carry `thinking` alone, `text` alone, or a
/// `tool_use`. A `tool_use` is always a `Did`; `text` is always a `Said`.
///
/// **It used to be `Said` only under `stop_reason == "end_turn"`, and that
/// gate is what left a claude row silent while anybody was watching it.**
/// Counted across the 40 largest transcripts on this machine: 5623 assistant
/// records carry a `text` block under `stop_reason == "tool_use"` -- the agent
/// saying what it is about to do, one line before it does it -- against 696
/// under `end_turn`. Eight narrating lines in nine were dropped, so the only
/// prose a transcript ever accepted was one that arrives when the turn is
/// already over. Correct for a list of the three most recent DISCRETE
/// messages, which is what the feed was; wrong for a window that is meant to
/// move.
///
/// The distinction is kept rather than erased: the LAST `text` block of an
/// `end_turn` message is the turn's answer and is marked as one, because a
/// summary is read from its start where narration is watched at its tail. See
/// `TurnEvent::Said`.
///
/// Only the FIRST `tool_use` becomes a `Did`, which is what this has always
/// done: an action line names one action, and a message carrying three
/// parallel calls would otherwise push two of them straight back out of a
/// three-deep feed. Every tool_use is still read for the signals BESIDE the
/// action -- a task moving, an agent spawning -- because those are counted
/// rather than displayed one per row, and a create dropped for being second
/// on its line is a total that is quietly wrong forever.
fn assistant_record(record: &Value) -> Vec<TurnEvent> {
    let Some(message) = record.get("message") else { return Vec::new() };
    let Some(content) = message.get("content").and_then(Value::as_array) else { return Vec::new() };
    let mut events = said(message, content);
    let tool_uses =
        content.iter().filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"));
    for (n, tool_use) in tool_uses.enumerate() {
        if n == 0 {
            events.extend(did(tool_use));
        }
        events.extend(spawned(tool_use));
        events.extend(task_state(tool_use));
    }
    events
}

/// A tool call as an action: the tool's name lowercased, and the object of
/// whatever it was pointed at.
fn did(tool_use: &Value) -> Option<TurnEvent> {
    let verb = tool_use.get("name")?.as_str()?.to_lowercase();
    let object = tool_use.get("input").and_then(step_object).unwrap_or_default();
    Some(TurnEvent::Did { verb, object })
}

/// Every `text` block on the record, in the order it was written.
///
/// The LAST one is the conclusion when the message ends the turn, and only
/// then. A message that ends a turn can open with a preamble and close with
/// the answer, and the answer is the half a person checking a finished pane
/// wants -- so the preamble goes in as narration ahead of it, which is exactly
/// where a reader would find it.
///
/// Whitespace-only prose yields nothing rather than an empty `Said`, which
/// would push a real line out of a three-line window to say nothing at all.
/// `thinking` is not read at all, here or anywhere: it is the agent talking to
/// itself, and it is not what it SENT.
fn said(message: &Value, content: &[Value]) -> Vec<TurnEvent> {
    let ends_turn = message.get("stop_reason").and_then(Value::as_str) == Some("end_turn");
    let texts: Vec<&str> = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .collect();
    let last = texts.len().saturating_sub(1);
    texts
        .into_iter()
        .enumerate()
        .map(|(n, text)| TurnEvent::Said {
            text: text.to_string(),
            conclusion: ends_turn && n == last,
        })
        .collect()
}

/// The object of a step, from whichever of `file_path` (basename only --
/// the object names WHAT was touched, not the full path to it), `command`,
/// `pattern`, or `description` appears first, in that order, in the tool's
/// `input`. Order follows the brief verbatim: a file-shaped tool names its
/// file first, a shell-shaped tool names what it ran, a search-shaped tool
/// names its pattern, and anything else falls back to its own description.
fn step_object(input: &Value) -> Option<String> {
    if let Some(path) = input.get("file_path").and_then(Value::as_str) {
        return Some(path.rsplit('/').next().unwrap_or(path).to_string());
    }
    for field in ["command", "pattern", "description"] {
        if let Some(s) = input.get(field).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

/// A `tool_use` named `Agent` spawns one, and its `id` is how everything else
/// refers to it: the matching `tool_result` carries it as `tool_use_id`, and
/// the sibling `subagents/agent-<agentId>.meta.json` carries it as
/// `toolUseId`. So this `id` is the join key for plan task 3, and nothing has
/// to guess which meta file belongs to which spawn.
///
/// `description` is read straight from the spawning call, against the plan's
/// expectation that it would not be there: of the 315 `Agent` calls on this
/// machine, all 315 carry `input.description`, and all 315 match the
/// `description` in the meta file that was written for them, character for
/// character. It is the same 3-5 word summary from the same source. Reading
/// it here means a spawn is nameable from the parent log alone, on the tick it
/// happens, without waiting for the sibling file to be found and opened.
fn spawned(tool_use: &Value) -> Option<TurnEvent> {
    if tool_use.get("name").and_then(Value::as_str) != Some("Agent") {
        return None;
    }
    let id = tool_use.get("id")?.as_str()?.to_string();
    let description = tool_use
        .get("input")
        .and_then(|input| input.get("description"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(TurnEvent::Subagent { id, description, running: true })
}

/// What a `Task*` call said about the agent's list.
///
/// The tally is NOT here, and cannot be. `TaskCreate` carries `subject`,
/// `description` and `activeForm` -- and no id, because claude assigns the id
/// in the tool RESULT (`toolUseResult.task.id`). `TaskUpdate` carries
/// `taskId` and `status` -- and no phrase, and no totals. `TaskList` carries
/// nothing at all going in; its result carries the whole list, and 6 of the
/// 340 sessions using tasks on this machine ever asked for one. So a running
/// `3/7 · Designing test matrix` can only be folded across lines, and this
/// parser is one line at a time by contract (`parse_line`), which settles
/// where the fold lives: the daemon, beside the turn state it already folds
/// (plan task 4).
///
/// What the fold is owed, and what it can do with it: a create arrives
/// carrying the phrase and no id, and the RESULT one line later states the id
/// (`created_task_id`). So the join is between a create and the very next
/// create result, not between a create's position in the turn and a number --
/// which is the rule the plan proposed, and which turned out to be true only
/// of a session's first turn. See `created_task_id` for what that cost when it
/// was tried live.
///
/// Three shapes reach the fold, told apart by which halves are present:
/// `id: None` with a phrase is a create; `id: Some` with no status is that
/// create being numbered; `id: Some` with a status is a task moving.
fn task_state(tool_use: &Value) -> Option<TurnEvent> {
    match tool_use.get("name").and_then(Value::as_str)? {
        "TaskCreate" => {
            let active_form =
                tool_use.get("input")?.get("activeForm").and_then(Value::as_str).map(str::to_string);
            // A create with no `activeForm` (34 of 289) still adds a task to
            // the total, so it is emitted with nothing but its existence --
            // dropping it would undercount the denominator a person reads.
            Some(TurnEvent::TaskState { id: None, active_form, status: None })
        }
        "TaskUpdate" => {
            let input = tool_use.get("input")?;
            let id = input.get("taskId").and_then(Value::as_str)?.to_string();
            // An update that changes only a description says nothing about
            // position (1 of 467 did), and an unrecognized status is worse
            // than silence -- both fall through to the action line.
            let status = task_status(input.get("status").and_then(Value::as_str)?)?;
            Some(TurnEvent::TaskState { id: Some(id), active_form: None, status: Some(status) })
        }
        // `TaskList` asks with an empty input; everything it learns arrives in
        // the result, which `tool_result` reads.
        _ => None,
    }
}

/// Claude's four observed `status` values, verbatim from the log.
///
/// Counted across every `TaskUpdate` on this machine: `completed` 251,
/// `in_progress` 214, `pending` 1, `deleted` 1. A fifth value claude invents
/// tomorrow yields `None` and leaves the task where the fold last saw it,
/// which is the conservative half of a bad choice: a stale phrase is a
/// smaller lie than a progress bar that moved for a reason nobody knows.
fn task_status(status: &str) -> Option<TaskStatus> {
    match status {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "completed" => Some(TaskStatus::Completed),
        "deleted" => Some(TaskStatus::Deleted),
        _ => None,
    }
}

/// The three tool results worth reading: a subagent's, a task list's, and the
/// id a create comes back with.
///
/// A `tool_result` block does not name the tool it came back from, so none of
/// these is found by name. All three are found by the shape of the sibling
/// `toolUseResult`: `agentId` appears on 313 results on this machine and every
/// one of them is an `Agent` call's; `tasks` is `TaskList`'s and only its;
/// `task.id` is `TaskCreate`'s and only its.
fn tool_result(record: &Value) -> Vec<TurnEvent> {
    let Some(result) = record.get("toolUseResult").filter(|value| value.is_object()) else {
        return Vec::new();
    };
    if result.get("agentId").is_some() {
        return finished_subagent(record, result).into_iter().collect();
    }
    if let Some(id) = created_task_id(result) {
        // Id and nothing else. A create result carries no `status` and no
        // `activeForm`, and that ABSENCE is what tells a fold this is a create
        // being numbered rather than a task moving -- see `task_state`.
        return vec![TurnEvent::TaskState { id: Some(id), active_form: None, status: None }];
    }
    let Some(tasks) = result.get("tasks").and_then(Value::as_array) else { return Vec::new() };
    tasks
        .iter()
        .filter_map(|task| {
            let id = task.get("id").and_then(Value::as_str)?.to_string();
            let status = task_status(task.get("status").and_then(Value::as_str)?)?;
            // A listed task states its `subject`, never an `activeForm` --
            // the phrase exists only on the call that created it.
            Some(TurnEvent::TaskState { id: Some(id), active_form: None, status: Some(status) })
        })
        .collect()
}

/// The id claude assigned to a task it has just created.
///
/// `{"task": {"id": "8", "subject": "Design test matrix"}}`, one line after the
/// `TaskCreate` that carried the phrase.
///
/// **This is read because the plan's join rule is wrong, measured live.** The
/// plan had it that the k-th task created comes back as id `"k"`, from a corpus
/// count of 289 creates. It holds for the FIRST turn of a session and no other:
/// task ids are numbered per SESSION, so a second turn that creates seven tasks
/// gets ids `"8"` through `"14"`. A fold counting creates per turn therefore
/// joined nothing at all from the second turn on -- and, worse, every update
/// naming an unrecognized id added a task, so a list of seven read `3/11`. Both
/// halves of a row wrong, on the second thing anybody would try.
///
/// The id being stated outright leaves nothing to infer from position except
/// which create it belongs to, and that one is a line apart rather than a turn.
fn created_task_id(result: &Value) -> Option<String> {
    Some(result.get("task")?.get("id")?.as_str()?.to_string())
}

/// An `Agent` call coming back -- which does NOT mean the subagent is over.
///
/// The plan had it that a subagent runs exactly until a `tool_result` with its
/// `tool_use_id` arrives. That is true for a foreground spawn and wrong for a
/// background one: `run_in_background: true` gets its result back IMMEDIATELY,
/// carrying `status: "async_launched"` and an `outputFile` to read later,
/// while the agent keeps going. 104 of the 313 spawns on this machine ended
/// that way, a third of them, and reading those as finished would have shown a
/// fleet as done the instant it was launched -- the exact failure the count is
/// meant to catch. `completed` is the end; `async_launched` is the start
/// restated.
///
/// Any other status is read as the end. An unknown status leaving a subagent
/// running forever is the worse half: the count only ever grows, and no
/// tool_result at all is what a genuinely running agent already looks like.
fn finished_subagent(record: &Value, result: &Value) -> Option<TurnEvent> {
    let id = record
        .get("message")?
        .get("content")?
        .as_array()?
        .iter()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))?
        .get("tool_use_id")?
        .as_str()?
        .to_string();
    let running = result.get("status").and_then(Value::as_str) == Some("async_launched");
    // Only a background launch restates the description; a completed one
    // carries `agentType` instead. Empty is honest -- the spawn already named
    // it, under the same id.
    let description =
        result.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
    Some(TurnEvent::Subagent { id, description, running })
}

/// A turn end is `type: "system"`, `subtype: "turn_duration"` -- not the
/// tentative `stop_hook_summary` system record that can precede it.
///
/// It is NOT always written. 243 of the 315 session files with a turn start
/// on the machine this was measured on contain none at all, most of them
/// programmatic `sdk-cli` sessions but not all (`docs/agent-session-logs.md`,
/// "Turn ends"). Nothing here has to change for that -- a parser reads what
/// it is given -- but a CALLER that waits for this record to close a turn
/// will wait forever on those sessions, which is the same hazard codex's
/// unmatched `task_started` carries.
///
/// `pendingBackgroundAgentCount` is present only when it is above zero (the
/// key is absent, not zero, when nothing is running -- see the reference
/// doc's "Turn ends"). When it IS present, the SAME line is reporting two
/// independent facts: the foreground turn just ended, AND background agents
/// are still running. Both are true at once and a reader needs both -- if
/// `Ended` were dropped in favor of `BackgroundAgents` (or the reverse), a
/// fleet of background agents would either never be reported, or a pane
/// running one would never be seen as Idle again. This is exactly why
/// `parse_line` returns a `Vec`: `Ended` always comes first (the turn ending
/// is the primary fact; `BackgroundAgents` is additional context about what's
/// still outstanding), followed by `BackgroundAgents` when the key is
/// present. A plain `turn_duration` with no such key -- the overwhelming
/// majority, since the key is rare by construction -- yields only `Ended`.
fn turn_end(record: &Value) -> Vec<TurnEvent> {
    if record.get("subtype").and_then(Value::as_str) != Some("turn_duration") {
        return Vec::new();
    }
    let mut events = vec![TurnEvent::Ended {
        at_ms: at_ms(record),
        duration_ms: record.get("durationMs").and_then(Value::as_i64),
        outcome: TurnOutcome::Finished,
    }];
    if let Some(count) = record.get("pendingBackgroundAgentCount").and_then(Value::as_u64) {
        events.push(TurnEvent::BackgroundAgents(count as u32));
    }
    events
}

/// `aiTitle` is re-emitted at every checkpoint and never changes within a
/// session (`docs/agent-session-logs.md`, "Fields worth reading"), so the
/// last one read is always the current one -- callers do not need to special
/// case which `ai-title` line in a session "counts".
fn title(record: &Value) -> Option<TurnEvent> {
    Some(TurnEvent::Title(record.get("aiTitle")?.as_str()?.to_string()))
}

fn at_ms(record: &Value) -> Option<i64> {
    record.get("timestamp")?.as_str().and_then(parse_iso8601_millis)
}

/// `2026-08-16T07:53:34.179Z` to unix millis.
///
/// Hand-rolled rather than adding a date-time crate to `farcooler-core`,
/// which has none today (see the `short_hash` decision in this same module's
/// parent for the precedent): this whole file already reads `serde_json`
/// fields by hand instead of deriving, so one more field read by hand is not
/// an outlier, and a crate whose only caller is one timestamp field would be.
fn parse_iso8601_millis(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 20 {
        return None;
    }
    let field = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse().ok() };
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);

    let millis = if bytes.get(19) == Some(&b'.') {
        let digits: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).collect();
        let mut padded = digits;
        padded.truncate(3);
        while padded.len() < 3 {
            padded.push('0');
        }
        padded.parse().unwrap_or(0)
    } else {
        0
    };

    // Days since the epoch, by the civil-from-days algorithm (Howard Hinnant's
    // `days_from_civil`), the same shape used in `crates/daemon/src/stack.rs`
    // for the same reason: one field, hand-rolled rather than a dependency.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_shifted = (month + 9) % 12;
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;

    Some(((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000) + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE_TURN: &str = include_str!("../../fixtures/session-logs/claude-complete-turn.jsonl");
    const OUT_OF_ORDER: &str = include_str!("../../fixtures/session-logs/claude-out-of-order-timestamps.jsonl");
    const TASK_LIST: &str = include_str!("../../fixtures/session-logs/claude-task-list.jsonl");
    const SUBAGENTS: &str = include_str!("../../fixtures/session-logs/claude-subagents.jsonl");

    fn line(fixture: &str, n: usize) -> &str {
        fixture.lines().nth(n).expect("fixture has that many lines")
    }

    #[test]
    fn a_typed_prompt_starts_a_turn() {
        // Line 1: the opening `user` record, `promptSource: "typed"`.
        match parse_line(line(COMPLETE_TURN, 0)).as_slice() {
            [TurnEvent::Started { at_ms }] => assert_eq!(*at_ms, Some(1_786_866_814_179)),
            other => panic!("expected [Started], got {other:?}"),
        }
    }

    #[test]
    fn a_non_typed_prompt_source_still_starts_a_turn() {
        // Line 5 of the out-of-order fixture: `promptSource: "sdk"`, not
        // `"typed"`. The brief's own example is `typed`, but the doc says the
        // distinction is PRESENCE of `promptSource`, not which value it holds.
        match parse_line(line(OUT_OF_ORDER, 4)).as_slice() {
            [TurnEvent::Started { .. }] => {}
            other => panic!("expected [Started], got {other:?}"),
        }
    }

    #[test]
    fn a_tool_result_is_not_a_turn_start() {
        // Line 4: also `type: "user"`, but a tool result -- `message.content[].type
        // == "tool_result"` and no `promptSource`. This is the distinction the
        // brief calls the single most important one in the task: get it wrong
        // and every tool call looks like a new turn.
        let record: Value = serde_json::from_str(line(COMPLETE_TURN, 3)).unwrap();
        assert_eq!(record["type"], "user");
        assert_eq!(record["message"]["content"][0]["type"], "tool_result");
        assert!(record.get("promptSource").is_none());
        assert_eq!(parse_line(line(COMPLETE_TURN, 3)), Vec::new());
    }

    #[test]
    fn turn_duration_ends_a_turn_with_its_duration() {
        // Line 7: `system`/`turn_duration`, `durationMs: 14681`, no
        // `pendingBackgroundAgentCount` -- exactly one event, and it's Ended.
        match parse_line(line(COMPLETE_TURN, 6)).as_slice() {
            [TurnEvent::Ended { duration_ms, outcome, .. }] => {
                assert_eq!(*duration_ms, Some(14681));
                assert_eq!(*outcome, TurnOutcome::Finished);
            }
            other => panic!("expected [Ended], got {other:?}"),
        }
    }

    #[test]
    fn a_stop_hook_summary_is_not_a_turn_end() {
        // Line 6: `system`, but `subtype: "stop_hook_summary"`, not
        // `turn_duration`. Only the latter is the documented turn-end record.
        assert_eq!(parse_line(line(COMPLETE_TURN, 5)), Vec::new());
    }

    #[test]
    fn pending_background_agents_arrives_alongside_ended_not_instead_of_it() {
        // Synthetic, not drawn from a fixture: `pendingBackgroundAgentCount` is
        // rare by construction (only written above zero), so no captured
        // fixture line happens to carry it. This is the regression the
        // coordinator flagged in fix round 1 -- a `turn_duration` line with
        // background agents pending must still end the turn, or a pane
        // running a fleet of background agents would sit on Working forever,
        // which is the exact failure this whole log-reading stage exists to
        // remove. Assert `Ended` is present, not just that the vec is non-empty.
        let synthetic = r#"{"type":"system","subtype":"turn_duration","durationMs":9000,"messageCount":4,"pendingBackgroundAgentCount":3,"timestamp":"2026-08-16T07:55:01.865Z"}"#;
        let events = parse_line(synthetic);
        assert_eq!(
            events,
            vec![
                TurnEvent::Ended { at_ms: Some(1_786_866_901_865), duration_ms: Some(9000), outcome: TurnOutcome::Finished },
                TurnEvent::BackgroundAgents(3),
            ]
        );
        assert!(events.iter().any(|e| matches!(e, TurnEvent::Ended { .. })), "Ended must survive alongside BackgroundAgents");
    }

    #[test]
    fn pending_background_agents_is_absent_not_zero() {
        // The real fixture line has no `pendingBackgroundAgentCount` key at
        // all -- reading a missing key as zero would be wrong for every idle
        // session, so the absence must yield exactly one event (`Ended`),
        // never a phantom `BackgroundAgents(0)` alongside it.
        assert_eq!(parse_line(line(COMPLETE_TURN, 6)).len(), 1);
        match parse_line(line(COMPLETE_TURN, 6)).as_slice() {
            [TurnEvent::Ended { .. }] => {}
            other => panic!("expected exactly [Ended], no BackgroundAgents event: {other:?}"),
        }
    }

    #[test]
    fn a_tool_use_becomes_an_action_named_from_its_file_path() {
        // Line 3: `assistant`, `tool_use`, `name: "Write"`,
        // `input.file_path: "/Users/example/project/haiku.txt"` -- the verb is
        // the tool name lowercased, the object is the basename, not the full path.
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 2)),
            vec![TurnEvent::Did { verb: "write".to_string(), object: "haiku.txt".to_string() }]
        );
    }

    #[test]
    fn the_closing_prose_of_a_turn_is_what_the_agent_said() {
        // Line 5: the real fixture's closing `assistant` record -- one `text`
        // block, `stop_reason: "end_turn"`. Without this, a finished claude
        // pane's transcript ended on `write haiku.txt` and never said what the
        // agent concluded, while codex and cursor panes both did.
        match parse_line(line(COMPLETE_TURN, 4)).as_slice() {
            [TurnEvent::Said { text, conclusion }] => {
                assert!(text.starts_with("Written to `haiku.txt`."), "{text}");
                assert!(*conclusion, "an end_turn message's last prose is the answer");
            }
            other => panic!("expected [Said], got {other:?}"),
        }
    }

    /// The finding this whole stage exists for, pinned on the shape claude
    /// really writes it in.
    ///
    /// This is the agent saying what it is ABOUT to do, one line before it
    /// does it -- `stop_reason: "tool_use"`, because a tool call follows in
    /// the next record. It outnumbers closing prose eight to one, and dropping
    /// it is what left a row with nothing to say for the entire time somebody
    /// was watching it work.
    #[test]
    fn mid_turn_narration_is_what_the_agent_said_too() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Now I'll run the tests."}],"stop_reason":"tool_use"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Said { text: "Now I'll run the tests.".to_string(), conclusion: false }],
            "narration reaches the transcript, and is not mistaken for the answer"
        );
    }

    #[test]
    fn a_text_block_with_no_stop_reason_at_all_is_narration() {
        // Streaming records and older CLI versions write `content` with no
        // `stop_reason` beside it. Absent is not `end_turn` -- the prose is
        // still what the agent said, but nothing has claimed it concludes
        // anything, so nothing here claims it either.
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Looking into it."}]}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Said { text: "Looking into it.".to_string(), conclusion: false }]
        );
    }

    #[test]
    fn the_last_text_block_is_the_answer_when_a_closing_message_has_several() {
        // A message that ends a turn can open with a preamble and close with
        // the answer. Both are what the agent said, in that order; only the
        // second is the conclusion.
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me summarize."},{"type":"text","text":"Both tests pass."}],"stop_reason":"end_turn"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![
                TurnEvent::Said { text: "Let me summarize.".to_string(), conclusion: false },
                TurnEvent::Said { text: "Both tests pass.".to_string(), conclusion: true },
            ]
        );
    }

    #[test]
    fn empty_closing_prose_yields_nothing_rather_than_an_empty_line() {
        // A `Said` with no text would still push a real line out of a
        // three-line window, to say nothing.
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"   "}],"stop_reason":"end_turn"}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    #[test]
    fn prose_and_a_tool_use_in_one_message_are_both_read() {
        // Rare -- 2 records in the 40 largest transcripts on this machine put
        // both in one message -- but the two facts are independent, and the
        // parser that had to choose between them chose the tool call and threw
        // the sentence away.
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Running it."},{"type":"tool_use","name":"Bash","input":{"command":"git status"}}],"stop_reason":"tool_use"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![
                TurnEvent::Said { text: "Running it.".to_string(), conclusion: false },
                TurnEvent::Did { verb: "bash".to_string(), object: "git status".to_string() },
            ]
        );
    }

    #[test]
    fn a_thinking_only_assistant_line_is_not_a_step() {
        // Line 2: `assistant`, content is `thinking` only, no `tool_use` block.
        assert_eq!(parse_line(line(COMPLETE_TURN, 1)), Vec::new());
    }

    #[test]
    fn a_bash_step_is_named_from_its_command_not_its_description() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"git status","description":"Check working tree status"}}]}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Did { verb: "bash".to_string(), object: "git status".to_string() }]
        );
    }

    #[test]
    fn a_grep_step_falls_back_to_its_pattern() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Grep","input":{"pattern":"TODO"}}]}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Did { verb: "grep".to_string(), object: "TODO".to_string() }]
        );
    }

    // -----------------------------------------------------------------
    // The task list -- what the row shows instead of `taskupdate`
    // -----------------------------------------------------------------

    /// The phrase the whole feature rests on, read off a real create.
    #[test]
    fn a_task_create_carries_the_agents_own_present_tense_phrase() {
        // Line 1 of the task fixture: `TaskCreate`, `activeForm: "Designing
        // test matrix"`. The create is BOTH an action and a task fact, and a
        // line carrying two facts emits both.
        match parse_line(line(TASK_LIST, 0)).as_slice() {
            [TurnEvent::Did { verb, .. }, TurnEvent::TaskState { id, active_form, status }] => {
                assert_eq!(verb, "taskcreate");
                assert_eq!(*id, None, "claude assigns the id in the RESULT, not here");
                assert_eq!(active_form.as_deref(), Some("Designing test matrix"));
                assert_eq!(*status, None, "a create says nothing about where the task sits");
            }
            other => panic!("expected [Did, TaskState], got {other:?}"),
        }
    }

    /// The create's RESULT states the id, and that is the join.
    ///
    /// Read rather than skipped, which is the correction this stage's live run
    /// forced: ids are numbered per SESSION, so counting creates within a turn
    /// joins nothing from the second turn on. See `created_task_id`.
    ///
    /// It carries the id and NOTHING else — no status, no `activeForm` — and
    /// that absence is how a fold tells it from a task moving.
    #[test]
    fn a_task_creates_result_states_the_id_the_create_did_not() {
        let record: Value = serde_json::from_str(line(TASK_LIST, 1)).unwrap();
        assert_eq!(record["toolUseResult"]["task"]["id"], "1", "the fixture really does state the id");
        assert_eq!(
            parse_line(line(TASK_LIST, 1)),
            vec![TurnEvent::TaskState { id: Some("1".to_string()), active_form: None, status: None }]
        );
    }

    #[test]
    fn a_task_update_names_the_task_and_where_it_went() {
        // Line 5: `TaskUpdate`, `{"taskId": "1", "status": "in_progress"}` --
        // the record that used to reach a lock screen as `taskupdate`.
        match parse_line(line(TASK_LIST, 4)).as_slice() {
            [TurnEvent::Did { .. }, TurnEvent::TaskState { id, active_form, status }] => {
                assert_eq!(id.as_deref(), Some("1"));
                assert_eq!(*active_form, None, "an update carries no phrase; the create did");
                assert_eq!(*status, Some(TaskStatus::InProgress));
            }
            other => panic!("expected [Did, TaskState], got {other:?}"),
        }
        // Line 7 completes the same task.
        match parse_line(line(TASK_LIST, 6)).as_slice() {
            [_, TurnEvent::TaskState { id, status, .. }] => {
                assert_eq!(id.as_deref(), Some("1"));
                assert_eq!(*status, Some(TaskStatus::Completed));
            }
            other => panic!("expected [Did, TaskState], got {other:?}"),
        }
    }

    /// The one record that states a whole list -- rare, and worth reading
    /// exactly because the fold's own count can be corrected by it.
    #[test]
    fn a_task_list_result_states_every_task_it_carries() {
        // Line 9: the `TaskList` result, five tasks, all completed. The
        // fixture keeps only the first two creates (see the fixtures README),
        // which is what an excerpt of a longer session looks like.
        let events = parse_line(line(TASK_LIST, 8));
        assert_eq!(events.len(), 5, "one per listed task: {events:?}");
        assert!(
            events.iter().all(|event| matches!(
                event,
                TurnEvent::TaskState { id: Some(_), status: Some(TaskStatus::Completed), .. }
            )),
            "{events:?}"
        );
        // The list's own tool_use carries an empty input and says nothing.
        assert_eq!(
            parse_line(line(TASK_LIST, 7)),
            vec![TurnEvent::Did { verb: "tasklist".to_string(), object: String::new() }]
        );
    }

    /// 34 of the 289 creates on this machine carried no `activeForm`. The
    /// task still exists, and a denominator that skipped it would be wrong.
    #[test]
    fn a_create_with_no_active_form_still_counts_as_a_task() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"TaskCreate","input":{"subject":"Ship it","description":"Ship the thing"}}]}}"#;
        match parse_line(synthetic).as_slice() {
            [_, TurnEvent::TaskState { active_form, .. }] => assert_eq!(*active_form, None),
            other => panic!("expected [Did, TaskState], got {other:?}"),
        }
    }

    /// A status nobody has seen moves nothing. The action line still reports
    /// the call, so the pane does not go quiet -- only the count holds still.
    #[test]
    fn an_unrecognized_status_leaves_the_task_where_it_was() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"TaskUpdate","input":{"taskId":"3","status":"blocked_on_a_human"}}]}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Did { verb: "taskupdate".to_string(), object: String::new() }]
        );
    }

    /// The action line names ONE action, but every task fact on the line is
    /// counted. A message carrying two creates is two tasks, and dropping the
    /// second would leave a denominator quietly wrong for the rest of the
    /// session.
    #[test]
    fn two_creates_in_one_message_are_two_tasks_and_one_action() {
        let synthetic = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"TaskCreate","input":{"subject":"a","activeForm":"Doing a"}},{"type":"tool_use","name":"TaskCreate","input":{"subject":"b","activeForm":"Doing b"}}]}}"#;
        let events = parse_line(synthetic);
        assert_eq!(events.iter().filter(|e| matches!(e, TurnEvent::Did { .. })).count(), 1);
        let forms: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                TurnEvent::TaskState { active_form, .. } => active_form.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(forms, vec!["Doing a", "Doing b"]);
    }

    // -----------------------------------------------------------------
    // Subagents
    // -----------------------------------------------------------------

    /// A spawn names itself, in the parent log, on the tick it happens.
    ///
    /// The plan expected the name to be readable only from the sibling
    /// `subagents/*.meta.json`. It is in both, identically: all 315 `Agent`
    /// calls on this machine carry `input.description`, and all 315 match the
    /// meta file written for them.
    #[test]
    fn spawning_an_agent_names_it_and_starts_it_running() {
        // Line 1 of the subagents fixture: `run_in_background: false`.
        match parse_line(line(SUBAGENTS, 0)).as_slice() {
            [TurnEvent::Did { verb, .. }, TurnEvent::Subagent { id, description, running }] => {
                assert_eq!(verb, "agent");
                assert_eq!(id, "toolu_01V5MTb3GuRtNbUVUSkSgWDz", "the id the meta file joins on");
                assert_eq!(description, "Fix Codex native backend gaps");
                assert!(*running);
            }
            other => panic!("expected [Did, Subagent], got {other:?}"),
        }
    }

    #[test]
    fn a_completed_agent_result_ends_the_one_it_names() {
        // Line 2: the matching `tool_result`, `toolUseResult.status:
        // "completed"`. The id is the SAME `tool_use_id`, which is what lets a
        // fold that never saw the two lines together pair them.
        match parse_line(line(SUBAGENTS, 1)).as_slice() {
            [TurnEvent::Subagent { id, running, .. }] => {
                assert_eq!(id, "toolu_01V5MTb3GuRtNbUVUSkSgWDz");
                assert!(!*running);
            }
            other => panic!("expected [Subagent], got {other:?}"),
        }
    }

    /// The finding that breaks the plan's liveness rule, on the real record.
    ///
    /// A background spawn's `tool_result` arrives at LAUNCH, seconds after the
    /// call, while the agent runs for another hour. 104 of the 313 spawns on
    /// this machine came back this way. "A result arrived, so it is done"
    /// would have shown a fleet as finished the moment it started.
    #[test]
    fn a_background_launch_is_not_an_ending() {
        // Line 3 spawns with `run_in_background: true`; line 4 is its result,
        // 1.9 seconds later, `status: "async_launched"`.
        match parse_line(line(SUBAGENTS, 2)).as_slice() {
            [_, TurnEvent::Subagent { id, running, .. }] => {
                assert_eq!(id, "toolu_015abdB6hcuQYTnrL45JDDYm");
                assert!(*running);
            }
            other => panic!("expected [Did, Subagent], got {other:?}"),
        }
        match parse_line(line(SUBAGENTS, 3)).as_slice() {
            [TurnEvent::Subagent { id, description, running }] => {
                assert_eq!(id, "toolu_015abdB6hcuQYTnrL45JDDYm");
                assert!(*running, "the agent is still going; only its launch came back");
                assert_eq!(description, "Document the native backend", "restated on an async launch");
            }
            other => panic!("expected [Subagent], got {other:?}"),
        }
    }

    /// A status this parser has not seen ends the subagent rather than
    /// leaving it running forever -- see `finished_subagent`.
    #[test]
    fn an_unrecognized_agent_status_ends_it() {
        let synthetic = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_x","type":"tool_result","content":"failed"}]},"toolUseResult":{"status":"errored","agentId":"a000000000000000c"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Subagent { id: "toolu_x".to_string(), description: String::new(), running: false }]
        );
    }

    /// Every other tool result stays silent. `Write`'s result is the one in
    /// the complete-turn fixture, and it carries neither `agentId` nor
    /// `tasks` -- the two shapes `tool_result` reads.
    #[test]
    fn an_ordinary_tool_result_is_not_a_subagent() {
        let record: Value = serde_json::from_str(line(COMPLETE_TURN, 3)).unwrap();
        assert!(record["toolUseResult"].is_object(), "the fixture line really has one");
        assert_eq!(parse_line(line(COMPLETE_TURN, 3)), Vec::new());
    }

    #[test]
    fn an_ai_title_record_yields_title() {
        // Dropped from the fixture as noise per the fixtures' README, so this
        // is constructed from the reference doc's own worked example: pane
        // title `Write haiku about lighthouse` equals `aiTitle` exactly.
        let synthetic = r#"{"type":"ai-title","aiTitle":"Write haiku about lighthouse","sessionId":"00000000-0000-4000-8000-000000000004"}"#;
        assert_eq!(parse_line(synthetic), vec![TurnEvent::Title("Write haiku about lighthouse".to_string())]);
    }

    #[test]
    fn a_non_json_line_yields_an_empty_vec_and_does_not_panic() {
        assert_eq!(parse_line("this is not json at all {"), Vec::new());
        assert_eq!(parse_line(""), Vec::new());
    }

    #[test]
    fn an_unlisted_record_type_yields_an_empty_vec() {
        // `docs/agent-session-logs.md` names the record types actually seen.
        // Anything else is either drift or invention, and both must fall
        // through rather than be guessed at.
        let synthetic = r#"{"type":"some-future-record-type","payload":{}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    #[test]
    fn a_listed_but_unhandled_record_type_yields_an_empty_vec() {
        // `agent-name` is named in the reference doc's "Other record types
        // seen" but carries nothing this parser's vocabulary maps to.
        let synthetic = r#"{"type":"agent-name","name":"scratch"}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    #[test]
    fn out_of_order_timestamps_do_not_confuse_a_single_line_parse() {
        // The whole point of this fixture: line 2 (`hook_success` attachment,
        // an unhandled type) has a timestamp 35 seconds BEFORE line 1's. This
        // parser reads one line at a time and must not care -- there is no
        // sorting or buffering here to get confused in the first place, which
        // is the proof: both lines parse independently of their order.
        assert_eq!(parse_line(line(OUT_OF_ORDER, 0)), Vec::new()); // queue-operation
        assert_eq!(parse_line(line(OUT_OF_ORDER, 1)), Vec::new()); // queue-operation
        assert_eq!(parse_line(line(OUT_OF_ORDER, 2)), Vec::new()); // attachment/hook_success
    }

    #[test]
    fn iso8601_with_millis_matches_a_real_claude_timestamp() {
        assert_eq!(parse_iso8601_millis("2026-08-16T07:53:34.179Z"), Some(1_786_866_814_179));
    }

    #[test]
    fn a_malformed_timestamp_is_none_rather_than_zero() {
        assert_eq!(parse_iso8601_millis("not a date"), None);
        assert_eq!(parse_iso8601_millis(""), None);
    }
}

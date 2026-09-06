//! Codex's rollout: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
//!
//! See `docs/agent-session-logs.md` for what every field named here was
//! observed to mean -- this file reads only the fields that document names,
//! and nothing else, so a field codex adds tomorrow breaks nothing today.
//!
//! Codex is the richest of the three: it states the turn outright, in
//! `task_started` and `task_complete`, rather than requiring the inference
//! claude and cursor need.
//!
//! It is also the poorest in one place, and that place is subagents. Codex
//! runs them -- a `collaboration` namespace of `spawn_agent`, `send_message`,
//! `followup_task`, `wait_agent`, `list_agents` -- and its own interface shows
//! them in no list anywhere, which is the whole reason it is worth reading
//! them out of here. What it writes down is every spawn and every exchange,
//! and no completion at all: see `sub_agent_activity` for the vocabulary and
//! `agent_roster` for the one record that ever ends one.

use serde_json::Value;

use super::{TurnEvent, TurnOutcome};

/// Parse one line of a codex rollout into the shared turn vocabulary.
///
/// Returns a `Vec` rather than an `Option` for the same reason as claude's
/// parser: the shared vocabulary is a `Vec` because some record can carry two
/// facts. No such record has been observed for codex yet, but the signature
/// stays a `Vec` so a future one costs nothing to add here.
///
/// Returns an empty `Vec` for anything this parser does not recognize: a line
/// that is not JSON, an outer `type` this parser does not read, or a
/// `payload.type` the reference doc does not name. An unrecognized line is
/// meant to fall through to the screen-scraping layer below, not to error --
/// these are private formats with no compatibility promise.
pub fn parse_line(line: &str) -> Vec<TurnEvent> {
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match record.get("type").and_then(Value::as_str) {
        // Almost every turn-shaped fact codex writes is nested under
        // `event_msg`'s own `payload.type` (`docs/agent-session-logs.md`,
        // "Turn starts" / "Turn ends" / "Other payloads").
        Some("event_msg") => event_msg(&record),
        // The one exception, and the reason this is a `match` rather than the
        // single `event_msg` guard it used to be: the ROSTER of a session's
        // subagents is a tool RESULT, so it arrives as `response_item` and
        // nowhere else. It is the only record codex writes that says a
        // subagent has stopped -- see `agent_roster`.
        Some("response_item") => response_item(&record),
        // `session_meta`, `turn_context`, `world_state` and the rest are
        // sibling top-level `type`s with no fact this parser reads.
        _ => Vec::new(),
    }
}

/// The `event_msg` half of `parse_line`.
fn event_msg(record: &Value) -> Vec<TurnEvent> {
    let Some(payload) = record.get("payload") else {
        return Vec::new();
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("task_started") => task_started(payload).into_iter().collect(),
        Some("task_complete") => task_complete(payload).into_iter().collect(),
        Some("turn_aborted") => turn_aborted(record).into_iter().collect(),
        Some("agent_message") => agent_message(payload).into_iter().collect(),
        Some("sub_agent_activity") => sub_agent_activity(payload).into_iter().collect(),
        Some("item_completed") => item_completed(payload).into_iter().collect(),
        // `token_count` is real and observed, but Task 10 is what reads it --
        // inventing an event for it here would be a field with no consumer.
        // `user_message` and anything else undocumented fall through the same
        // way.
        _ => Vec::new(),
    }
}

/// A turn start is `event_msg`/`task_started`, carrying `turn_id`,
/// `started_at`, `model_context_window` (`docs/agent-session-logs.md`, "Turn
/// starts"). Only `started_at` is read here -- `model_context_window` is the
/// field this task was told to leave alone (see the module doc's note on
/// `task_started` vs. `session_meta` naming the context window differently).
///
/// `started_at` is unix SECONDS ("Units, and this is a trap" in the
/// reference doc), so `at_ms` must multiply by 1000 -- reading it straight
/// through would understate every timestamp by three orders of magnitude.
fn task_started(payload: &Value) -> Option<TurnEvent> {
    let started_at = payload.get("started_at")?.as_i64()?;
    Some(TurnEvent::Started { at_ms: Some(started_at * 1000) })
}

/// A turn end is `event_msg`/`task_complete`, carrying a matching `turn_id`,
/// `completed_at`, `duration_ms`, `time_to_first_token_ms`, and
/// `last_agent_message` (`docs/agent-session-logs.md`, "Turn ends").
///
/// `completed_at` is unix SECONDS like `started_at`, so it gets the same
/// `* 1000`. `duration_ms` is already milliseconds IN THE SAME PAYLOAD --
/// the trap this task exists to catch -- so it is read straight through with
/// no multiplication.
fn task_complete(payload: &Value) -> Option<TurnEvent> {
    let at_ms = payload.get("completed_at").and_then(Value::as_i64).map(|seconds| seconds * 1000);
    let duration_ms = payload.get("duration_ms").and_then(Value::as_i64);
    Some(TurnEvent::Ended { at_ms, duration_ms, outcome: TurnOutcome::Finished })
}

/// A turn can end without completing: `turn_aborted` carries `reason` and is
/// a real terminal state (`docs/agent-session-logs.md`, "Turn ends"). No
/// fixture and no timestamp field is named for this payload -- unlike
/// `task_complete`'s `completed_at` -- so `at_ms` and `duration_ms` are left
/// `None` rather than guessed at from a shape nobody has observed. What
/// matters is that the turn is over: a reader waiting on this pane must stop
/// waiting for a `task_complete` that is never coming.
fn turn_aborted(_record: &Value) -> Option<TurnEvent> {
    Some(TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Aborted })
}

/// The agent's own words are `event_msg`/`agent_message`. `phase:
/// "final_answer"` is the conclusion; every other phase is narration on the
/// way to it, and both are what the agent said.
///
/// A missing `message` is nothing to say, not an empty line.
fn agent_message(payload: &Value) -> Option<TurnEvent> {
    let conclusion = payload.get("phase").and_then(Value::as_str) == Some("final_answer");
    let message = payload.get("message")?.as_str()?.trim().to_string();
    (!message.is_empty()).then_some(TurnEvent::Said { text: message, conclusion })
}

/// One of this session's subagents, seen doing something.
///
/// Codex spawns agents through its `collaboration` tools -- `spawn_agent`
/// naming a `task_name`, then `send_message`, `followup_task`, `wait_agent`
/// and `list_agents` -- and writes each spawn and each exchange down as this
/// record. Both shapes it uses reach here: `event_msg`/`sub_agent_activity`
/// (137 records on the machine this was read from) and, since 0.147.0, the
/// same fields as an `item_completed` item of type `SubAgentActivity`.
///
/// **The `kind` vocabulary is `started` and `interacted`, and NOTHING ELSE.**
/// Counted across every rollout here: 20 `started`, 117 `interacted`, zero of
/// any third value. There is no `finished`, no `completed`, no `exited`. So
/// this record can open a subagent and can never close one, and the close has
/// to come from somewhere else -- `agent_roster`, which is the only place
/// codex ever states that an agent has stopped.
///
/// `interacted` is read as `running: true` on purpose, and it is the one
/// INFERENCE here rather than something observed: the record says the parent
/// exchanged a message with that agent, not that the agent is busy this
/// instant. It is read anyway because without it this feature would show
/// nothing on the sessions it exists for. Every one of the eight
/// subagent-bearing rollouts on this machine is over `tail::
/// READ_FROM_START_BYTES` -- the smallest is 1.02 MB and the largest 92 MB --
/// so such a pane is ALWAYS attached to at its end, and the `started` lines
/// are already behind that point. `interacted` is then the only evidence left
/// in the file that a fleet exists at all. The inference is bounded at both
/// ends: a roster naming the agent as finished overrides it, and the daemon's
/// fold drops every spawn at a turn boundary anyway (`Signals::
/// forget_the_turn`), so nothing here can outlive the turn that wrote it.
///
/// An unrecognized `kind` yields nothing rather than a guess. If codex ever
/// adds a terminal one, this is the arm to add it to -- and it would be a far
/// better close than the roster is.
fn sub_agent_activity(record: &Value) -> Option<TurnEvent> {
    let path = record.get("agent_path")?.as_str()?;
    let name = sub_agent_name(path)?;
    match record.get("kind").and_then(Value::as_str)? {
        "started" | "interacted" => Some(TurnEvent::Subagent {
            id: path.to_string(),
            description: name.to_string(),
            running: true,
        }),
        _ => None,
    }
}

/// The `response_item` half of `parse_line`, which exists for one record.
///
/// A `function_call_output` does not name the tool it came back from -- it
/// carries a `call_id` and an `output` string and nothing else -- and this
/// parser is one line at a time by contract, so the `list_agents` call that
/// asked for it is long gone by the time its answer arrives. The roster is
/// therefore recognized by its SHAPE: an object with an `agents` array whose
/// entries carry `agent_name` and `agent_status`. Nothing else codex writes
/// through this field has that shape, and a command whose stdout happened to
/// be exactly that JSON would be reporting the same fact anyway.
fn response_item(record: &Value) -> Vec<TurnEvent> {
    let Some(payload) = record.get("payload") else {
        return Vec::new();
    };
    if payload.get("type").and_then(Value::as_str) != Some("function_call_output") {
        return Vec::new();
    }
    // `output` is a STRING carrying JSON, not nested JSON -- 224 of the 226
    // outputs on this machine are strings and the other two are arrays of
    // content blocks, which carry no roster.
    let Some(output) = payload.get("output").and_then(Value::as_str) else {
        return Vec::new();
    };
    let Ok(roster) = serde_json::from_str::<Value>(output) else {
        return Vec::new();
    };
    agent_roster(&roster)
}

/// Every agent in the session, and whether each is still going.
///
/// `{"agents":[{"agent_name":"/root","agent_status":"running"},
/// {"agent_name":"/root/marked_picker","agent_status":{"completed":"..."}}]}`
///
/// **This is the only record codex writes that ends a subagent**, which is why
/// a tool result is read at all. It is authoritative when it arrives -- it
/// names every agent and states each one's status -- but it arrives only when
/// the model chooses to call `list_agents`, which it did in 6 of the 15
/// subagent-bearing rollouts here. A fleet is therefore capable of being shown
/// as running for longer than it truly ran, until the turn ends and the fold
/// clears it. That is the honest limit of what codex records, and inventing a
/// close for the other 9 would be worse: a row that empties on a signal the
/// log does not carry is wrong at a moment nobody can check.
///
/// `completed` is not death, either. Observed in one session: `marked_picker`
/// is `completed` in one roster and `running` in the next, because codex's
/// agents are re-taskable through `followup_task` -- completed means "idle,
/// having answered", not "gone". Reading it as `running: false` is still
/// right for a row that asks how many agents are WORKING, and the next roster
/// or the next `interacted` puts it back.
///
/// A status this does not recognize yields no event for that agent at all --
/// not `running`, which would pin it to the row forever, and not finished,
/// which would invent an ending. Only the two observed shapes are read: the
/// string `"running"`, and an object, which is a RESULT the agent handed back
/// (`completed` is the only key ever seen in one).
fn agent_roster(roster: &Value) -> Vec<TurnEvent> {
    let Some(agents) = roster.get("agents").and_then(Value::as_array) else {
        return Vec::new();
    };
    agents
        .iter()
        .filter_map(|agent| {
            let path = agent.get("agent_name")?.as_str()?;
            let name = sub_agent_name(path)?;
            let status = agent.get("agent_status")?;
            let running = if status.as_str() == Some("running") {
                true
            } else if status.is_object() {
                false
            } else {
                return None;
            };
            Some(TurnEvent::Subagent {
                id: path.to_string(),
                description: name.to_string(),
                running,
            })
        })
        .collect()
}

/// The agent's own name, out of the path codex identifies it by.
///
/// `/root/stage_probe` is the `task_name` its `spawn_agent` call chose, under
/// the root. The whole path is the join key -- it is the one identifier
/// present in BOTH shapes, since a roster entry carries no `agent_thread_id`
/// and an activity record carries no `agent_name` -- but the last segment is
/// what a row has room for.
///
/// **`/root` alone is this session's OWN agent and is never a subagent of it.**
/// Every roster names it (`{"agent_name":"/root","agent_status":"running"}`),
/// so reading it would make every codex pane that ever called `list_agents`
/// report one agent it does not have. It also turns up as an `agent_path` in
/// the SUBAGENT's own rollout, where it means the parent -- the same wrong
/// answer from the other direction. Requiring something under `/root/` rules
/// out both, and a codex that renames its root simply yields nothing here
/// rather than a wrong count.
fn sub_agent_name(path: &str) -> Option<&str> {
    let below = path.strip_prefix("/root/")?;
    let name = below.rsplit('/').next().unwrap_or(below).trim();
    (!name.is_empty()).then_some(name)
}

/// An event from `event_msg`/`item_completed`, the shape codex 0.147.0 writes.
///
/// This is not a refinement of `agent_message` -- it replaces it. Codex stopped
/// emitting `event_msg`/`agent_message` entirely: of 264 rollouts on the
/// machine this was found on, 251 carry the old shape, 10 carry this one, and
/// the two sets do not overlap at all. The 10 are today's. So a reader that
/// knows only `agent_message` shows an empty feed on the version codex
/// actually ships, while the turn clock keeps working -- which is why this
/// went unnoticed: nothing looks broken, there is just never anything to say.
///
/// Both shapes are read, because both versions are still on disk and a user
/// can downgrade. They cannot double-count: no rollout has ever carried both.
///
/// `item.type` names what happened. Three of them are worth a line in a feed
/// meant to be read at a glance; `UserMessage` is the prompt that already
/// started the turn, and `Reasoning` was empty in every observed record.
fn item_completed(payload: &Value) -> Option<TurnEvent> {
    let item = payload.get("item")?;
    match item.get("type").and_then(Value::as_str)? {
        "AgentMessage" => {
            // `commentary` is the running narration and `final_answer` the
            // conclusion. Both are what the agent said, and dropping the
            // narration is what left a codex row's transcript empty for the
            // whole of every turn: counted across the 324 rollouts on this
            // machine, 161 of the 193 `AgentMessage` items written in this
            // shape are commentary -- five in six -- and every one of them
            // arrives while somebody could still be watching. The conclusion
            // is still told apart, and read differently -- see
            // `TurnEvent::Said`.
            //
            // The agent's private thinking is NOT this: codex writes that as
            // its own `agent_reasoning` payload (896 records), which this
            // parser never reaches.
            let conclusion = item.get("phase").and_then(Value::as_str) == Some("final_answer");
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            let text = text.trim().to_string();
            (!text.is_empty()).then_some(TurnEvent::Said { text, conclusion })
        }
        "FileChange" => {
            // `changes` maps an absolute path to what happened to it. The
            // path is the machine's, not the reader's, so only the file name
            // survives -- the same narrowing claude's `Write` step makes.
            let changes = item.get("changes")?.as_object()?;
            let object = match changes.len() {
                0 => return None,
                1 => file_name(changes.keys().next()?),
                n => format!("{n} files"),
            };
            Some(TurnEvent::Did { verb: "write".to_string(), object })
        }
        "CommandExecution" => {
            // `command` is argv, and codex always wraps in a login shell, so
            // the first two entries are `/bin/zsh -lc` on every record. The
            // last is the command a person would recognize.
            let command = item.get("command")?.as_array()?;
            let object = command.last()?.as_str()?.to_string();
            Some(TurnEvent::Did { verb: "run".to_string(), object })
        }
        // The same fields as the `event_msg`/`sub_agent_activity` payload,
        // moved inside an item the way `agent_message` was -- `kind`,
        // `agent_path`, `agent_thread_id`. One reader serves both, because
        // both are still on disk for the same reason both message shapes are.
        "SubAgentActivity" => sub_agent_activity(item),
        _ => None,
    }
}

/// The last path segment, or the whole string when there is no separator.
fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE_TURN: &str = include_str!("../../fixtures/session-logs/codex-complete-turn.jsonl");
    const ITEM_COMPLETED_TURN: &str = include_str!("../../fixtures/session-logs/codex-item-completed-turn.jsonl");
    const UNMATCHED_TASK_STARTED: &str = include_str!("../../fixtures/session-logs/codex-unmatched-task-started.jsonl");
    const SUBAGENTS: &str = include_str!("../../fixtures/session-logs/codex-subagents.jsonl");

    fn line(fixture: &str, n: usize) -> &str {
        fixture.lines().nth(n).expect("fixture has that many lines")
    }

    #[test]
    fn task_started_converts_seconds_to_millis() {
        // Line 2: `event_msg`/`task_started`, `started_at: 1781462823`. The
        // trap this test exists to catch: reading `started_at` straight
        // through (no `* 1000`) would silently pass any assertion that only
        // checks "some number is present" -- so this asserts the exact
        // converted value against the real fixture number.
        match parse_line(line(COMPLETE_TURN, 1)).as_slice() {
            [TurnEvent::Started { at_ms }] => assert_eq!(*at_ms, Some(1_781_462_823_000)),
            other => panic!("expected [Started], got {other:?}"),
        }
    }

    #[test]
    fn task_complete_takes_duration_ms_unmultiplied() {
        // Line 11: `event_msg`/`task_complete`, `completed_at: 1781462826`,
        // `duration_ms: 3153`. `duration_ms` is already milliseconds in this
        // SAME payload as the seconds-valued `completed_at` -- the mixed-unit
        // trap the reference doc names. Multiplying `duration_ms` here would
        // report a "say hi" turn as taking 52+ minutes.
        match parse_line(line(COMPLETE_TURN, 10)).as_slice() {
            [TurnEvent::Ended { at_ms, duration_ms, outcome }] => {
                assert_eq!(*at_ms, Some(1_781_462_826_000));
                assert_eq!(*duration_ms, Some(3153));
                assert_eq!(*outcome, TurnOutcome::Finished);
            }
            other => panic!("expected [Ended], got {other:?}"),
        }
    }

    #[test]
    fn turn_aborted_ends_a_turn_as_aborted_not_finished() {
        // Synthetic, not drawn from a fixture: neither real fixture happens
        // to carry `turn_aborted` (5 of 183 real files end on an unmatched
        // `task_started` instead, which is `codex-unmatched-task-started.jsonl`
        // below). A turn ending this way is still a real terminal state, and
        // a reader that only knew `task_complete` would leave it open forever.
        let synthetic = r#"{"timestamp":"2026-06-14T18:47:06.348Z","type":"event_msg","payload":{"type":"turn_aborted","turn_id":"00000000-0000-4000-8000-000000000021","reason":"interrupted"}}"#;
        match parse_line(synthetic).as_slice() {
            [TurnEvent::Ended { outcome, .. }] => assert_eq!(*outcome, TurnOutcome::Aborted),
            other => panic!("expected [Ended], got {other:?}"),
        }
    }

    #[test]
    fn a_final_answer_agent_message_is_what_the_agent_said() {
        // Line 8: `event_msg`/`agent_message`, `phase: "final_answer"`,
        // `message: "Hi."`.
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 7)),
            vec![TurnEvent::Said { text: "Hi.".to_string(), conclusion: true }]
        );
    }

    // -----------------------------------------------------------------
    // item_completed -- the shape codex 0.147.0 actually writes
    // -----------------------------------------------------------------

    /// The whole reason this shape is read: driving a real codex 0.147.0 in a
    /// pane produced a working status and turn clock, and a permanently empty
    /// feed, because none of the records below are `agent_message`.
    #[test]
    fn a_file_change_item_is_a_write_step_named_by_its_file() {
        // Line 6: `item.type: "FileChange"`, one path under `changes`.
        assert_eq!(
            parse_line(line(ITEM_COMPLETED_TURN, 5)),
            vec![TurnEvent::Did { verb: "write".to_string(), object: "fruit.txt".to_string() }]
        );
    }

    #[test]
    fn a_command_execution_item_is_a_run_step_without_its_shell_wrapper() {
        // Line 7: `command: ["/bin/zsh", "-lc", "rg ... fruit.txt"]`. The
        // wrapper is on every record and carries no information.
        match parse_line(line(ITEM_COMPLETED_TURN, 6)).as_slice() {
            [TurnEvent::Did { verb, object }] => {
                assert_eq!(verb, "run");
                assert!(!object.contains("/bin/zsh"), "the shell wrapper leaked into {object:?}");
                assert!(object.contains("fruit.txt"), "expected the real command, got {object:?}");
            }
            other => panic!("expected one run step, got {other:?}"),
        }
    }

    #[test]
    fn a_final_answer_item_is_what_the_agent_said() {
        // Line 8: `item.type: "AgentMessage"`, `phase: "final_answer"`.
        assert_eq!(
            parse_line(line(ITEM_COMPLETED_TURN, 7)),
            vec![TurnEvent::Said {
                text: "Created `fruit.txt` containing `banana`.".to_string(),
                conclusion: true
            }]
        );
    }

    /// The complaint "when using codex, there's no transcript" reduces to this
    /// line, and to nothing else.
    ///
    /// Line 5 is an `AgentMessage` whose phase is `commentary` -- the agent
    /// narrating, four seconds into a turn, while a person is watching the
    /// row. Rejecting it meant a codex pane's transcript gained its first
    /// entry when the turn ENDED, so a short turn said nothing at all from
    /// start to finish.
    #[test]
    fn a_commentary_item_is_what_the_agent_said_and_is_not_the_answer() {
        assert_eq!(
            parse_line(line(ITEM_COMPLETED_TURN, 4)),
            vec![TurnEvent::Said {
                text: "I\u{2019}ll create `fruit.txt` in the workspace with the requested word, then verify it.".to_string(),
                conclusion: false
            }]
        );
    }

    #[test]
    fn the_prompt_and_the_reasoning_items_are_not_steps() {
        // Line 3 is `UserMessage` -- the prompt, which already started the
        // turn. Line 4 is `Reasoning`, empty in every observed record.
        assert_eq!(parse_line(line(ITEM_COMPLETED_TURN, 2)), Vec::new());
        assert_eq!(parse_line(line(ITEM_COMPLETED_TURN, 3)), Vec::new());
    }

    /// The turn boundaries must survive unchanged in the new format, since
    /// they are what kept working while the feed was silently empty.
    #[test]
    fn the_new_format_still_starts_and_ends_its_turn() {
        match parse_line(line(ITEM_COMPLETED_TURN, 1)).as_slice() {
            [TurnEvent::Started { at_ms: Some(ms) }] => assert!(*ms > 1_700_000_000_000, "seconds leaked through as {ms}"),
            other => panic!("expected [Started], got {other:?}"),
        }
        match parse_line(line(ITEM_COMPLETED_TURN, 8)).as_slice() {
            [TurnEvent::Ended { outcome, .. }] => assert_eq!(*outcome, TurnOutcome::Finished),
            other => panic!("expected [Ended], got {other:?}"),
        }
    }

    /// The two shapes have never been seen in one file, so a feed can never
    /// show the same conclusion twice -- but reading both must not change what
    /// the old fixture produces.
    #[test]
    fn reading_the_new_shape_did_not_disturb_the_old_one() {
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 7)),
            vec![TurnEvent::Said { text: "Hi.".to_string(), conclusion: true }]
        );
    }

    /// The older shape narrates too, and 498 of the 1399 `agent_message`
    /// records on this machine are that narration. It reaches the transcript
    /// on the same terms the new shape's does.
    #[test]
    fn a_commentary_agent_message_is_narration_not_the_answer() {
        let synthetic = r#"{"timestamp":"2026-06-14T18:47:06.340Z","type":"event_msg","payload":{"type":"agent_message","message":"Checking the tests first.","phase":"commentary"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Said { text: "Checking the tests first.".to_string(), conclusion: false }]
        );
    }

    /// The agent's private thinking is not prose it sent, and it never was:
    /// codex writes it as its own `agent_reasoning` payload -- 896 records
    /// against 1399 `agent_message`s -- which this parser does not read at all.
    #[test]
    fn agent_reasoning_is_not_a_step() {
        let synthetic = r#"{"timestamp":"2026-06-14T18:47:06.340Z","type":"event_msg","payload":{"type":"agent_reasoning","text":"thinking out loud"}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    #[test]
    fn a_token_count_record_yields_nothing() {
        // Line 10: `event_msg`/`token_count`. Real and observed, but read by
        // Task 10, not this one -- inventing an event here would be a field
        // with no consumer.
        assert_eq!(parse_line(line(COMPLETE_TURN, 9)), Vec::new());
    }

    // -----------------------------------------------------------------
    // subagents -- the agents codex runs and shows nobody
    // -----------------------------------------------------------------

    /// Line 3: `event_msg`/`sub_agent_activity`, `kind: "started"`,
    /// `agent_path: "/root/stage_probe"`.
    ///
    /// The id is the PATH and not the `agent_thread_id` sitting right beside
    /// it in the same record, which is the trap this asserts against. A roster
    /// entry carries no thread id at all, so a parser that keyed on the
    /// obvious field would file the spawn and its own completion under two
    /// different agents -- and the count would only ever grow.
    #[test]
    fn a_started_activity_is_a_running_subagent_keyed_by_its_path() {
        assert_eq!(
            parse_line(line(SUBAGENTS, 2)),
            vec![TurnEvent::Subagent {
                id: "/root/stage_probe".to_string(),
                description: "stage_probe".to_string(),
                running: true,
            }]
        );
    }

    /// Line 7: the same fields as an `item_completed` item of type
    /// `SubAgentActivity`, which is the shape 0.147.0 writes. Both are on disk
    /// and a user can downgrade, exactly as with `agent_message`.
    #[test]
    fn the_item_shape_of_a_spawn_is_read_the_same_way() {
        assert_eq!(
            parse_line(line(SUBAGENTS, 6)),
            vec![TurnEvent::Subagent {
                id: "/root/coverage_audit".to_string(),
                description: "coverage_audit".to_string(),
                running: true,
            }]
        );
    }

    /// Line 5: the `list_agents` roster, and the only record codex writes that
    /// ENDS a subagent.
    ///
    /// Three things at once, and each is a way to get this wrong. `/root` is
    /// the session's own agent and must not appear at all -- every roster
    /// names it, so reading it makes every codex pane report one agent it does
    /// not have. `marked_picker` carries an OBJECT status (`{"completed":
    /// ...}`) and is the one entry that is not running. The other two are the
    /// plain string `"running"` and stay.
    #[test]
    fn a_roster_ends_the_completed_agent_and_leaves_the_running_ones() {
        let events = parse_line(line(SUBAGENTS, 4));
        assert_eq!(
            events,
            vec![
                TurnEvent::Subagent {
                    id: "/root/marked_picker".to_string(),
                    description: "marked_picker".to_string(),
                    running: false,
                },
                TurnEvent::Subagent {
                    id: "/root/stage_probe".to_string(),
                    description: "stage_probe".to_string(),
                    running: true,
                },
                TurnEvent::Subagent {
                    id: "/root/stage_types".to_string(),
                    description: "stage_types".to_string(),
                    running: true,
                },
            ]
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                TurnEvent::Subagent { id, .. } if id == "/root"
            )),
            "the session's own agent reached the list: {events:?}"
        );
    }

    /// The roster's id is the same string the spawn used, or the fold that
    /// joins them (`Signals::spawn`, keyed on `id`) would file one agent under
    /// two entries and never take it off the row.
    #[test]
    fn a_spawn_and_its_roster_entry_name_one_agent() {
        let spawned = parse_line(line(SUBAGENTS, 2));
        let listed = parse_line(line(SUBAGENTS, 4));
        let id_of = |events: &[TurnEvent], want: &str| {
            events.iter().any(|event| matches!(
                event,
                TurnEvent::Subagent { id, running, .. } if id == want && *running
            ))
        };
        assert!(id_of(&spawned, "/root/stage_probe"), "the spawn: {spawned:?}");
        assert!(id_of(&listed, "/root/stage_probe"), "the roster: {listed:?}");
    }

    /// Line 6: `kind: "interacted"`, and the inference this parser makes on
    /// purpose. Every subagent-bearing rollout on this machine is larger than
    /// `tail::READ_FROM_START_BYTES`, so the pane is attached to at its END
    /// and the `started` lines are already behind it -- an `interacted` is
    /// then the only thing left in the file saying a fleet exists.
    #[test]
    fn an_interaction_keeps_a_subagent_on_the_row() {
        assert_eq!(
            parse_line(line(SUBAGENTS, 5)),
            vec![TurnEvent::Subagent {
                id: "/root/stage_probe".to_string(),
                description: "stage_probe".to_string(),
                running: true,
            }]
        );
    }

    /// `/root` is the session's own agent seen from inside a SUBAGENT's
    /// rollout, where `agent_path` names the parent. Reading it would put a
    /// pane's own agent on its own row as a child of itself.
    #[test]
    fn the_root_agent_is_never_one_of_its_own_subagents() {
        let synthetic = r#"{"timestamp":"2026-08-18T17:51:16.669Z","type":"event_msg","payload":{"type":"sub_agent_activity","event_id":"call_00000000000000000000000009","occurred_at_ms":1787075476669,"agent_thread_id":"00000000-0000-4000-8000-000000000010","agent_path":"/root","kind":"interacted"}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    /// The `kind` vocabulary is `started` and `interacted` and nothing else --
    /// 137 records, no third value. A kind nobody has seen must not be guessed
    /// at in either direction: called running it pins an agent to the row, and
    /// called finished it invents an ending the log never stated.
    #[test]
    fn an_unknown_activity_kind_is_not_guessed_at() {
        let synthetic = r#"{"timestamp":"2026-08-18T17:51:16.669Z","type":"event_msg","payload":{"type":"sub_agent_activity","event_id":"call_00000000000000000000000009","occurred_at_ms":1787075476669,"agent_thread_id":"00000000-0000-4000-8000-000000000010","agent_path":"/root/stage_probe","kind":"hibernated"}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }

    /// The same rule for a roster status. Only the string `"running"` and an
    /// object result have ever been seen; a third shape yields nothing for
    /// that agent rather than moving it either way.
    #[test]
    fn an_unknown_roster_status_yields_nothing_for_that_agent() {
        let synthetic = r#"{"timestamp":"2026-07-27T00:25:45.732Z","type":"response_item","payload":{"type":"function_call_output","id":"fco_0000","call_id":"call_0000","output":"{\"agents\":[{\"agent_name\":\"/root/stage_probe\",\"agent_status\":\"queued\"},{\"agent_name\":\"/root/stage_types\",\"agent_status\":\"running\"}]}"}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Subagent {
                id: "/root/stage_types".to_string(),
                description: "stage_types".to_string(),
                running: true,
            }]
        );
    }

    /// A `function_call_output` does not name the tool it came back from, so
    /// the roster is recognized by its shape. Every OTHER tool result -- and
    /// `exec` is 3,924 of them against 13 `list_agents` -- must fall through
    /// untouched, including one whose output is JSON that is simply not a
    /// roster.
    #[test]
    fn an_ordinary_tool_result_is_not_a_roster() {
        let exec = r#"{"timestamp":"2026-07-27T00:25:45.732Z","type":"response_item","payload":{"type":"function_call_output","id":"fco_0000","call_id":"call_0000","output":"{\"output\":\"banana\",\"metadata\":{\"exit_code\":0}}"}}"#;
        assert_eq!(parse_line(exec), Vec::new());
        let prose = r#"{"timestamp":"2026-07-27T00:25:45.732Z","type":"response_item","payload":{"type":"function_call_output","id":"fco_0000","call_id":"call_0000","output":"total 8\ndrwxr-xr-x  3 user staff 96 Jul 27 00:25 ."}}"#;
        assert_eq!(parse_line(prose), Vec::new());
    }

    /// Reading a second top-level `type` must not have disturbed the first
    /// one. `parse_line` grew a `response_item` arm where it used to reject
    /// everything but `event_msg`, and `response_item` is the single most
    /// common record in a rollout -- 20,853 of them here.
    #[test]
    fn the_other_response_items_still_yield_nothing() {
        // A `function_call` is not its output, and a `message` carries prose
        // that `agent_message` and `item_completed` already report.
        let call = r#"{"timestamp":"2026-07-27T00:23:38.889Z","type":"response_item","payload":{"type":"function_call","id":"fc_0000","name":"spawn_agent","namespace":"collaboration","arguments":"{\"task_name\":\"stage_probe\"}","call_id":"call_0000"}}"#;
        assert_eq!(parse_line(call), Vec::new());
        assert_eq!(parse_line(line(COMPLETE_TURN, 2)), Vec::new());
        assert_eq!(parse_line(line(COMPLETE_TURN, 8)), Vec::new());
    }

    /// The turn boundaries the fixture opens and closes with still work, since
    /// they are what clears the fleet off a row: the daemon's fold drops every
    /// spawn at either end of a turn, which is the whole reason an inferred
    /// `interacted` cannot outlive the turn that wrote it.
    #[test]
    fn the_subagent_fixture_still_starts_and_ends_its_turn() {
        match parse_line(line(SUBAGENTS, 1)).as_slice() {
            [TurnEvent::Started { at_ms }] => assert_eq!(*at_ms, Some(1_785_111_779_000)),
            other => panic!("expected [Started], got {other:?}"),
        }
        match parse_line(line(SUBAGENTS, 7)).as_slice() {
            [TurnEvent::Ended { outcome, .. }] => assert_eq!(*outcome, TurnOutcome::Finished),
            other => panic!("expected [Ended], got {other:?}"),
        }
    }

    #[test]
    fn a_malformed_line_yields_an_empty_vec_and_does_not_panic() {
        assert_eq!(parse_line("this is not json at all {"), Vec::new());
        assert_eq!(parse_line(""), Vec::new());
    }

    #[test]
    fn session_meta_turn_context_and_response_item_yield_nothing() {
        // These are top-level `type`s with no `payload.type` this parser
        // reads (lines 1, 3, 4, 5, 6, 9 of the complete-turn fixture).
        assert_eq!(parse_line(line(COMPLETE_TURN, 0)), Vec::new()); // session_meta
        assert_eq!(parse_line(line(COMPLETE_TURN, 2)), Vec::new()); // response_item (developer)
        assert_eq!(parse_line(line(COMPLETE_TURN, 4)), Vec::new()); // turn_context
        assert_eq!(parse_line(line(COMPLETE_TURN, 8)), Vec::new()); // response_item (assistant)
    }

    #[test]
    fn a_user_message_event_yields_nothing() {
        // Line 7: `event_msg`/`user_message` -- a real, documented payload,
        // but not one this parser's vocabulary maps to (the turn already
        // started at `task_started`; this is not a second start).
        assert_eq!(parse_line(line(COMPLETE_TURN, 6)), Vec::new());
    }

    #[test]
    fn an_unmatched_task_started_still_starts_a_turn() {
        // The whole point of this fixture: `task_started` with no
        // `task_complete` or `turn_aborted` anywhere in the file. This parser
        // reads one line at a time and has no way to know that -- and must
        // not pretend to. It reports `Started` exactly as it would for any
        // other `task_started`; a reader across the whole file is what has to
        // notice the turn never closes.
        match parse_line(line(UNMATCHED_TASK_STARTED, 1)).as_slice() {
            [TurnEvent::Started { at_ms }] => assert_eq!(*at_ms, Some(1_786_344_608_000)),
            other => panic!("expected [Started], got {other:?}"),
        }
        assert_eq!(parse_line(line(UNMATCHED_TASK_STARTED, 0)), Vec::new()); // session_meta
    }
}

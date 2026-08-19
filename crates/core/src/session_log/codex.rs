//! Codex's rollout: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
//!
//! See `docs/agent-session-logs.md` for what every field named here was
//! observed to mean -- this file reads only the fields that document names,
//! and nothing else, so a field codex adds tomorrow breaks nothing today.
//!
//! Codex is the richest of the three: it states the turn outright, in
//! `task_started` and `task_complete`, rather than requiring the inference
//! claude and cursor need.

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
/// that is not JSON, an outer `type` other than `event_msg`, or a
/// `payload.type` the reference doc does not name. An unrecognized line is
/// meant to fall through to the screen-scraping layer below, not to error --
/// these are private formats with no compatibility promise.
pub fn parse_line(line: &str) -> Vec<TurnEvent> {
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    // Every turn-shaped fact codex writes is nested under `event_msg`'s own
    // `payload.type` (`docs/agent-session-logs.md`, "Turn starts" / "Turn
    // ends" / "Other payloads"). `session_meta`, `turn_context`, and
    // `response_item` are sibling top-level `type`s with no such further
    // discriminator this parser needs, so they fall through here.
    if record.get("type").and_then(Value::as_str) != Some("event_msg") {
        return Vec::new();
    }
    let Some(payload) = record.get("payload") else {
        return Vec::new();
    };
    match payload.get("type").and_then(Value::as_str) {
        Some("task_started") => task_started(payload).into_iter().collect(),
        Some("task_complete") => task_complete(payload).into_iter().collect(),
        Some("turn_aborted") => turn_aborted(&record).into_iter().collect(),
        Some("agent_message") => agent_message(payload).into_iter().collect(),
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

//! Cursor's transcript: `~/.cursor/projects/<slug>/agent-transcripts/<uuid>/<uuid>.jsonl`.
//!
//! See `docs/agent-session-logs.md` for what every field named here was
//! observed to mean -- this file reads only the fields that document names,
//! and nothing else, so a field cursor adds tomorrow breaks nothing today.
//!
//! **This parser is on much thinner evidence than its siblings, and that is
//! deliberate, not sloppy.** `claude.rs` and `codex.rs` were checked against
//! 276 and 183 real files respectively; this one against four, totaling
//! under 2 KB combined, and most cursor project directories on the machine
//! this was built against have no transcripts at all. Only one tool name
//! (`Shell`) was ever observed. No tool-result record exists in any sample.
//! Every sample shut down cleanly, so nothing here says what an interrupted
//! transcript looks like. Where the siblings can afford to be strict because
//! the sample was large enough to be confident about what is absent, this one
//! cannot: it degrades rather than assumes, everywhere a shape it has not
//! seen might show up.

use serde_json::Value;

use super::{TurnEvent, TurnOutcome};

/// Parse one line of a cursor transcript into the shared turn vocabulary.
///
/// Returns a `Vec` rather than an `Option` for the same reason as the other
/// two parsers: the shared vocabulary is a `Vec` because some record can
/// carry two facts. Here that happens routinely rather than as an edge case
/// -- an assistant record's `content` array can hold prose AND a tool call in
/// the same line, and both are worth a row.
///
/// Returns an empty `Vec` for anything this parser does not recognize: a line
/// that is not JSON, a `role` other than `user`/`assistant`, or a `type`
/// other than `turn_ended`. An unrecognized line is meant to fall through to
/// the screen-scraping layer below, not to error -- these are private
/// formats with no compatibility promise, and this one in particular has
/// barely been observed at all.
pub fn parse_line(line: &str) -> Vec<TurnEvent> {
    let Ok(record) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    // Records are either `{role, message}` or `{type, status}`
    // (`docs/agent-session-logs.md`, "Records") -- never both, so checking
    // `type` first and falling back to `role` does not risk misreading one
    // shape as the other.
    if record.get("type").and_then(Value::as_str) == Some("turn_ended") {
        return turn_end(&record).into_iter().collect();
    }
    match record.get("role").and_then(Value::as_str) {
        Some("user") => turn_start(&record).into_iter().collect(),
        Some("assistant") => steps(&record),
        _ => Vec::new(),
    }
}

/// A turn start is `role: "user"` (`docs/agent-session-logs.md`, "Turn
/// starts"). Unlike claude, no tool-result-shaped `user` record has ever
/// been observed for cursor, so `role` alone is enough -- there is no second
/// shape to tell apart from a real turn start, on the evidence that exists.
fn turn_start(record: &Value) -> Option<TurnEvent> {
    if record.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    // The request text is NOT read.
    //
    // It used to be: this function unwrapped cursor's
    // `<timestamp>...</timestamp><user_query>...</user_query>` envelope and
    // threw the result away, kept only so a test could prove the unwrap
    // worked. That was written when the feed did not exist yet and the guess
    // was that something would want the request soon. The feed exists now, and
    // it carries what the AGENT did -- `TurnEvent::Step`s -- not what the
    // human asked, so there is still no consumer, and a computation with no
    // consumer is one that can be silently wrong forever.
    //
    // Whoever needs it next: the envelope is documented in
    // `docs/agent-session-logs.md` ("Turn starts"), the timestamp inside it is
    // a localized human string (`Sunday, Aug 16, 2026, 1:19 AM (UTC-7)`) that
    // needs its own parser, and `fixtures/session-logs/cursor-complete-turn.jsonl`
    // has a real one to write it against. Reconstructing three lines of string
    // slicing from that is cheaper than carrying dead code that claims to be
    // tested.
    Some(TurnEvent::Started { at_ms: None })
}

/// An assistant record's `content` array, one event per block worth naming.
/// Prose (`type: "text"`) becomes `Said`, matching what claude's `text` blocks
/// and codex's `agent_message` do for the same thing. A `tool_use`
/// block becomes a named action (see `tool_step`). The two can appear in the
/// SAME record -- the fixture's own line 2 is prose ("Running that command
/// now.") immediately followed by a `Shell` call -- which is why this
/// collects rather than returning the first match: dropping either one would
/// under-report what the agent actually did on that line.
fn steps(record: &Value) -> Vec<TurnEvent> {
    let Some(content) = record.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else {
        return Vec::new();
    };
    content.iter().filter_map(block_step).collect()
}

fn block_step(block: &Value) -> Option<TurnEvent> {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => {
            let text = block.get("text").and_then(Value::as_str)?;
            // Nothing marks a conclusion. Cursor writes no `stop_reason` and
            // no `phase` -- its only turn-terminal record is `turn_ended`, a
            // LINE later, and `parse_line` reads one line at a time by
            // contract. So everything cursor says is read as narration, which
            // costs a finished pane nothing: the last thing it said is still
            // the last thing in the window.
            Some(TurnEvent::Said { text: text.to_string(), conclusion: false })
        }
        Some("tool_use") => tool_step(block),
        // `thinking` or anything else undocumented falls through -- only
        // `text` and `tool_use` were ever observed in a cursor `content`
        // array.
        _ => None,
    }
}

/// A `tool_use` block's action. `name` becomes the verb, lowercased, matching
/// claude and codex. The object prefers `input.description` over
/// `input.command` -- cursor writes a human sentence there ("Write banana to
/// fruit.txt") and it reads better in a row than the shell line does
/// (`docs/agent-session-logs.md`, "Tool calls"). Only `Shell` was ever
/// observed, so a different tool name still yields a `Did`, degrading to
/// `command` and then to the tool's own name if neither field is a string --
/// treating an unseen tool name as an error would be assuming the four-file
/// sample was complete when it plainly is not.
fn tool_step(block: &Value) -> Option<TurnEvent> {
    let name = block.get("name")?.as_str()?;
    let verb = name.to_lowercase();
    let input = block.get("input");
    let object = input
        .and_then(|i| i.get("description"))
        .and_then(Value::as_str)
        .or_else(|| input.and_then(|i| i.get("command")).and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string());
    Some(TurnEvent::Did { verb, object })
}

/// A turn end is `{"type": "turn_ended", "status": ...}`
/// (`docs/agent-session-logs.md`, "Turn ends"). Only `success` and `error`
/// were ever observed, mapping to `Finished` and `Failed`. Anything else --
/// never seen, but on four samples that proves nothing -- still ends the
/// turn rather than being ignored: a turn that never ends is the exact
/// failure this whole project exists to remove, so an unrecognized status is
/// read as `Aborted` (matching codex's own outcome for a turn that ended
/// without a confirmed completion) rather than dropped on the floor.
fn turn_end(record: &Value) -> Option<TurnEvent> {
    let outcome = match record.get("status").and_then(Value::as_str) {
        Some("success") => TurnOutcome::Finished,
        Some("error") => TurnOutcome::Failed,
        _ => TurnOutcome::Aborted,
    };
    // No timestamp or duration field was ever observed on this record --
    // unlike claude's `turn_duration` or codex's `task_complete` -- so both
    // are left `None` rather than guessed at.
    Some(TurnEvent::Ended { at_ms: None, duration_ms: None, outcome })
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE_TURN: &str = include_str!("../../fixtures/session-logs/cursor-complete-turn.jsonl");
    const TURN_ENDED_ERROR: &str = include_str!("../../fixtures/session-logs/cursor-turn-ended-error.jsonl");

    fn line(fixture: &str, n: usize) -> &str {
        fixture.lines().nth(n).expect("fixture has that many lines")
    }

    #[test]
    fn a_user_record_starts_a_turn() {
        // Line 1: `role: "user"`, text wrapped in the timestamp/user_query
        // envelope. `at_ms` is `None` -- the wrapped timestamp is a
        // localized human string, not something this parser converts.
        assert_eq!(parse_line(line(COMPLETE_TURN, 0)), vec![TurnEvent::Started { at_ms: None }]);
    }

    /// A turn start says a turn started, and nothing else.
    ///
    /// The envelope around the request text is deliberately not read -- see
    /// `turn_start`. This pins that the record still parses when the text is
    /// wrapped, which is the only way it ever arrives.
    #[test]
    fn the_wrapped_request_text_does_not_stop_the_turn_from_starting() {
        let record: Value = serde_json::from_str(line(COMPLETE_TURN, 0)).unwrap();
        let text = record["message"]["content"][0]["text"].as_str().expect("line 1 has a text block");
        assert!(text.starts_with("<timestamp>"), "fixture text should be wrapped: {text}");
        assert_eq!(parse_line(line(COMPLETE_TURN, 0)), vec![TurnEvent::Started { at_ms: None }]);
    }

    #[test]
    fn turn_ended_success_finishes_the_turn() {
        // Line 4: `{"type":"turn_ended","status":"success"}`.
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 3)),
            vec![TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Finished }]
        );
    }

    #[test]
    fn turn_ended_error_fails_the_turn() {
        // The entire content of one of the four real transcript files: a
        // free-plan model-availability error, `status: "error"`.
        assert_eq!(
            parse_line(line(TURN_ENDED_ERROR, 0)),
            vec![TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Failed }]
        );
    }

    #[test]
    fn an_unrecognized_turn_ended_status_still_ends_the_turn() {
        // Synthetic: only `success` and `error` were ever seen, but an
        // unrecognized status must still close the turn out as `Aborted`
        // rather than being silently dropped -- an open turn is the failure
        // this whole project exists to remove.
        let synthetic = r#"{"type":"turn_ended","status":"cancelled"}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Ended { at_ms: None, duration_ms: None, outcome: TurnOutcome::Aborted }]
        );
    }

    #[test]
    fn a_shell_tool_use_prefers_description_over_command() {
        // Line 2: prose ("Running that command now.") followed by a `Shell`
        // tool_use whose `input` carries both `command` and `description`.
        // Both blocks are steps -- this is the routine two-facts-in-one-line
        // case the module doc calls out.
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 1)),
            vec![
                TurnEvent::Said { text: "Running that command now.".to_string(), conclusion: false },
                TurnEvent::Did { verb: "shell".to_string(), object: "Write banana to fruit.txt".to_string() },
            ]
        );
    }

    #[test]
    fn assistant_prose_alone_is_what_the_agent_said() {
        // Line 3: a closing assistant record with only a text block, no
        // tool_use.
        assert_eq!(
            parse_line(line(COMPLETE_TURN, 2)),
            vec![TurnEvent::Said { text: "Done. `fruit.txt` now contains `banana`.".to_string(), conclusion: false }]
        );
    }

    #[test]
    fn a_tool_name_other_than_shell_still_yields_a_step() {
        // Only `Shell` was ever observed. Treating anything else as an error
        // would be assuming the four-file sample was complete.
        let synthetic = r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Browser","input":{"command":"open https://example.com"}}]}}"#;
        assert_eq!(
            parse_line(synthetic),
            vec![TurnEvent::Did { verb: "browser".to_string(), object: "open https://example.com".to_string() }]
        );
    }

    #[test]
    fn a_tool_use_with_neither_field_falls_back_to_its_own_name() {
        // Degrades one step further than the command fallback: no
        // `description`, no `command`, so the object becomes the tool's own
        // name rather than an empty string.
        let synthetic = r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Mystery"}]}}"#;
        assert_eq!(parse_line(synthetic), vec![TurnEvent::Did { verb: "mystery".to_string(), object: "Mystery".to_string() }]);
    }

    #[test]
    fn a_malformed_line_yields_an_empty_vec_and_does_not_panic() {
        assert_eq!(parse_line("this is not json at all {"), Vec::new());
        assert_eq!(parse_line(""), Vec::new());
    }

    #[test]
    fn an_unrecognized_role_yields_an_empty_vec() {
        let synthetic = r#"{"role":"system","message":{"content":[]}}"#;
        assert_eq!(parse_line(synthetic), Vec::new());
    }
}

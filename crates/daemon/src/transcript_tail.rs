//! Codex and cursor's prose, read out of the transcript they name rather
//! than out of a hook payload that never carries it.
//!
//! `assemble.rs`'s `Stop` arm already turns codex's `last_assistant_message`
//! into one `Message` per turn, and cursor's `stop` carries no prose at all
//! (`docs/agent-session-logs.md`, "Records"). Between those two facts, a
//! cursor chat renders prompts and turn boundaries with nothing in between,
//! and a codex chat renders only the closing line of a turn that may have
//! narrated for minutes on the way there. This reads the transcript path
//! every hook payload already carries (`Facts::transcript_path`,
//! `crates/agent-hooks/src/facts.rs`) and turns its own lines into the same
//! prose, as they are written.
//!
//! Deliberately not `farcooler_core::session_log`: that module's
//! `parse_line` functions decode the full `TurnEvent` vocabulary a feed
//! derivation needs -- task lists, tool calls, subagent activity -- and this
//! needs exactly one fact out of a line, whether an assistant said
//! something, which does not need that module's weight pulled into a daemon
//! module that is on a hook's own critical path.
//!
//! Uses `notify`, the same crate `log_watch.rs` watches its three fixed log
//! roots with, rather than a second file-watching mechanism -- but not that
//! module's own `LogWatcher` type, whose `drain`-on-a-timer shape is a
//! different fit than the push-per-line callback this needs. What is reused
//! is the crate and the pattern (a `recommended_watcher` closure feeding a
//! channel), not the struct.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use notify::{RecursiveMode, Watcher as _};
use serde_json::Value;

/// The largest line this will decode. `farcooler_core::session_log::tail`
/// draws the same line at the same value, for the same reason: the biggest
/// line ever observed in a real session log is 1.35 MB
/// (`docs/agent-session-logs.md`, claude's "Hazards"), so this is nowhere
/// near it and a line over the cap is one this tail was never going to be
/// able to afford anyway. A separate constant rather than that module's,
/// because `Tail` computes its own starting offset from the file's size and
/// this type's whole reason to exist is that its caller supplies one
/// instead -- see `follow`.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// The longest a real append can go undelivered when the filesystem watch
/// stays silent -- either FSEvents' own documented gap right after a stream
/// starts, or a genuinely missed event some other way. Short enough that a
/// live viewer never reads as broken, long enough that this is a safety net
/// under the watch and not a second polling mechanism doing the watch's job
/// for it -- one no-op read a second, for as long as an agent's terminal is
/// tailed, is not a cost worth avoiding.
const WAIT_POLL_FALLBACK: std::time::Duration = std::time::Duration::from_secs(1);

/// Tails one agent's transcript and hands its assistant prose to a sink, one
/// line's worth at a time, once each, however many times the file changes
/// underneath.
///
/// Holds nothing of its own. `follow` keeps everything it needs -- the read
/// offset, the watcher -- alive on the thread it spawns for as long as that
/// thread keeps running, which today is the life of the daemon process:
/// nothing stops it, because nothing this type is given ever asks it to. A
/// caller that wants delivery to stop (`hook_ingress::HookIngress::forget`,
/// when a terminal's row is deleted) has to gate its own sink instead; see
/// that call site's `alive` flag.
pub struct TranscriptTail;

impl TranscriptTail {
    pub fn new() -> Self {
        TranscriptTail
    }

    /// Deliver every complete line's assistant text from `path`, starting at
    /// byte `from`, once per line, for as long as the file keeps growing.
    ///
    /// The catch-up read and the OS-level watch registration happen on the
    /// CALLING thread, before this returns -- both are quick (no wait for a
    /// file event, only for the watch to confirm itself is scheduled), which
    /// keeps this well inside the 400ms `hook_ingress::HookIngress::serve`
    /// has for a whole connection. Only the open-ended part -- blocking until
    /// the file changes again, for as long as this daemon runs -- moves to a
    /// spawned thread, and deliberately not sooner: registering the watch
    /// from a thread OTHER than the one that keeps running past this call
    /// (verified empirically against this exact `notify` backend) delivers no
    /// events at all, silently, forever. `log_watch.rs`'s `LogWatcher::start`
    /// registers on its caller's thread for the same reason, whether or not
    /// its own doc says so.
    ///
    /// `from` is the caller's choice and not derived here on purpose. The
    /// caller knows why it is starting a tail at this moment -- a session
    /// that has just announced itself, mid-conversation -- and `hook_ingress`
    /// starts every tail at the file's length at that moment, so a session
    /// with turns already behind it does not replay them into the live
    /// transcript as if they had just happened.
    pub fn follow<S>(&self, path: PathBuf, from: u64, on_text: S)
    where
        S: Fn(String) + Send + Sync + 'static,
    {
        let mut offset = from;
        // What is already on disk when this starts is read once before any
        // watch exists, so a burst that landed between the hook that
        // triggered this and this call actually running is not missed
        // waiting for a filesystem event that already happened.
        read_new_lines(&path, &mut offset, &on_text);

        let Some(parent) = path.parent().map(|p| p.to_path_buf()) else {
            tracing::warn!(path = %path.display(), "a transcript path with no parent directory; nothing to watch");
            return;
        };

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        });
        let mut watcher = match watcher {
            Ok(w) => w,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    path = %path.display(),
                    "could not start a transcript tail; this session's prose stops here"
                );
                return;
            }
        };

        // The DIRECTORY, not the file. Codex does not open its rollout until
        // the first turn is submitted (`docs/agent-session-logs.md`, "Which
        // file belongs to which pane"), so a tail can start before the file
        // exists -- and even once it exists, a watch on the file alone would
        // miss a truncate-and-replace, which some editors and log rotators do
        // instead of an in-place append. The date directory it lives under is
        // created once per day across every codex session on the machine, so
        // by the time any hook fires it is essentially always there; if it
        // genuinely is not (the very first codex session of the day, hooked
        // before its first turn), this tail never starts and says so at
        // `debug!` rather than `warn!` -- `log_watch.rs`'s call for the same
        // shape of absence: a directory nobody has written into yet is the
        // ordinary case, not a fault.
        if let Err(error) = watcher.watch(&parent, RecursiveMode::Recursive) {
            tracing::debug!(?error, path = %parent.display(), "transcript directory not there yet; not tailed");
            return;
        }

        std::thread::spawn(move || {
            // Keeping `watcher` alive is this closure's whole job for as
            // long as it runs: a `notify::Watcher` stops watching the moment
            // it is dropped (`log_watch.rs`'s own `LogWatcher` carries the
            // same note about its field), and this is the last place that
            // still holds it once `follow` has returned.
            let _watcher = watcher;

            // `recv_timeout` rather than `recv`: an actual event triggers a
            // read immediately, and `WAIT_POLL_FALLBACK` triggers one anyway
            // if none arrives. FSEvents (this daemon's target platform) is
            // documented by Apple as coalescing and, immediately after a
            // stream starts, as able to miss an event that lands in the same
            // instant the watch was registered -- a real gap, not a
            // hypothetical one, and one a hook firing right after a session
            // announces itself can land in. The fallback bounds how long that
            // gap can cost: at worst, an agent's prose is this constant late,
            // never lost. Only a disconnected channel -- this end of `tx`
            // dropped, `notify`'s watcher gone -- ends the loop for good.
            loop {
                match rx.recv_timeout(WAIT_POLL_FALLBACK) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // Every event on the directory triggers a read, not
                        // only ones that name this exact path. The three
                        // platform backends do not agree on which paths one
                        // event carries for a rename or a coalesced burst
                        // (`log_watch.rs` makes the same call, for the same
                        // reason), and the cost of over-triggering -- like the
                        // cost of the periodic fallback above -- is one cheap
                        // no-op read against an unchanged offset.
                        read_new_lines(&path, &mut offset, &on_text);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    }
}

impl Default for TranscriptTail {
    fn default() -> Self {
        Self::new()
    }
}

/// Read every complete line appended to `path` since `*offset`, decode
/// whatever assistant text each holds, and hand it to `on_text` -- advancing
/// `*offset` past exactly the bytes read, so a line is never delivered
/// twice.
///
/// A missing file, or one that has not grown past `*offset`, is not an
/// error and produces nothing: both are the ordinary state before an
/// agent's first turn opens its transcript at all, and the next filesystem
/// event tries again.
fn read_new_lines(path: &Path, offset: &mut u64, on_text: &impl Fn(String)) {
    let Ok(mut file) = std::fs::File::open(path) else { return };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return };

    // Smaller than what was already read: the file was truncated or
    // replaced underneath this tail. `farcooler_core::session_log::tail`
    // resets to zero for the same case and the same reason -- the stored
    // offset now points past the end of a file that no longer has that much
    // in it, and seeking there would read nothing forever rather than
    // picking the replacement up from its own start.
    if len < *offset {
        *offset = 0;
    }
    if len == *offset {
        return;
    }
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return;
    }

    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return;
    }

    // Only bytes up to and including the LAST newline are consumed. A
    // trailing fragment with no newline yet is a record the writer is
    // still mid-`write()` on -- parsing it now would either fail or, worse,
    // succeed on a coincidentally-valid truncated prefix. It is left
    // exactly where it is; the next event re-reads it whole once its own
    // newline lands, because `*offset` was never advanced past it.
    let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else { return };
    let complete = &buf[..=last_newline];
    *offset += complete.len() as u64;

    for line in complete.split(|&b| b == b'\n') {
        // `split` on a slice ending in the separator yields one trailing
        // empty slice, which is not a line to decode.
        if line.is_empty() || line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Ok(line) = std::str::from_utf8(line) else { continue };
        if let Some(text) = assistant_text(line) {
            on_text(text);
        }
    }
}

/// The assistant's own words in one line of a transcript, or `None` when the
/// line is not assistant prose -- a user turn, a tool call, a status record,
/// or a shape this function does not recognize. Never panics on malformed or
/// unexpected JSON: these are private, undocumented formats
/// (`docs/agent-session-logs.md` says so for both), and a line neither agent
/// has been observed writing yet is meant to be skipped, not to bring down a
/// hook connection nothing on the agent's side can ever be told failed.
fn assistant_text(line: &str) -> Option<String> {
    let record: Value = serde_json::from_str(line).ok()?;

    // Codex's shape: a top-level `event_msg` wrapping a `payload` whose own
    // `type` says what happened (`docs/agent-session-logs.md`, "codex").
    if record.get("type").and_then(Value::as_str) == Some("event_msg") {
        return codex_assistant_text(record.get("payload")?);
    }

    // Cursor's shape: `{role, message}`, never wrapped in an outer `type`
    // (`docs/agent-session-logs.md`, "cursor" -- "Records"). `role ==
    // "assistant"` is the only one this function has anything to read out
    // of; `"user"` is a turn start and carries no assistant prose.
    if record.get("role").and_then(Value::as_str) == Some("assistant") {
        if let Some(text) = cursor_assistant_text(&record) {
            return Some(text);
        }
        // A bare `{role: "assistant", text: "..."}`, which is not a shape
        // either agent's four-sample or 183-file corpus ever produced
        // (`docs/agent-session-logs.md` names cursor's only two record
        // shapes as `{role, message}` and `{type, status}`). Kept as a
        // fallback rather than refused outright, on this module's own
        // "decode leniently" charter: cursor's own format is documented from
        // four files total and is free to add a flatter shape tomorrow, and
        // a reader that only ever tries today's nesting finds nothing the
        // day it does.
        return said(record.get("text")?.as_str()?);
    }

    None
}

/// Codex's `event_msg` payload, in either of the two shapes
/// `docs/agent-session-logs.md`'s "What happened during a turn" table
/// names. Codex has been observed writing only one or the other per file,
/// never both, so trying both here costs nothing on a real rollout and
/// nothing here needs to know which version wrote it.
///
/// The turn's CLOSING line is deliberately dropped in both shapes:
/// `phase == "final_answer"` is the exact text codex's `Stop` hook already
/// hands over as `last_assistant_message`, which
/// `farcooler_agent_hooks::assemble::MessageAssembler`'s `Stop` arm already
/// turns into a `Message` -- see that module's
/// `codex_says_what_it_answered_when_the_turn_ends` test. Forwarding it
/// again here would draw every codex answer twice on the same transcript,
/// which is the exact failure `hook_ingress::HookIngress` keeps one tail per
/// terminal to avoid on a smaller scale. What this DOES forward is codex's
/// running commentary (`phase == "commentary"`), which `Stop` never carries
/// at all and which is otherwise invisible for the whole turn.
fn codex_assistant_text(payload: &Value) -> Option<String> {
    match payload.get("type").and_then(Value::as_str) {
        Some("agent_message") => {
            if payload.get("phase").and_then(Value::as_str) == Some("final_answer") {
                return None;
            }
            said(payload.get("message")?.as_str()?)
        }
        Some("item_completed") => {
            let item = payload.get("item")?;
            if item.get("type").and_then(Value::as_str) != Some("AgentMessage") {
                return None;
            }
            if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
                return None;
            }
            let text: String = item
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            said(&text)
        }
        _ => None,
    }
}

/// Cursor's `role: "assistant"` record. Prose lives in `message.content[]`
/// blocks of `type == "text"` alongside possible `tool_use` blocks in the
/// same array (`docs/agent-session-logs.md`, "cursor" -- "Tool calls"; the
/// only sample seen carrying both is prose immediately followed by one
/// `Shell` call). The FIRST text block is taken rather than every one
/// joined: no sample has ever carried two, and joining them with no
/// separator would silently run two unrelated messages together the day one
/// does.
fn cursor_assistant_text(record: &Value) -> Option<String> {
    let content = record.get("message")?.get("content")?.as_array()?;
    let text = content
        .iter()
        .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)?;
    said(text)
}

/// Trims whitespace and turns an empty result into "nothing to say" rather
/// than an empty `Message`.
fn said(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn text_appended_after_we_started_watching_is_delivered_once() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(&path, "{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"first\"}\n")
            .expect("seed");

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = { let seen = seen.clone(); move |t: String| seen.lock().unwrap().push(t) };
        let tail = TranscriptTail::new();
        tail.follow(path.clone(), 0, sink);

        for _ in 0..100 {
            if seen.lock().unwrap().len() == 1 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(seen.lock().unwrap().as_slice(), ["first"]);

        // Appending must not re-deliver what was already read.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).expect("open");
        writeln!(f, "{{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"second\"}}")
            .expect("append");

        for _ in 0..100 {
            if seen.lock().unwrap().len() == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["first", "second"],
            "a tail delivers each line once, however often the file changes"
        );
    }

    /// `follow`'s `from` is honored, not recomputed: history before it must
    /// never surface, which is the whole reason `hook_ingress` is trusted to
    /// pass the file's length at the moment a session announced itself
    /// rather than 0.
    #[tokio::test]
    async fn text_written_before_from_is_never_delivered() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"old\"}\n\
             {\"type\":\"message\",\"role\":\"assistant\",\"text\":\"new\"}\n",
        )
        .expect("seed");
        let from = std::fs::metadata(&path).unwrap().len()
            - "{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"new\"}\n".len() as u64;

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = { let seen = seen.clone(); move |t: String| seen.lock().unwrap().push(t) };
        TranscriptTail::new().follow(path.clone(), from, sink);

        for _ in 0..100 {
            if !seen.lock().unwrap().is_empty() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(seen.lock().unwrap().as_slice(), ["new"], "\"old\" sat before `from` and must stay unseen");
    }

    // -- assistant_text: the decode `follow` runs every line through --

    #[test]
    fn codex_commentary_is_forwarded() {
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": "Checking the tests first." },
        })
        .to_string();
        assert_eq!(assistant_text(&line), Some("Checking the tests first.".to_string()));
    }

    /// The nearest wrong implementation here forwards codex's every
    /// `agent_message`, `final_answer` included -- which is exactly the text
    /// `assemble.rs`'s `Stop` arm already emits from
    /// `last_assistant_message`, so that implementation draws every codex
    /// answer twice.
    #[test]
    fn codex_final_answer_is_dropped_because_stop_already_sends_it() {
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "phase": "final_answer",
                "message": "TCP slow start is a congestion-control mechanism.",
            },
        })
        .to_string();
        assert_eq!(
            assistant_text(&line),
            None,
            "the closing line must come from Stop.last_assistant_message alone, not from here too"
        );
    }

    #[test]
    fn codex_item_completed_commentary_is_forwarded_and_its_final_answer_dropped() {
        let commentary = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "item": {
                    "type": "AgentMessage",
                    "phase": "commentary",
                    "content": [{ "text": "I'll inspect the current phone state." }],
                },
            },
        })
        .to_string();
        assert_eq!(assistant_text(&commentary), Some("I'll inspect the current phone state.".to_string()));

        let closing = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "item": {
                    "type": "AgentMessage",
                    "phase": "final_answer",
                    "content": [{ "text": "Done." }],
                },
            },
        })
        .to_string();
        assert_eq!(assistant_text(&closing), None, "item_completed's closing answer is dropped for the same reason as agent_message's");
    }

    /// A `FileChange` or `CommandExecution` item, or any other `item.type`
    /// this parser does not read, must not be mistaken for prose.
    #[test]
    fn codex_item_completed_of_a_non_message_item_yields_nothing() {
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "item": { "type": "CommandExecution", "command": ["ls"] },
            },
        })
        .to_string();
        assert_eq!(assistant_text(&line), None);
    }

    #[test]
    fn cursor_text_block_is_forwarded_alongside_a_tool_call_in_the_same_line() {
        let line = serde_json::json!({
            "role": "assistant",
            "message": {
                "content": [
                    { "type": "text", "text": "Running that command now." },
                    { "type": "tool_use", "name": "Shell", "input": { "command": "ls" } },
                ],
            },
        })
        .to_string();
        assert_eq!(assistant_text(&line), Some("Running that command now.".to_string()));
    }

    /// The fallback shape the mandated test above exercises end to end --
    /// unit-tested directly here so a change to `assistant_text` that broke
    /// it would fail fast, without waiting on a `tokio::time::sleep` loop to
    /// say so.
    #[test]
    fn a_bare_role_and_text_record_is_read_by_the_fallback() {
        let line = serde_json::json!({ "type": "message", "role": "assistant", "text": "first" }).to_string();
        assert_eq!(assistant_text(&line), Some("first".to_string()));
    }

    #[test]
    fn cursor_user_turn_and_turn_ended_carry_no_assistant_text() {
        let user = serde_json::json!({ "role": "user", "message": { "content": [] } }).to_string();
        assert_eq!(assistant_text(&user), None);

        let ended = serde_json::json!({ "type": "turn_ended", "status": "success" }).to_string();
        assert_eq!(assistant_text(&ended), None);
    }

    #[test]
    fn a_line_that_is_not_json_is_skipped_rather_than_panicking() {
        assert_eq!(assistant_text("not json at all"), None);
        assert_eq!(assistant_text(""), None);
    }

    #[test]
    fn whitespace_only_prose_is_nothing_to_say() {
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": { "type": "agent_message", "phase": "commentary", "message": "   " },
        })
        .to_string();
        assert_eq!(assistant_text(&line), None);
    }

    // -- read_new_lines: the half-written-tail hazard --

    /// The nearest wrong implementation reads up to EOF unconditionally,
    /// which would hand a half-written JSON record to `assistant_text`
    /// before its closing brace has even landed -- either a decode failure
    /// that silently eats a real message, or worse, a truncated string that
    /// still happens to parse.
    #[test]
    fn a_half_written_last_line_is_held_until_its_newline_arrives() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"whole\"}\n{\"type\":\"mess",
        )
        .expect("seed");

        use std::io::Write;

        let seen: Vec<String> = Vec::new();
        let seen = std::sync::Mutex::new(seen);
        let mut offset = 0u64;
        read_new_lines(&path, &mut offset, &|t| seen.lock().unwrap().push(t));
        assert_eq!(seen.lock().unwrap().as_slice(), ["whole"], "the half-written record must not be read yet");

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"age\",\"role\":\"assistant\",\"text\":\"second\"}\n")
            .unwrap();
        read_new_lines(&path, &mut offset, &|t| seen.lock().unwrap().push(t));
        assert_eq!(seen.lock().unwrap().as_slice(), ["whole", "second"]);
    }
}

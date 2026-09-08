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
//!
//! **A known, accepted ordering quirk, named here rather than fixed.**
//! Codex's own closing line reaches `agents.record` through the hook path
//! (`assemble.rs`'s `Stop` arm), synchronously, the moment the `Stop`
//! payload is parsed. Its narration for the SAME turn reaches `agents.record`
//! through this module instead, on a background thread that wakes on a real
//! filesystem event OR `WAIT_POLL_FALLBACK` (up to a second), whichever comes
//! first. A turn short enough to finish inside that gap can therefore have
//! its closing answer NUMBERED AND RENDERED before the narration that, in
//! the agent's own transcript, preceded it. Not a numbering violation --
//! `AgentSupervisor::record` still numbers everything by the order it
//! actually arrives, which is the only order it can promise -- but it is
//! user-visible, and worth a client someday reading the delivered order
//! rather than assuming it matches the turn's real one.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use notify::{RecursiveMode, Watcher as _};
use serde_json::Value;

/// The largest line this will decode, at the same value
/// `farcooler_core::session_log::tail::Tail` uses -- but not "for the same
/// reason": that module reads in bounded chunks and clears an over-cap line
/// as it goes, so its cap is what keeps a still-growing or oversized line
/// from ever being held in memory whole. This one is applied AFTER
/// `read_new_lines` has already read the candidate bytes into `buf` (bounded
/// by `MAX_READ_BYTES`, not by this), so what this cap actually does is
/// refuse to spend a `serde_json` parse on a line that has no business being
/// this large -- a line's WORST case is skipped rather than decoded, not
/// kept out of memory. Set to the same number anyway because the reasoning
/// for the number itself still holds: the biggest line ever observed in a
/// real session log is 1.35 MB (`docs/agent-session-logs.md`, claude's
/// "Hazards"), so this is nowhere near it and a line over the cap was never
/// going to be assistant prose worth affording the parse for.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// The most this will read from the file in one call to `read_new_lines`,
/// regardless of how much is actually new. Ordinary growth between polls is
/// nowhere near this -- an agent's commentary is a few hundred bytes to a
/// few KB per line -- but the truncate-and-replace case this module's own
/// doc names resets `*offset` to 0 and would otherwise read the WHOLE
/// replacement file into `buf` in one call, unbounded. Capping the read
/// bounds that to one memory-sized chunk per call: whatever does not fit is
/// simply picked up on the next call, exactly like an ordinary append that
/// outpaces one poll interval already is.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

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
/// offset, the watcher, if any -- alive on the thread it spawns for as long
/// as `alive` (the flag passed into `follow`, not this struct) says to keep
/// running. That is the ONLY way delivery stops: `hook_ingress::HookIngress
/// ::forget` flips the same `Arc` this was handed, and the loop checks it
/// itself rather than relying on a channel disconnecting, which a fix to
/// this file's first review round found could never actually happen while
/// the loop ran (see `follow`'s own doc).
pub struct TranscriptTail;

impl TranscriptTail {
    pub fn new() -> Self {
        TranscriptTail
    }

    /// Deliver every complete line's assistant text from `path`, starting at
    /// byte `from`, once per line, for as long as `alive` holds `true` and
    /// the file keeps growing. Returns whether a tailing thread was actually
    /// started; a caller that gets `false` back has nothing running and
    /// should treat this as a request it may retry, not as a stopped tail.
    ///
    /// **The watch is an optimization over `WAIT_POLL_FALLBACK` below, not
    /// the mechanism.** A first version of this function treated a failed
    /// `watch()` as fatal -- log and return, with nothing spawned -- which
    /// bricked the feature FOREVER for any terminal whose transcript
    /// directory did not exist yet at the moment its first hook payload
    /// arrived: guaranteed for the first codex session of every calendar day
    /// (`~/.codex/sessions/YYYY/MM/DD/` is created with that day's first
    /// turn), and routine for cursor, whose per-conversation directory is
    /// created lazily and which has no OTHER source of prose to fall back to
    /// at all. The loop below is spawned regardless of whether a watch could
    /// be registered, and `WAIT_POLL_FALLBACK` alone is what bounds delivery
    /// either way; a watch that registers and fires only ever moves a
    /// delivery earlier inside that bound, never makes one happen that would
    /// not have. Nothing here asserts the watch does fire: this file's own
    /// history records a `notify` registration in this sandbox going
    /// silently deaf depending on which thread registered it, and
    /// `start_transcript_tail` now registers from a blocking-pool thread
    /// that does not outlive the call.
    ///
    /// **Calling this may block the calling thread for a while doing OS-level
    /// work** -- `File::open`, `read_to_end`,
    /// `notify::recommended_watcher`, `Watcher::watch` all run here, inline,
    /// with no `.await` between them. Measured in the sandbox this was built
    /// against, watch registration alone has taken as long as ~11 seconds
    /// under load. `hook_ingress::HookIngress::start_transcript_tail` is
    /// where that is accounted for -- it runs this inside `tokio::spawn` and
    /// `spawn_blocking` rather than on `serve`'s own task, which is this
    /// function's caller's problem to solve, not this function's to pretend
    /// does not exist.
    ///
    /// `from` is the caller's choice and not derived here on purpose. The
    /// caller knows why it is starting a tail at this moment -- a session
    /// that has just announced itself, mid-conversation -- and `hook_ingress`
    /// starts every tail at the file's length at that moment, so a session
    /// with turns already behind it does not replay them into the live
    /// transcript as if they had just happened.
    pub fn follow<S>(&self, path: PathBuf, from: u64, on_text: S, alive: Arc<AtomicBool>) -> bool
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
            return false;
        };

        let (tx, rx) = std::sync::mpsc::channel::<()>();
        // A clone taken BEFORE `tx` is moved into the watcher's callback,
        // and held for the life of the spawned loop below regardless of
        // whether that watcher ever exists. Without it, a failed
        // `recommended_watcher` or a failed `watch()` -- the exact cases
        // this function now degrades gracefully from rather than refusing
        // outright -- drops the only `Sender` there is, `rx` reports
        // `Disconnected` on the very next call, and the "poll-only" fallback
        // this whole change exists to provide would itself never run a
        // second time.
        let keep_alive_tx = tx.clone();
        let watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        }) {
            Ok(mut w) => {
                // The DIRECTORY, not the file. Codex does not open its
                // rollout until the first turn is submitted
                // (`docs/agent-session-logs.md`, "Which file belongs to
                // which pane"), so a tail can start before the file exists
                // -- and even once it exists, a watch on the file alone
                // would miss a truncate-and-replace, which some editors and
                // log rotators do instead of an in-place append.
                match w.watch(&parent, RecursiveMode::Recursive) {
                    Ok(()) => Some(w),
                    Err(error) => {
                        // NOT fatal -- see this function's own doc. A
                        // directory nobody has written into yet, or one that
                        // does not exist for either agent's reason above, is
                        // the ordinary case for a first payload, not a
                        // fault, so this stays below `warn!`.
                        tracing::debug!(
                            ?error,
                            path = %parent.display(),
                            "could not watch this transcript's directory; falling back to polling alone"
                        );
                        None
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    path = %path.display(),
                    "could not build a filesystem watcher; falling back to polling alone"
                );
                None
            }
        };

        std::thread::spawn(move || {
            // Keeping `watcher` alive is this closure's whole job for as
            // long as it runs, when there is one to keep: a
            // `notify::Watcher` stops watching the moment it is dropped
            // (`log_watch.rs`'s own `LogWatcher` carries the same note about
            // its field), and this is the last place that still holds it
            // once `follow` has returned. `None` here is a no-op to hold and
            // means every wakeup below comes from `WAIT_POLL_FALLBACK`
            // rather than a real event.
            let _watcher = watcher;
            // See `keep_alive_tx`'s own doc above: held so `rx` never
            // disconnects on its own, which is what makes `alive` -- checked
            // below -- the only thing that can end this loop.
            let _keep_alive_tx = keep_alive_tx;

            loop {
                if !alive.load(Ordering::Relaxed) {
                    return;
                }
                // The `Result` is not matched on. `Ok` means a real event
                // fired; `Err(Timeout)` means `WAIT_POLL_FALLBACK` elapsed
                // with nothing; `Err(Disconnected)` cannot happen while
                // `_keep_alive_tx` above is held, but treating it as a
                // reason to stop here would resurrect exactly the dead exit
                // condition this loop used to have before `alive` existed --
                // every one of the three is answered the same way: read
                // whatever is new, then check `alive` again at the top.
                // Every event on the directory triggers a read, not only
                // ones that name this exact path -- the three platform
                // backends do not agree on which paths one event carries for
                // a rename or a coalesced burst (`log_watch.rs` makes the
                // same call, for the same reason), and the cost of
                // over-triggering, like the cost of the periodic fallback
                // itself, is one cheap no-op read against an unchanged
                // offset.
                let _ = rx.recv_timeout(WAIT_POLL_FALLBACK);
                read_new_lines(&path, &mut offset, &on_text);
            }
        });
        true
    }
}

impl Default for TranscriptTail {
    fn default() -> Self {
        Self::new()
    }
}

/// Read every complete line appended to `path` since `*offset`, decode
/// whatever assistant text each holds, and hand it to `on_text` -- advancing
/// `*offset` past exactly the bytes read, so an ordinary append is never
/// delivered twice.
///
/// **That guarantee has one named exception: a truncate-and-replace.** If
/// the file has shrunk since `*offset` was last set, `*offset` resets to 0
/// below and the WHOLE of whatever replaced it is read as new -- including
/// any line the replacement happens to share with what this tail already
/// delivered from the file's previous life. None of the three formats
/// `docs/agent-session-logs.md` documents has ever been observed to rotate
/// or truncate, so this is a real behavior with no known real trigger, kept
/// deliberately simple rather than tracking a file identity (an inode, a
/// leading record's own id) to detect and suppress the replay -- the
/// complexity is not worth affording for a case nothing here has seen.
///
/// A missing file, or one that has not grown past `*offset`, is not an
/// error and produces nothing: both are the ordinary state before an
/// agent's first turn opens its transcript at all, and the next filesystem
/// event tries again.
fn read_new_lines(path: &Path, offset: &mut u64, on_text: &impl Fn(String)) {
    let Ok(mut file) = std::fs::File::open(path) else { return };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return };

    // Smaller than what was already read: the file was truncated or
    // replaced underneath this tail -- see this function's own doc on what
    // that costs. `farcooler_core::session_log::tail` resets to zero for the
    // same case and the same reason -- the stored offset now points past the
    // end of a file that no longer has that much in it, and seeking there
    // would read nothing forever rather than picking the replacement up from
    // its own start.
    if len < *offset {
        *offset = 0;
    }
    if len == *offset {
        return;
    }
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return;
    }

    // `take(MAX_READ_BYTES)` rather than a plain `read_to_end`: see
    // `MAX_READ_BYTES`'s own doc. What is not read here is simply left for
    // the next call -- `*offset` only ever advances past bytes this
    // function actually consumed below.
    let mut buf = Vec::new();
    if file.take(MAX_READ_BYTES).read_to_end(&mut buf).is_err() {
        return;
    }

    // Only bytes up to and including the LAST newline are consumed. A
    // trailing fragment with no newline yet is a record the writer is
    // still mid-`write()` on -- parsing it now would either fail or, worse,
    // succeed on a coincidentally-valid truncated prefix. It is left
    // exactly where it is; the next event re-reads it whole once its own
    // newline lands, because `*offset` was never advanced past it.
    let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else {
        // No newline in a read that filled `MAX_READ_BYTES`. Byte for byte
        // that is indistinguishable from the half-written tail below, but it
        // cannot be one: a record still being written that is already longer
        // than the cap is also already far past `MAX_LINE_BYTES` and could
        // never have been decoded. Leaving `*offset` where it is -- the right
        // answer for a genuine fragment -- would re-read the same capped
        // chunk on every wakeup, advance nothing, and block every ordinary
        // line sitting behind the oversized one for as long as this tail
        // runs. So the bytes are stepped over. Whatever remains of that line
        // after the next call's own cap is eventually consumed as a leading
        // fragment, fails `from_utf8` or `serde_json`, and is skipped like
        // any other line this does not recognize.
        if buf.len() as u64 == MAX_READ_BYTES {
            tracing::debug!(
                path = %path.display(),
                "a transcript line longer than one capped read; stepping over it"
            );
            *offset += buf.len() as u64;
        }
        return;
    };
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
            // Joined with a space, matching
            // `farcooler_core::session_log::codex::item_completed`'s own read
            // of this same field: two readers of one record shape must not
            // hand back different text for it, and concatenating bare -- what
            // this did before -- runs the last word of one block into the
            // first word of the next.
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
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
        // The brief's own mandated block called `follow(path, from, sink)` --
        // three arguments, matching the task-8 brief verbatim. The fourth,
        // `alive`, was added in this file's first review round (`hook_ingress
        // ::HookIngress::forget` needs a way to stop the loop itself, not
        // only its sink -- see `follow`'s own doc), and the brief's
        // "parameter list" was never meant to survive a review that found a
        // real bug in the shape it pinned. `AtomicBool::new(true)` is this
        // test's own stand-in for what `hook_ingress` normally owns.
        assert!(tail.follow(path.clone(), 0, sink, Arc::new(AtomicBool::new(true))));

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
        assert!(TranscriptTail::new().follow(path.clone(), from, sink, Arc::new(AtomicBool::new(true))));

        for _ in 0..100 {
            if !seen.lock().unwrap().is_empty() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(seen.lock().unwrap().as_slice(), ["new"], "\"old\" sat before `from` and must stay unseen");
    }

    /// The exact shape of the bug this file's first review round found: a
    /// codex pane hooked before its rollout directory exists at all --
    /// guaranteed for the first codex session of every calendar day
    /// (`~/.codex/sessions/YYYY/MM/DD/` is created with that day's first
    /// turn) and routine for cursor, whose per-conversation directory is
    /// created lazily. The nearest wrong implementation treats a failed
    /// `watch()` as fatal: logs and returns with nothing spawned, which
    /// bricks this terminal's prose for the rest of the session with no
    /// symptom above `debug!`. This asserts BOTH halves of the fix -- that
    /// `follow` still reports it started, and that content written after the
    /// directory finally appears is still delivered, on the poll fallback
    /// alone since nothing ever re-registers a watch.
    #[tokio::test]
    async fn a_transcript_directory_missing_at_start_still_delivers_once_it_exists() {
        let dir = tempfile::tempdir().expect("dir");
        let parent = dir.path().join("2026").join("09").join("08");
        let path = parent.join("rollout.jsonl");

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = { let seen = seen.clone(); move |t: String| seen.lock().unwrap().push(t) };
        let started = TranscriptTail::new().follow(path.clone(), 0, sink, Arc::new(AtomicBool::new(true)));
        assert!(
            started,
            "a directory that does not exist yet must not be treated as a reason to spawn nothing"
        );

        // The directory -- and the file -- appear only now, the way codex's
        // own first turn of the day creates both at once.
        std::fs::create_dir_all(&parent).expect("mkdir");
        std::fs::write(&path, "{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"hello\"}\n").expect("seed");

        for _ in 0..100 {
            if !seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["hello"],
            "the poll fallback alone must still find this, since nothing ever re-registered a watch \
             once the directory existed"
        );
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

    /// `MAX_READ_BYTES` bounds one call's read, and a line longer than that
    /// bound arrives with no newline anywhere in what was read -- which is
    /// indistinguishable, byte for byte, from the half-written tail the test
    /// above holds onto. The nearest wrong implementation treats both the
    /// same way and holds the offset still: every later wakeup then re-reads
    /// the same capped chunk, finds no newline again, and advances nothing,
    /// so this tail spends the rest of the daemon's life reading
    /// `MAX_READ_BYTES` a second and never delivers another line -- including
    /// the perfectly ordinary records sitting behind the oversized one.
    #[test]
    fn a_line_longer_than_one_capped_read_does_not_stall_the_tail_behind_it() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("rollout.jsonl");

        // One line of `MAX_READ_BYTES + 1` bytes before its newline, so the
        // first read fills the cap with no newline in it at all, and a real
        // record behind it that must still arrive.
        let mut oversized = vec![b'x'; MAX_READ_BYTES as usize + 1];
        oversized.push(b'\n');
        oversized.extend_from_slice(b"{\"type\":\"message\",\"role\":\"assistant\",\"text\":\"behind it\"}\n");
        std::fs::write(&path, &oversized).expect("seed");

        let seen: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let mut offset = 0u64;
        // Three calls is generous: the oversized line needs two capped reads
        // to get past, and the third reaches the record behind it. A
        // stalling implementation delivers nothing however many are made.
        for _ in 0..3 {
            read_new_lines(&path, &mut offset, &|t| seen.lock().unwrap().push(t));
        }
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["behind it"],
            "a line too long to ever decode must be stepped over, not left blocking every line behind it"
        );
    }

    /// `farcooler_core::session_log::codex::item_completed` reads this exact
    /// field and joins its blocks with a space. The nearest wrong
    /// implementation `collect()`s them with no separator, which runs the
    /// last word of one block into the first word of the next -- two readers
    /// of one record handing back different text for it.
    #[test]
    fn codex_item_completed_joins_its_content_blocks_the_way_the_session_log_reader_does() {
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "item": {
                    "type": "AgentMessage",
                    "phase": "commentary",
                    "content": [
                        { "text": "Reading the config" },
                        { "text": "now." },
                    ],
                },
            },
        })
        .to_string();
        assert_eq!(
            assistant_text(&line),
            Some("Reading the config now.".to_string()),
            "`session_log::codex::item_completed` joins the same blocks with a space; a reader that \
             concatenates them bare says something else"
        );
    }
}

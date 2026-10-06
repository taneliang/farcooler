//! Pressing Enter in a working claude, and confirming its queue took the
//! message (ov-360). See `answer_wake`'s docs, "Mid-turn".
//!
//! **The Enter.** Mid-turn is when claude raises a dialog: a permission, a
//! question (`AskUserQuestion`), leaving plan mode. An Enter that lands on
//! one answers it, with its first option, which is Yes. So the Enter is
//! pressed only past two more checks, in this order, with nothing awaited
//! between the second and the write (`enter`):
//! 1. a fresh capture reads the box holding exactly the text, and the pane
//!    not Blocked;
//! 2. no gate (`PermissionRequest`, which claude raises for all three) has
//!    begun in the session since the paste began (`HookAsks::gated_since`),
//!    and no ask is held on the pane.
//!
//! The gate is heard before its dialog is drawn, so a dialog drawn after
//! check 1 was announced before check 2, unless it came in the time between
//! check 2 and the Enter reaching the pane, which is one `tmux send-keys`.
//! A session this daemon has never heard a hook from has no such signal,
//! and is never typed into mid-turn: its agent waits for the turn to end.
//! codex queues too, but raises its approvals with no hook at all, so a
//! working codex waits likewise (`queues_mid_turn`).
//!
//! **The queue.** Claude writes `{"type":"queue-operation","operation":
//! "enqueue","content":"<text>"}` to its transcript at once (measured on
//! 2.1.290), and that's what says it was queued: never only that the box
//! emptied, which it does for a command run instead too. The transcript is
//! found from the agent's process: its session registry,
//! `<config>/sessions/<pid>.json`, names the session and its cwd, and the
//! transcript is `<config>/projects/<cwd, munged>/<session>.jsonl`. The
//! config directory is the process's `CLAUDE_CONFIG_DIR`, else its `HOME`'s
//! `.claude`. Only records written past the transcript's length before the
//! paste count. A `user` record holding the text counts too, since the turn
//! may have ended between the checks and the Enter, and the text then went in
//! as a prompt; but not when the same text was already queued before the
//! paste, whose prompt that record could be.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_core::composer;
use farcooler_store::models::Terminal;

use super::{PASTE_POLL, Proven};
use crate::runtime::Runtime;
use crate::watch::Watcher;

/// How long after the Enter the queue has to show the message.
const QUEUE_SETTLES: Duration = Duration::from_secs(3);

/// The most of a transcript read on either side of the mark.
const LONGEST_READ: u64 = 1 << 20;

/// What will show a message was queued, set up before it's typed.
#[derive(Debug)]
pub(crate) struct Witness {
    /// The session, whose gates say a dialog is coming.
    session: String,
    /// Its transcript, and the transcript's length before the paste.
    path: PathBuf,
    from: u64,
    /// The same text queued already, before the paste.
    queued_before: bool,
    /// When the paste began: a gate from then on stops the Enter.
    pasted: Instant,
}

#[cfg(test)]
impl Witness {
    /// A witness for `session`'s transcript at `path`, the paste beginning now.
    pub(crate) fn for_tests(session: &str, path: &Path) -> Witness {
        let from = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        Witness { session: session.into(), path: path.into(), from, queued_before: false, pasted: Instant::now() }
    }
}

/// Why the Enter wasn't pressed, or didn't go.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NoEnter {
    /// A dialog is up, or announced: the text is left in the box.
    Dialog,
    /// The box no longer reads as holding the text.
    Moved,
    /// The Enter couldn't be sent.
    Failed,
}

impl Watcher {
    /// The witness for typing `text` into `proven` mid-turn, or `None` when
    /// nothing could make the Enter safe or show the message was queued.
    pub(super) async fn witness(&self, proven: &Proven, text: &str) -> Option<Witness> {
        if proven.preset != "claude" {
            return None;
        }
        let config = config_dir(&process_env(proven.pid).await?)?;
        let (session, path) = transcript_in(&config, proven.pid)?;
        if !self.service.hooks().asks().hooked(&session) {
            return None;
        }
        let from = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let queued_before = enqueued_before(&path, from, text);
        Some(Witness { session, path, from, queued_before, pasted: Instant::now() })
    }

    /// Press Enter in `to`, mid-turn, past the checks in this module's docs.
    pub(super) async fn enter(&self, to: &Terminal, preset: &str, witness: &Witness, text: &str) -> Result<(), NoEnter> {
        match self.box_of(to, preset).await {
            Ok(Ok((now, _))) if composer::holds_exactly(&now, text) => {}
            Ok(Err(super::Held::Prompt)) => return Err(NoEnter::Dialog),
            _ => return Err(NoEnter::Moved),
        }
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let asks = self.service.hooks().asks();
        // From here to the write, nothing is awaited.
        if asks.gated_since(&witness.session, witness.pasted) || asks.is_holding(to.id) {
            return Err(NoEnter::Dialog);
        }
        runtime.send_bytes_hex(to.id, "0d").await.map_err(|_| NoEnter::Failed)
    }

    /// Whether `text`, just submitted mid-turn, reached the agent's queue
    /// within `QUEUE_SETTLES`.
    pub(super) async fn queued(&self, witness: &Witness, text: &str) -> bool {
        let deadline = tokio::time::Instant::now() + QUEUE_SETTLES;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if recorded(&witness.path, witness.from, text, witness.queued_before) {
                return true;
            }
        }
        false
    }
}

/// The session and transcript `<config>/sessions/<pid>.json` names.
pub(crate) fn transcript_in(config: &Path, pid: i32) -> Option<(String, PathBuf)> {
    let registry = std::fs::read_to_string(config.join("sessions").join(format!("{pid}.json"))).ok()?;
    let registry: serde_json::Value = serde_json::from_str(&registry).ok()?;
    let session = registry.get("sessionId")?.as_str()?;
    let cwd = registry.get("cwd")?.as_str()?;
    // A session id is a uuid: never a path.
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let project = crate::session_discovery::project_dir_name(Path::new(cwd));
    Some((session.to_string(), config.join("projects").join(project).join(format!("{session}.jsonl"))))
}

/// claude's config directory, from its process's environment. The last of
/// a name wins: `ps -E` writes the environment after the arguments, which
/// can hold anything.
pub(crate) fn config_dir(env: &[(String, String)]) -> Option<PathBuf> {
    let var = |name: &str| env.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
    match (var("CLAUDE_CONFIG_DIR"), var("HOME")) {
        (Some(dir), _) => Some(PathBuf::from(dir)),
        (None, Some(home)) => Some(Path::new(home).join(".claude")),
        (None, None) => None,
    }
}

/// A process's environment: `/proc/<pid>/environ` on Linux, `ps -E` on
/// macOS, which writes it after the command, space-separated. A value with
/// a space in it is read only up to the space there, so a config directory
/// named that way isn't found, and the agent isn't typed into mid-turn.
async fn process_env(pid: i32) -> Option<Vec<(String, String)>> {
    if let Ok(raw) = std::fs::read(format!("/proc/{pid}/environ")) {
        return Some(pairs(raw.split(|b| *b == 0).map(|v| String::from_utf8_lossy(v).into_owned())));
    }
    let out = tokio::process::Command::new("ps")
        .args(["-E", "-ww", "-o", "command=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).into_owned();
    Some(pairs(line.split_whitespace().map(str::to_string)))
}

pub(crate) fn pairs(words: impl Iterator<Item = String>) -> Vec<(String, String)> {
    words
        .filter_map(|w| {
            let (k, v) = w.split_once('=')?;
            let name = !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            name.then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// The transcript's records in `[start, start + LONGEST_READ)`, as JSON.
fn records(path: &Path, start: u64) -> Vec<serde_json::Value> {
    records_between(path, start, start + LONGEST_READ)
}

/// What a record says was queued, or sent as a prompt.
fn said(record: &serde_json::Value) -> Option<(&'static str, &str)> {
    match record.get("type").and_then(|t| t.as_str()) {
        Some("queue-operation") if record.get("operation").and_then(|o| o.as_str()) == Some("enqueue") => {
            record.get("content").and_then(|c| c.as_str()).map(|c| ("enqueue", c))
        }
        Some("user") => record.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()).map(|c| ("user", c)),
        _ => None,
    }
}

/// Whether `text` was queued in the last stretch before byte `from`. A
/// record the stretch begins inside of fails to parse, and is skipped.
pub(crate) fn enqueued_before(path: &Path, from: u64, text: &str) -> bool {
    let want = squeeze(text);
    records_between(path, from.saturating_sub(LONGEST_READ), from)
        .iter()
        .filter_map(said)
        .any(|(kind, held)| kind == "enqueue" && squeeze(held) == want)
}

/// The transcript's records in `[start, end)`, as JSON. A record cut by
/// either end fails to parse, and is skipped.
fn records_between(path: &Path, start: u64, end: u64) -> Vec<serde_json::Value> {
    let Ok(mut file) = std::fs::File::open(path) else { return Vec::new() };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(end - start).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    String::from_utf8_lossy(&bytes).lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// Whether the transcript at `path`, past byte `from`, records `text`
/// queued, or sent as a prompt when it wasn't queued before (`queued_before`).
/// Whitespace is ignored, as the box's read-back ignores it.
pub(crate) fn recorded(path: &Path, from: u64, text: &str, queued_before: bool) -> bool {
    let want = squeeze(text);
    records(path, from)
        .iter()
        .filter_map(said)
        .any(|(kind, held)| squeeze(held) == want && (kind == "enqueue" || !queued_before))
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

//! Confirming a message typed into a working agent reached its queue
//! (ov-360). See `answer_wake`'s docs, "Mid-turn".
//!
//! Between turns the Enter is the end of it, as it always was: the agent
//! takes the box's text as its prompt. Mid-turn the agent keeps the text for
//! later, so what's confirmed is that it kept it, never only that the box
//! emptied: a box empties for a slash command run instead of queued too.
//!
//! - **claude** writes `{"type":"queue-operation","operation":"enqueue",
//!   "content":"<text>"}` to its transcript at once (measured on 2.1.290).
//!   The transcript is found from the agent's process: its session registry,
//!   `<config>/sessions/<pid>.json`, names the session and its cwd, and the
//!   transcript is `<config>/projects/<cwd, munged>/<session>.jsonl`. The
//!   config directory is the process's `CLAUDE_CONFIG_DIR`, else its
//!   `HOME`'s `.claude`. Only what's written past the transcript's length
//!   before the paste counts. A `user` record holding the text counts too:
//!   the turn may have ended between the check and the Enter, and then the
//!   text went in as a prompt.
//! - **codex** writes nothing anywhere at once, so its screen is the witness:
//!   the box no longer holds the text, and the text is drawn above it, under
//!   "Messages to be submitted after next tool call" (measured on 0.153.4), or
//!   as the prompt it became.
//!
//! A claude whose transcript can't be found is never typed into mid-turn:
//! there'd be no telling a queued message from a lost one, so it waits for
//! the turn to end, as every agent did before.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_core::composer;
use farcooler_store::models::Terminal;

use super::{PASTE_POLL, Proven};
use crate::watch::Watcher;

/// How long after the Enter the queue has to show the message.
const QUEUE_SETTLES: Duration = Duration::from_secs(3);

/// The most of a transcript read past the mark: one record, with room.
const LONGEST_READ: u64 = 1 << 20;

/// What will show a message was queued, set up before it's typed.
#[derive(Debug)]
pub(crate) enum Witness {
    /// claude's transcript, and its length before the paste.
    Transcript { path: PathBuf, from: u64 },
    /// codex's screen.
    Screen,
}

/// The witness for typing into `proven` mid-turn, or `None` when nothing
/// could show what's typed was queued.
pub(crate) async fn witness(proven: &Proven) -> Option<Witness> {
    match proven.preset {
        "claude" => {
            let path = transcript_of(proven.pid).await?;
            let from = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            Some(Witness::Transcript { path, from })
        }
        "codex" => Some(Witness::Screen),
        _ => None,
    }
}

impl Watcher {
    /// Whether `text`, just submitted to `to` mid-turn, reached its queue
    /// within `QUEUE_SETTLES`.
    pub(super) async fn queued(&self, to: &Terminal, preset: &str, witness: &Witness, text: &str) -> bool {
        let deadline = tokio::time::Instant::now() + QUEUE_SETTLES;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            let seen = match witness {
                Witness::Transcript { path, from } => recorded(path, *from, text),
                Witness::Screen => match self.service.screen(to.id).await {
                    Ok((screen, _, _)) => drawn_out_of_the_box(preset, &screen, text),
                    Err(_) => false,
                },
            };
            if seen {
                return true;
            }
        }
        false
    }
}

/// claude's transcript for the process `pid`, from its session registry.
async fn transcript_of(pid: i32) -> Option<PathBuf> {
    let config = config_dir(&process_env(pid).await?)?;
    transcript_in(&config, pid)
}

/// The transcript `<config>/sessions/<pid>.json` names.
pub(crate) fn transcript_in(config: &Path, pid: i32) -> Option<PathBuf> {
    let registry = std::fs::read_to_string(config.join("sessions").join(format!("{pid}.json"))).ok()?;
    let registry: serde_json::Value = serde_json::from_str(&registry).ok()?;
    let session = registry.get("sessionId")?.as_str()?;
    let cwd = registry.get("cwd")?.as_str()?;
    // A session id is a uuid: never a path.
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let project = crate::session_discovery::project_dir_name(Path::new(cwd));
    Some(config.join("projects").join(project).join(format!("{session}.jsonl")))
}

/// claude's config directory, from its process's environment.
pub(crate) fn config_dir(env: &[(String, String)]) -> Option<PathBuf> {
    let var = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
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

fn pairs(words: impl Iterator<Item = String>) -> Vec<(String, String)> {
    words
        .filter_map(|w| {
            let (k, v) = w.split_once('=')?;
            let name = !k.is_empty() && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            name.then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// Whether the transcript at `path`, past byte `from`, records `text`
/// queued, or sent as a prompt. Whitespace is ignored, as the box's
/// read-back ignores it.
pub(crate) fn recorded(path: &Path, from: u64, text: &str) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return false;
    }
    let mut tail = String::new();
    if file.take(LONGEST_READ).read_to_string(&mut tail).is_err() {
        return false;
    }
    let want = squeeze(text);
    tail.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok()).any(|record| {
        let kind = record.get("type").and_then(|t| t.as_str());
        let held = match kind {
            Some("queue-operation") if record.get("operation").and_then(|o| o.as_str()) == Some("enqueue") => {
                record.get("content").and_then(|c| c.as_str())
            }
            Some("user") => record.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_str()),
            _ => None,
        };
        held.is_some_and(|held| squeeze(held) == want)
    })
}

/// Whether `screen` shows `text` out of the agent's box: the box no longer
/// holds it, and it's drawn elsewhere on the screen.
pub(crate) fn drawn_out_of_the_box(preset: &str, screen: &str, text: &str) -> bool {
    if composer::holds_exactly(&composer::read(preset, screen), text) {
        return false;
    }
    composer::printed(screen).chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(&squeeze(text))
}

fn squeeze(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

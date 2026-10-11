//! Chat panes move to their TUI at their next idle (ov-376).
//!
//! Chat mode was retired (ov-377): an agent pane is its TUI, with the
//! conversation view drawn over it. A pane that was a chat when its runner
//! updated is still running `farcooler agent-host`, the shim, from the build
//! before, and its record still says `PaneMode::Agent`. This moves each one to
//! `claude --resume` (or `codex resume`) on the same conversation, once its
//! agent is between turns, with a line in the pane saying why.
//!
//! **Idle is the shim's to say.** It outlives a daemon restart, holds its own
//! ring of everything the agent said, and dials its old socket until something
//! answers. So this binds that socket once, asks for the ring from the start,
//! folds it the way the chat's activity always was, and waits for a turn to
//! end. A turn in flight is never cut off: the agent's half-finished work
//! would be lost with the process, and `--resume` can't reattach to it.
//!
//! Two things move a pane without a turn ending, because there is no turn to
//! lose: a shim that says its agent stopped or never started, and no shim at
//! all — none dialing in within `Timing::grace` (it died, or the pane did), or
//! the one that did hanging up. Silence never does: a turn can be one tool
//! call that runs for an hour (a whole `cargo test`), and moving the pane then
//! would kill it. An open permission ask doesn't hold a pane either: nothing
//! can answer one now, so waiting on it would hold the pane forever.
//!
//! A move that fails (tmux busy, a seat in flux) is tried again, backing off
//! from `Timing::retry` to `Timing::retry_max`, until it lands or the terminal
//! is gone; a daemon restart isn't needed to finish it.
//!
//! The shim's lines are read here as plain JSON, not through the link types
//! that went with it, so none of that crate had to stay for this.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use farcooler_agent_core::event::AgentEvent;
use farcooler_protocol::v1::AgentActivity;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use uuid::Uuid;

use crate::agent_supervisor::fold_activity;
use crate::service::Service;

/// What the pane prints before its TUI starts, so the person who left a chat
/// there knows why it's a terminal now.
pub const NOTICE: &str =
    "Far Cooler: chat view retired; this conversation continues here in the terminal.";

/// Where a chat pane's shim dials: the path the build before bound for it.
///
/// Per terminal and per runtime directory, and short, because a Unix socket
/// path can't exceed `sun_path` (`crate::paths::MAX_SOCKET_PATH`). The last
/// 12 hex digits of a v7 uuid are its random tail.
pub fn socket_path(runtime_dir: &Path, terminal: Uuid) -> PathBuf {
    let hex = terminal.simple().to_string();
    runtime_dir.join(format!("a-{}.sock", &hex[hex.len() - 12..]))
}

/// The pane's command, with the notice printed first.
///
/// `;` rather than `&&`, so a `printf` that failed can't keep the agent from
/// starting. Quoted for every login shell a pane runs under, fish included.
pub fn with_notice(command: &str) -> String {
    format!("printf '%s\\n' {}; {command}", crate::service::shell_quote(NOTICE))
}

/// How long the watch waits for each thing. Injectable so a test doesn't
/// wait a minute.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// For the shim to dial in at all.
    pub grace: Duration,
    /// Quiet after the last line before an idle agent is taken as idle: the
    /// replay arrives in batches, and the first may not hold the turn's end.
    pub settle: Duration,
    /// The wait before trying a failed move again, doubled each time.
    pub retry: Duration,
    /// The longest wait between tries.
    pub retry_max: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            grace: Duration::from_secs(60),
            settle: Duration::from_secs(2),
            retry: Duration::from_secs(5),
            retry_max: Duration::from_secs(5 * 60),
        }
    }
}

/// Why a pane is moving now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// Its agent is between turns.
    Idle,
    /// Its shim said the agent stopped or never started.
    Stopped,
    /// No shim dialed in, or the one that did hung up.
    NoShim,
}

/// What the watch found: why the pane moves, and the conversation the shim
/// said it was in, which is the one to resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub why: Why,
    pub session: Option<String>,
}

/// A chat's turn, folded from what its shim says.
#[derive(Debug, Default)]
pub struct Turn {
    activity: AgentActivity,
    /// Prompts Far Cooler's own queue still held, which the shim sends the
    /// moment a turn ends. A pane moved with any left would lose them.
    queued: usize,
    session: Option<String>,
    stopped: bool,
}

impl Turn {
    /// Fold one line the shim wrote. Lines it can't read are skipped: a
    /// newer or older shim's shape is no reason to hold a pane forever.
    pub fn hear(&mut self, line: &str) {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else { return };
        match message.get("kind").and_then(|k| k.as_str()) {
            Some("events" | "trimmed") => {
                let events = message.get("events").and_then(|e| e.as_array()).cloned().unwrap_or_default();
                for item in events {
                    let Some(raw) = item.get("event") else { continue };
                    let Ok(event) = serde_json::from_value::<AgentEvent>(raw.clone()) else { continue };
                    self.fold(&event);
                }
            }
            Some("established") => {
                self.session = message.get("session_id").and_then(|s| s.as_str()).map(str::to_string);
                // A new shim's handshake: nothing observed of its session yet.
                self.activity = AgentActivity::Unspecified;
            }
            Some("failed") => self.stopped = true,
            _ => {}
        }
    }

    fn fold(&mut self, event: &AgentEvent) {
        if let AgentEvent::PromptQueue { items } = event {
            self.queued = items.len();
        }
        self.activity = fold_activity(self.activity, event);
    }

    /// Whether the pane can move without losing anything: no turn running
    /// and nothing queued behind one.
    pub fn idle(&self) -> bool {
        self.activity != AgentActivity::Working && self.queued == 0
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn session(&self) -> Option<String> {
        self.session.clone()
    }
}

/// Bind a chat pane's old socket and wait until the pane can move.
///
/// Returns as soon as there's an answer; the listener is dropped with it, so
/// a shim still dialing afterwards finds nothing and keeps trying until the
/// respawn takes it away.
pub async fn watch(socket: &Path, timing: Timing) -> Verdict {
    let _ = std::fs::remove_file(socket);
    let listener = match UnixListener::bind(socket) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!(error = %e, path = %socket.display(), "could not bind a chat pane's socket; moving it now");
            return Verdict { why: Why::NoShim, session: None };
        }
    };
    let verdict = listen(&listener, timing).await;
    let _ = std::fs::remove_file(socket);
    verdict
}

async fn listen(listener: &UnixListener, timing: Timing) -> Verdict {
    let Ok(Ok((stream, _))) = tokio::time::timeout(timing.grace, listener.accept()).await else {
        return Verdict { why: Why::NoShim, session: None };
    };
    let (read_half, mut write_half) = stream.into_split();
    // From the start: the shim's ring is the only record of this turn.
    if write_half.write_all(b"{\"kind\":\"subscribe\",\"from_seq\":0}\n").await.is_err() {
        return Verdict { why: Why::NoShim, session: None };
    }
    let mut lines = BufReader::new(read_half).lines();
    let mut turn = Turn::default();
    loop {
        // Mid-turn, no deadline at all: only the turn's end, or the shim
        // going, moves the pane (see the module's docs).
        let next = if turn.idle() {
            match tokio::time::timeout(timing.settle, lines.next_line()).await {
                Ok(next) => next,
                Err(_) => return Verdict { why: Why::Idle, session: turn.session() },
            }
        } else {
            lines.next_line().await
        };
        match next {
            Ok(Some(line)) => {
                turn.hear(&line);
                if turn.stopped() {
                    return Verdict { why: Why::Stopped, session: turn.session() };
                }
            }
            // The shim hung up: its pane, or its agent, is gone.
            Ok(None) | Err(_) => return Verdict { why: Why::NoShim, session: turn.session() },
        }
    }
}

/// Watch every pane still in chat mode, and move each to its TUI at its next
/// idle. Called from `main`, once, beside `Service::resume_agent_listeners`.
pub fn start(service: &Arc<Service>) {
    start_with(service, Timing::default());
}

/// `start`, with the waits given.
pub fn start_with(service: &Arc<Service>, timing: Timing) {
    let Ok(worktrees) = service.list_worktrees() else { return };
    for worktree in worktrees {
        let Ok(terminals) = service.store.list_terminals_for_worktree(worktree.id) else { continue };
        for terminal in terminals.into_iter().filter(|t| t.pane_mode == farcooler_store::models::PaneMode::Agent) {
            let service = service.clone();
            let id = terminal.id;
            tokio::spawn(async move {
                let verdict = watch(&socket_path(service.root_dir(), id), timing).await;
                tracing::info!(terminal = %id, why = ?verdict.why, "moving a chat pane to its terminal");
                let session = verdict.session;
                retrying(timing, || {
                    let (service, session) = (service.clone(), session.clone());
                    async move { service.move_chat_pane_to_its_tui(id, session).await.map(|_| ()) }
                })
                .await;
            });
        }
    }
}

/// Try `attempt` until it works, waiting `Timing::retry` and doubling up to
/// `Timing::retry_max` between tries. A terminal that is gone ends it: there
/// is nothing left to move. Returns whether the move landed.
pub async fn retrying<F, Fut>(timing: Timing, mut attempt: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = farcooler_core::Result<()>>,
{
    let mut wait = timing.retry;
    loop {
        match attempt().await {
            Ok(()) => return true,
            Err(farcooler_core::DomainError::NotFound) => return false,
            Err(e) => {
                tracing::warn!(error = %e, retry_in = ?wait, "could not move a chat pane to its terminal; trying again");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(timing.retry_max);
            }
        }
    }
}

#[cfg(test)]
#[path = "chat_migration_tests.rs"]
mod tests;

//! How a message wake reaches its agent (ov-563): the seam in `wake()`.
//!
//! Typing into the box is the default and the fallback (`Delivery::Typed`).
//! Where an agent has a way in that doesn't touch its box, the message goes
//! that way instead, past none of the box checks, because nothing is pasted:
//!
//! - **codex** (`Delivery::CodexQueue`), opt-in only (`FARCOOLER_CODEX_QUEUE=1`;
//!   ruling R-77): `codex queue --thread <id> --message
//!   <text>`, run with the pane's `CODEX_HOME`, for at most `QUEUE_PATIENCE`;
//!   a wake whose command failed is typed from then on. The thread is the one the
//!   pane's codex holds open (`log_join::codex_thread_of`), so the pane is
//!   proven by its process first. A queued message runs as the next turn, mid
//!   turn included, which typing can't do for codex (`Held::Busy`). Any
//!   failure, an unknown thread included, falls back to typing.
//! - **claude** (`Delivery::ClaudeMailbox`): the daemon drops the message in
//!   the session's mailbox (`farcooler_agent_hooks::mailbox`) when its waiter
//!   is alive, and the waiter hands it over by exiting 42, which its hook command maps to the 2 that wakes
//!   the model (`asyncRewake`, `claude_mailbox`),
//!   idle or mid-turn. The wake settles when the waiter has taken it. A
//!   waiter that is not alive (a pane started before this, or between a
//!   delivery and the next Stop) means typing; so does one that hasn't taken
//!   the message in `MAILBOX_PATIENCE`, after the message is taken back out
//!   (`retract`), which loses to a waiter that got there first. Where it went
//!   is recorded (`routes`), so every settle without a delivery takes it back
//!   first (`retract_dropped`), and a pane's session moving on (`/clear`)
//!   can't drop a second copy.
//! - **cursor** (`Delivery::CursorFollowup`): its `stop` hook answers with a
//!   `followup_message` when a turn ends (`followup`). Nothing is sent from
//!   here: an idle cursor pane is typed to, and a working one waits for the
//!   hook.
//!
//! Who may be hostile: any pane's agent can send a message (`message.send`),
//! so the text is an argument, never a shell string; the thread id is a bare
//! uuid read from a rollout (`codex_thread_of`); and the log says that a
//! delivery failed, never the text.

use std::time::Duration;

use farcooler_agent_hooks::mailbox::{Mailbox, Slot, pid_alive};

use super::*;

/// How long `codex queue` may take before the message is typed instead: it
/// writes a row and returns, so a longer wait is a command that is stuck, and
/// the pump holds `wake_pump` while it waits.
const QUEUE_PATIENCE: Duration = Duration::from_secs(2);

/// The wakes whose `codex queue` failed, so the pump types them rather than
/// try the command again on every pass. Bounded: a wake settles in the end.
static QUEUE_FAILED: std::sync::Mutex<Vec<Uuid>> = std::sync::Mutex::new(Vec::new());

/// How long a message may sit in a mailbox with a live waiter that doesn't
/// take it: a waiter polls every 200 ms, so this is a waiter that isn't
/// waiting (a recycled pid, a wedged process). Then it's typed.
const MAILBOX_PATIENCE: Duration = Duration::from_secs(15);

/// Where a message wake goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Delivery {
    /// Pasted into the box: the default.
    Typed,
    /// `codex queue`.
    CodexQueue,
    /// Cursor's `stop` hook, when its turn ends; typed when it is idle.
    CursorFollowup,
    /// The session's mailbox and its `asyncRewake` waiter.
    ClaudeMailbox,
}

impl Delivery {
    /// The route for a pane launched as `preset`. Only a route; whether it
    /// works is decided where it's tried, and typing is the answer then.
    pub(super) fn of(preset: &str, terminal: Uuid) -> Self {
        match preset.split(':').next() {
            Some("codex") if codex_queue_on(terminal) => Delivery::CodexQueue,
            Some("cursor") => Delivery::CursorFollowup,
            Some("claude") => Delivery::ClaudeMailbox,
            _ => Delivery::Typed,
        }
    }
}

/// Whether the codex queue is opted into (ruling R-77: off by default, since
/// whether a plain codex TUI picks up a queued message is unverified):
/// `FARCOOLER_CODEX_QUEUE=1` in the daemon's environment.
#[cfg(not(test))]
fn codex_queue_on(_terminal: Uuid) -> bool {
    std::env::var("FARCOOLER_CODEX_QUEUE").is_ok_and(|v| v == "1")
}

/// In tests the opt-in is per pane, so tests don't turn it on for each other.
#[cfg(test)]
fn codex_queue_on(terminal: Uuid) -> bool {
    CODEX_ON_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner()).contains(&terminal)
}

#[cfg(test)]
/// The panes a test opted into the codex queue.
pub(super) static CODEX_ON_FOR_TESTS: std::sync::Mutex<Vec<Uuid>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
/// A `codex` for a pane, in place of the one on the PATH.
pub(super) static CODEX_FOR_TESTS: std::sync::Mutex<Vec<(Uuid, std::path::PathBuf)>> = std::sync::Mutex::new(Vec::new());

impl Watcher {
    /// Hand a message wake to `to` some way other than typing, if there is
    /// one and it works: `Some(Pass::Settled)`. `None` means type it.
    pub(super) async fn deliver_otherwise(&self, wake: &PendingWake, task: &Task, to: &Terminal) -> Option<Pass> {
        // Claimed already: typing may have begun, and nothing sends it twice.
        if wake.claimed_at.is_some() {
            return None;
        }
        match Delivery::of(&to.command_preset, to.id) {
            Delivery::CodexQueue => self.codex_queue(wake, task, to).await,
            Delivery::ClaudeMailbox => self.claude_mailbox(wake, task, to),
            Delivery::CursorFollowup | Delivery::Typed => None,
        }
    }

    /// The mailbox route for a claude pane: drop, wait for the waiter to take
    /// it, settle. `None` means type it (see this module's docs).
    ///
    /// Where a message went is recorded (`routes`), not worked out again: a
    /// pane's `agent_session_id` moves on `/clear`, and the message must be
    /// looked for, taken back and never dropped a second time elsewhere.
    fn claude_mailbox(&self, wake: &PendingWake, task: &Task, to: &Terminal) -> Option<Pass> {
        let root = crate::claude_mailbox::root_of(self.service.root_dir());
        let mut route = self.read_route(wake.note);
        // Another pane now: take it back from the first.
        if route.as_ref().is_some_and(|(pane, _)| *pane != to.id) {
            self.retract_dropped(wake);
            route = None;
        }
        let session = route.as_ref().map(|(_, session)| session.as_str()).or(to.agent_session_id.as_deref())?;
        let mailbox = Mailbox::of_session(&root, session)?;
        let name = Self::mail_name(wake);
        let waiting = || Some(Pass::Waiting(Held::Busy));
        match mailbox.slot(&name) {
            Slot::Taken => {
                self.mark_told(to.id);
                Some(self.settle(wake, Some(task), None))
            }
            Slot::Waiting => {
                let stale = mailbox.dropped_at(&name).and_then(|at| at.elapsed().ok()).is_none_or(|age| age > MAILBOX_PATIENCE);
                if mailbox.waiter(&pid_alive).is_some() && !stale {
                    return waiting();
                }
                // Taken back, to be typed; a waiter that got there first has it.
                if mailbox.retract(&name) {
                    self.retract_dropped(wake);
                    return None;
                }
                Some(self.settle(wake, Some(task), None))
            }
            Slot::Absent => {
                mailbox.waiter(&pid_alive)?;
                self.write_route(wake.note, to.id, session);
                if mailbox.drop_message(&name, &self.message_text(wake, task)).is_err() {
                    tracing::warn!(terminal = %to.id, "couldn't drop a message in a mailbox; typing it");
                    self.forget_route(wake.note);
                    return None;
                }
                waiting()
            }
        }
    }

    /// Remove the mailboxes of finished sessions (`claude_mailbox::sweep`),
    /// at most every ten minutes.
    pub(super) fn sweep_mailboxes(&self) {
        static LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        let now = now_millis();
        if now - LAST.load(Ordering::SeqCst) < 600_000 {
            return;
        }
        LAST.store(now, Ordering::SeqCst);
        let store = &self.service.store;
        crate::claude_mailbox::sweep(self.service.root_dir(), &|session| {
            store.terminals_with_agent_session(session).map(|rows| !rows.is_empty()).unwrap_or(true)
        });
    }

    /// A message's file name in a mailbox: its order, then its note.
    fn mail_name(wake: &PendingWake) -> String {
        format!("{:013}-{}", wake.enqueued_at.max(0), wake.note.simple())
    }

    fn route_file(&self, note: Uuid) -> std::path::PathBuf {
        crate::claude_mailbox::routes_of(self.service.root_dir()).join(note.simple().to_string())
    }

    /// The pane and session `note` was dropped for, if it was.
    fn read_route(&self, note: Uuid) -> Option<(Uuid, String)> {
        let text = std::fs::read_to_string(self.route_file(note)).ok()?;
        let (pane, session) = text.trim().split_once(' ')?;
        Some((pane.parse().ok()?, session.to_string()))
    }

    fn write_route(&self, note: Uuid, pane: Uuid, session: &str) {
        let file = self.route_file(note);
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        // Private, as the mailboxes are: it names a pane and a session.
        if let Some(dir) = file.parent() {
            let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        }
        if let Ok(mut out) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(file) {
            let _ = out.write_all(format!("{pane} {session}").as_bytes());
        }
    }

    fn forget_route(&self, note: Uuid) {
        let _ = std::fs::remove_file(self.route_file(note));
    }

    /// Take back a message dropped in a mailbox that no waiter has taken, and
    /// forget where it went. Before any settle without delivery (the task
    /// gone, the switch off, the give-up), or a waiter would deliver it after
    /// the wake is over. A message already taken stays delivered.
    pub(super) fn retract_dropped(&self, wake: &PendingWake) {
        if wake.kind != WakeKind::Message {
            return;
        }
        let Some((_, session)) = self.read_route(wake.note) else { return };
        let root = crate::claude_mailbox::root_of(self.service.root_dir());
        if let Some(mailbox) = Mailbox::of_session(&root, &session) {
            let name = Self::mail_name(wake);
            if !mailbox.retract(&name) {
                mailbox.clear(&name);
            }
        }
        self.forget_route(wake.note);
    }

    async fn codex_queue(&self, wake: &PendingWake, task: &Task, to: &Terminal) -> Option<Pass> {
        if QUEUE_FAILED.lock().unwrap_or_else(|e| e.into_inner()).contains(&wake.note) {
            return None;
        }
        let (preset, _tty, pid) = self.proven_agent(to).await.ok()?;
        if preset != "codex" {
            return None;
        }
        let rollout = codex_turn::rollout_now(pid).await?;
        let (thread, home) = tokio::task::spawn_blocking(move || crate::log_join::codex_thread_of(&rollout)).await.ok()??;
        let program = Self::codex_program(to.id)?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(["queue", "--thread", &thread, &format!("--message={}", self.message_text(wake, task))])
            .env("CODEX_HOME", home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let ran = tokio::time::timeout(QUEUE_PATIENCE, command.status()).await;
        if !matches!(ran, Ok(Ok(status)) if status.success()) {
            tracing::warn!(terminal = %to.id, "codex queue didn't take a message; typing it");
            let mut failed = QUEUE_FAILED.lock().unwrap_or_else(|e| e.into_inner());
            if failed.len() > 1024 {
                failed.clear();
            }
            failed.push(wake.note);
            return None;
        }
        self.mark_told(to.id);
        // Not recorded: a message's own note is the record.
        Some(self.settle(wake, Some(task), None))
    }

    #[cfg(not(test))]
    fn codex_program(_terminal: Uuid) -> Option<std::path::PathBuf> {
        farcooler_core::programs::find("codex")
    }

    #[cfg(test)]
    fn codex_program(terminal: Uuid) -> Option<std::path::PathBuf> {
        let set = CODEX_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        set.iter().find(|(id, _)| *id == terminal).map(|(_, p)| p.clone())
    }
}

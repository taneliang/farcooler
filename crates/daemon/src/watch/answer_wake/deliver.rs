//! How an agent message reaches its agent (ov-563): the seam in `wake()`.
//!
//! Only the agent-message route comes here (`route::Kind::AgentMessage`: a
//! lane's, a rep's, the orchestrator's, the runner's notices and a hold's
//! end). A person's words never do: they are typed, as their own turn, or
//! held (`route`, R-78). Typing into the box is the default and the fallback
//! (`Way::Typed`). Where an agent has a way in that doesn't touch its box,
//! the message goes that way instead, past none of the box checks, because
//! nothing is pasted:
//!
//! - **codex** (`Way::CodexQueue`), by default since ov-627 measured it
//!   (R-77 replaced; `FARCOOLER_CODEX_QUEUE=0` turns it off): `codex queue
//!   --thread <id> --message <text>`, run with the pane's `CODEX_HOME`, for at
//!   most `QUEUE_PATIENCE` (`queued::run_codex_queue`);
//!   a wake whose command failed is typed from then on. The thread is the one the
//!   pane's codex holds open (`log_join::codex_thread_of`), so the pane is
//!   proven by its process first. A queued message runs as the next turn, mid
//!   turn included, which typing can't do for codex (`Held::Busy`). Any
//!   failure, an unknown thread included, falls back to typing.
//! - **claude** (`Way::ClaudeMailbox`): the daemon drops the message in
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
//!   A lane's message to its orchestrator carries its sender
//!   (`messages::mail_sender`, ov-575, R-78), and the waiter then hands it to
//!   claude's inbox socket first, so claude shows it as "Message from
//!   @mac-ux on ov-505", expandable (`hook_inbox`); it falls back to the wake
//!   when the transcript doesn't show it. A rep's, the orchestrator's and the
//!   runner's have no sender and keep the wake. `FARCOOLER_CLAUDE_INBOX=0`
//!   turns the inbox off.
//! - **cursor** (`Way::CursorFollowup`): its `stop` hook answers with a
//!   `followup_message` when a turn ends (`followup`). Nothing is sent from
//!   here: an idle cursor pane is typed to, and a working one waits for the
//!   hook.
//!
//! Both side channels give the model the same plain role:user message as
//! typing (ov-627, measured on codex-cli 0.153.4 and cursor-agent
//! 2026.09.26), so a person's plain text may take them too (`queued`).
//!
//! Who may be hostile: any pane's agent can send a message (`message.send`),
//! so the text is an argument, never a shell string; the thread id is a bare
//! uuid read from a rollout (`codex_thread_of`); and the log says that a
//! delivery failed, never the text.

use std::time::Duration;

use farcooler_agent_hooks::mailbox::{Mailbox, Slot, pid_alive};

use super::route::Kind;
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
pub(super) enum Way {
    /// Pasted into the box: the default.
    Typed,
    /// `codex queue`.
    CodexQueue,
    /// Cursor's `stop` hook, when its turn ends; typed when it is idle.
    CursorFollowup,
    /// The session's mailbox and its `asyncRewake` waiter.
    ClaudeMailbox,
}

impl Way {
    /// The route for a pane launched as `preset`. Only a route; whether it
    /// works is decided where it's tried, and typing is the answer then.
    pub(super) fn of(preset: &str, terminal: Uuid) -> Self {
        match preset.split(':').next() {
            Some("codex") if codex_queue_on(terminal) => Way::CodexQueue,
            Some("cursor") => Way::CursorFollowup,
            Some("claude") => Way::ClaudeMailbox,
            _ => Way::Typed,
        }
    }
}

/// Whether the codex queue is on: by default, since ov-627 measured that a
/// plain codex TUI picks up a queued message as the same plain role:user
/// message typing gives (R-77, which made it opt-in, is replaced).
/// `FARCOOLER_CODEX_QUEUE=0` in the daemon's environment turns it off.
#[cfg(not(test))]
pub(super) fn codex_queue_on(_terminal: Uuid) -> bool {
    std::env::var("FARCOOLER_CODEX_QUEUE").map_or(true, |v| v != "0")
}

/// In tests it's off per pane, so a test can see the typed fallback.
#[cfg(test)]
pub(super) fn codex_queue_on(terminal: Uuid) -> bool {
    !CODEX_OFF_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner()).contains(&terminal)
}

#[cfg(test)]
/// The panes a test turned the codex queue off for.
pub(super) static CODEX_OFF_FOR_TESTS: std::sync::Mutex<Vec<Uuid>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
/// A `codex` for a pane, in place of the one on the PATH.
pub(super) static CODEX_FOR_TESTS: std::sync::Mutex<Vec<(Uuid, std::path::PathBuf)>> = std::sync::Mutex::new(Vec::new());

impl Watcher {
    /// Hand an agent message, `text`, to `to` some way other than typing, if
    /// there is one and it works: `Some(Pass::Settled)`. `None` means type it.
    pub(super) async fn deliver_otherwise(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Option<Pass> {
        // Claimed already: typing may have begun, and nothing sends it twice.
        if wake.claimed_at.is_some() {
            return None;
        }
        // A person's words are never sent another way (`route`): checked here
        // too, so no caller can drop one in a mailbox. One a daemon from
        // before ov-620 dropped already is settled or taken back, never sent
        // twice (`recalled`).
        if self.route_of(wake) != Kind::AgentMessage {
            return self.recalled(wake, task, to);
        }
        match Way::of(&to.command_preset, to.id) {
            Way::CodexQueue => self.codex_queue(wake, task, to, text).await,
            Way::ClaudeMailbox => self.claude_mailbox(wake, task, to, text),
            Way::CursorFollowup | Way::Typed => None,
        }
    }

    /// The mailbox route for a claude pane: drop, wait for the waiter to take
    /// it, settle. `None` means type it (see this module's docs).
    ///
    /// Where a message went is recorded (`routes`), not worked out again: a
    /// pane's `agent_session_id` moves on `/clear`, and the message must be
    /// looked for, taken back and never dropped a second time elsewhere.
    fn claude_mailbox(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Option<Pass> {
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
                Some(self.settle(wake, Some(task), told(wake.kind, to, Turn::Between)))
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
                let text = match self.mail_sender(wake, task) {
                    Some(sender) => farcooler_agent_hooks::mailbox::with_sender(&sender, text),
                    None => text.to_string(),
                };
                if mailbox.drop_message(&name, &text).is_err() {
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
    /// Any message may have a route: a person's dropped by a daemon from
    /// before ov-620 too (review 1, S3).
    pub(super) fn retract_dropped(&self, wake: &PendingWake) {
        // A person's answer is typed, never mailboxed; an agent's from a pane is.
        if wake.kind == WakeKind::Answer && self.route_of(wake) != Kind::AgentMessage {
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

    /// A person's message a daemon from before ov-620 dropped in a mailbox
    /// (its route recorded), found by the pump that now types it: taken by
    /// the waiter, it's settled as told, never typed again; still waiting,
    /// it's taken back and typed; already gone, it's typed. Nothing new is
    /// ever dropped here.
    fn recalled(&self, wake: &PendingWake, task: &Task, to: &Terminal) -> Option<Pass> {
        let (_, session) = self.read_route(wake.note)?;
        let root = crate::claude_mailbox::root_of(self.service.root_dir());
        let name = Self::mail_name(wake);
        let Some(mailbox) = Mailbox::of_session(&root, &session) else {
            self.forget_route(wake.note);
            return None;
        };
        if matches!(mailbox.slot(&name), Slot::Waiting) && mailbox.retract(&name) {
            self.forget_route(wake.note);
            return None;
        }
        if matches!(mailbox.slot(&name), Slot::Taken) {
            self.mark_told(to.id);
            return Some(self.settle(wake, Some(task), None));
        }
        self.forget_route(wake.note);
        None
    }

    async fn codex_queue(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Option<Pass> {
        if QUEUE_FAILED.lock().unwrap_or_else(|e| e.into_inner()).contains(&wake.note) {
            return None;
        }
        let Some(queued) = self.run_codex_queue(to, text).await else {
            tracing::warn!(terminal = %to.id, "codex queue didn't take a message; typing it");
            let mut failed = QUEUE_FAILED.lock().unwrap_or_else(|e| e.into_inner());
            if failed.len() > 1024 {
                failed.clear();
            }
            failed.push(wake.note);
            return None;
        };
        self.mark_told(to.id);
        self.watch_queued(to.id, wake.note, text, queued, Some(task));
        // A message's own note is its record; a hold's end says it was told.
        Some(self.settle(wake, Some(task), told(wake.kind, to, Turn::Between)))
    }

    /// `codex queue` for `text` into the codex `to` runs, past its process
    /// and the thread it holds open: where to look for it running, or `None`
    /// when it didn't take it (no codex, no thread, no program, a failure, or
    /// `QUEUE_PATIENCE` out).
    pub(super) async fn run_codex_queue(&self, to: &Terminal, text: &str) -> Option<super::queued::Queued> {
        let (preset, _tty, pid) = self.proven_agent(to).await.ok()?;
        if preset != "codex" {
            return None;
        }
        let rollout = codex_turn::rollout_now(pid).await?;
        let from = std::fs::metadata(&rollout).map_or(0, |m| m.len());
        let at = rollout.clone();
        let (thread, home) = tokio::task::spawn_blocking(move || crate::log_join::codex_thread_of(&at)).await.ok()??;
        let program = Self::codex_program(to.id)?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(["queue", "--thread", &thread, &format!("--message={text}")])
            .env("CODEX_HOME", home)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let queued_at = now_millis();
        let ran = tokio::time::timeout(QUEUE_PATIENCE, command.status()).await;
        matches!(ran, Ok(Ok(status)) if status.success()).then_some(super::queued::Queued { pid, rollout, from, at: queued_at })
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

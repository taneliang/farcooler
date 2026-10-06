//! Answering a decision wakes the agent waiting on it, but only when typing
//! there is provably safe. When in doubt, it doesn't type, and it says so.
//!
//! Every answer, from every client, is an ANSWER note written through
//! `task.note` (`task_ops::note`): `farcooler task note --kind answer`, the
//! Mac's board and Needs You, and both phones' Needs You and task screens.
//! The store writes a person's answer and, when the task's workspace has
//! the switch on, its queue row (`answer_wakes`) in one transaction
//! (`Store::add_note_waking`). Only `user` answers queue: `actor` is
//! self-asserted on the wire, so an agent can claim to be the person, but
//! `task.note` already needs Control scope, the scope that can type into
//! any pane directly. `pump_wakes` then tries each queued answer right away
//! and again after every sample.
//!
//! **Whom.** A running agent or orchestrator terminal in the task's
//! workspace: the newest opened for the task (`Terminal.task_id`), else the
//! only agent pane in the task's worktree (never the main checkout's: an
//! agent opened by hand there is nobody's), else the workspace's
//! orchestrator. Never another workspace's terminal or a Changes pane. An
//! agent pane counts only if launched as an agent; a pane ADOPTED as the
//! orchestrator counts whatever it was launched as (a shell someone runs
//! claude in by hand), and check 3 proves what it runs. A shell pane not
//! adopted never counts, whatever runs in it. Nobody: "Nobody to tell about
//! the decision".
//!
//! **The gate, for a TUI pane.** Every check runs on this pass, against the
//! pane as it is now, and every one fails closed:
//! 1. The watcher reads the terminal Idle, Done or Working, and this runner
//!    hasn't told it anything in the last `TOLD_SPACING_MS` (`told`): one
//!    answer at a time per terminal. Nothing waits on catching a transition;
//!    each tick looks at the pane as it is, and check 4 is what proves it
//!    between turns, or mid-turn in an agent that queues (below).
//! 2. Nobody has typed there lately (`typed_lately`).
//! 3. The pane's foreground process is the agent its preset names (for an
//!    adopted orchestrator launched as a shell, any agent), proven by its
//!    executable, or for a Node install by the script Node runs
//!    (`foreground_agent`), never by screen text. Checks 4 and 5 read the
//!    pane as that proven agent. A shell, or anything not recognized, isn't
//!    typed into.
//! 4. A fresh capture classifies as Idle, or as Working in an agent that
//!    queues what's sent mid-turn (`queues_mid_turn`), AND `composer::read`
//!    positively recognizes the agent's box and finds it empty. A menu, a
//!    picker, a prompt, an unfamiliar screen or a draft: not typed into.
//! 5. The agent has bracketed paste on, as tmux reports it. tmux older than
//!    3.7 (Ubuntu 24.04 has 3.4) can't report it, so there it's read from
//!    the pane's own output (`paste_mode`), which the daemon follows for
//!    every agent pane from when it starts the agent, and for an adopted
//!    orchestrator from when it's adopted (`may_be_typed_to`). Known only
//!    if the daemon has seen every byte since the agent last set or reset
//!    it, and the pane still runs that program: for an adopted shell, the
//!    shell, so an agent started there by hand before the adoption stays
//!    unknown until it sets the mode again. Otherwise the answer waits, as for
//!    any other check, and is typed if the agent sets the mode again in
//!    time; if not, its note says why: "Not delivered: Far Cooler couldn't
//!    tell whether the agent takes a paste…".
//!
//! **Typing.** The row is claimed first, in its own write. The text goes in
//! as one bracketed paste. The box is then read back until it holds exactly
//! the text; only then is Enter (`\r`) sent, on its own. If the box never
//! matches, or someone types meanwhile, no Enter: "Paste left in the
//! composer; not sent", and the text is left for the person rather than
//! erased. A send that fails after the claim, and any row found claimed and
//! unfinished (a crash mid-typing), is "Couldn't confirm the agent got the
//! decision" and is never typed again.
//!
//! **Mid-turn** (ov-360). claude takes a message submitted while it works,
//! as the person typing it would, and queues it for its next turn. Measured
//! on claude 2.1.290: the box is the same box, a paste and Enter go in, and
//! the message arrives as the next prompt. So a working claude is typed into
//! under the same checks, and what it's sent waits in its own queue, not in
//! this one. An Enter on a dialog would answer it, so the Enter goes in
//! under the session's fence, which claude's `PreToolUse` hook must pass
//! before any dialog is drawn, and only with no tool call in flight and
//! nothing on the screen (`mid_turn`); a dialog in the way is "Paste left in
//! the composer; not sent". That the queue took it is confirmed after, by the `enqueue` record
//! claude writes to its transcript; unconfirmed is "Couldn't confirm". A
//! claude whose session can't be found, or that no hook was ever heard from,
//! waits for the turn to end, as every agent did before; so does codex,
//! which queues too but raises approvals no hook tells of.
//!
//! **A chat pane** takes the answer as a prompt on its agent channel, after
//! check 1: the channel can't reach a shell, a menu or a draft, and a turn
//! running holds it in the pane's own queue (`ChatSession::prompt`).
//!
//! **Superseded.** A newer answer on the same task replaces an older one not
//! yet told: only the newest is told, and the older is noted "Not
//! delivered: a newer answer replaced it." One told is never replaced, even
//! one that waits in a working agent's own queue: a newer answer is told
//! after it, and the agent reads them in the order they were told.
//!
//! **Waiting, said.** The first pass an answer waits on, its task says so
//! once: "Waiting to tell the orchestrator about the decision: it's busy."
//!
//! **Bounded.** An answer not told within `GIVE_UP_AFTER_MS` is noted "Not
//! delivered: <why it last waited>" and dropped.
//!
//! **A subagent's task.** A task a subagent works (ov-213) has no pane, so
//! the orchestrator is told, and the message names the subagent open on it
//! so the orchestrator can pass the decision on with `SendMessage`.
//!
//! **A hold that ended** (ov-212) is told through the same pump and under the
//! same five checks, from its own queue (`hold_wakes`, kind `HoldEnded`):
//! "Hold ended on ov-12 (“…”): Held until Oct 5, 9:00 AM. That time has
//! come. Start it when you're ready." It is never superseded by an answer
//! or by another hold, is dropped silently if the task has left Backlog
//! since, and is given up on after the same half hour. The workspace's
//! wake-on-answer switch covers it: it's what allows typing into a pane.

use std::sync::atomic::Ordering;
use std::time::Duration;

use farcooler_agent::link::DaemonMessage;
use farcooler_core::composer::{self, Composer};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::{PendingWake, WakeKind};
use farcooler_store::models::{Actor, NoteKind, PaneMode, Task, TaskNote, TaskStatus, Terminal, TerminalRole};
use uuid::Uuid;

use super::{Watcher, anyone_watching, now_millis};
use crate::runtime::{Runtime, last_input};

/// How long after the last keystroke a pane counts as quiet.
pub(crate) const QUIET_MS: i64 = 5_000;
/// The same, while a client says the pane is in front of a person
/// (`terminal.watching`): someone looking at it may only be pausing.
pub(crate) const QUIET_WATCHED_MS: i64 = 15_000;
/// How long after telling a terminal one answer the next may be told: long
/// enough for the agent to have drawn its turn, so check 4 sees it working.
pub(crate) const TOLD_SPACING_MS: i64 = 2_000;
/// How long an answer may wait to be told before it's dropped.
pub(crate) const GIVE_UP_AFTER_MS: i64 = 30 * 60 * 1_000;
/// Long enough for any answer picked from options and most written ones;
/// short enough to stay under claude's collapsed-paste threshold.
const LONGEST_ANSWER: usize = 200;
const LONGEST_TITLE: usize = 80;
/// How long a pasted answer has to show up in the box before it's judged
/// not to have.
const PASTE_SETTLES: Duration = Duration::from_secs(2);
const PASTE_POLL: Duration = Duration::from_millis(100);

const COULDNT_CONFIRM: &str = "Couldn't confirm the agent got the decision";
const PASTE_LEFT: &str = "Paste left in the composer; not sent";
const LEFT_AT_A_SHELL: &str = "Answer left at a shell prompt; not run";
const SUPERSEDED: &str = "Not delivered: a newer answer replaced it.";
const NOBODY: &str = "Nobody to tell about the decision";
const WAITING: &str = "Waiting to tell";

/// What the agent is told.
#[cfg(test)]
pub(crate) fn message(key: &str, title: &str, answer: &str) -> String {
    message_for(key, title, answer, &[])
}

/// A subagent working a task: its id, which `SendMessage` names, and its
/// label.
pub(crate) type Subagent = (String, String);

/// What the agent is told, when `subagents` are working the task it was asked
/// about: the orchestrator is the one told (a subagent has no pane), and it
/// passes the decision on to the subagent by name.
pub(crate) fn message_for(key: &str, title: &str, answer: &str, subagents: &[Subagent]) -> String {
    let full = one_line(answer, usize::MAX);
    let cut = one_line(answer, LONGEST_ANSWER);
    let title = one_line(title, LONGEST_TITLE);
    let stop = if cut.ends_with(['.', '!', '?', '…']) { "" } else { "." };
    let more = if cut != full { format!(" (Full answer: farcooler task show {key}.)") } else { String::new() };
    // Curly quotes: in fish, `("…")` left at a prompt is a command
    // substitution, and these are no quote to any shell.
    let then = match subagents {
        [] => "Continue.".to_string(),
        [(id, label)] => format!("Its subagent ({}) is working on it: pass this on, then continue.", named(id, label)),
        many => {
            let names: Vec<String> = many.iter().map(|(id, label)| named(id, label)).collect();
            format!("Its subagents ({}) are working on it: pass this on, then continue.", names.join("; "))
        }
    };
    format!("Decision on {key} (“{title}”): {cut}{stop}{more} {then}")
}

/// What the agent is told when a task held until a time reaches it.
pub(crate) fn hold_message(key: &str, title: &str, said: &str) -> String {
    let title = one_line(title, LONGEST_TITLE);
    let said = one_line(said, LONGEST_ANSWER);
    format!("Hold ended on {key} (“{title}”): {said} Start it when you're ready.")
}

/// A subagent as the message names it: `a3fd8fceef581c787, “ov-12 Mac polish”`, or the
/// id alone when it has no label.
fn named(id: &str, label: &str) -> String {
    let id = one_line(id, LONGEST_TITLE);
    match one_line(label, LONGEST_TITLE).as_str() {
        "" => id,
        label => format!("{id}, “{label}”"),
    }
}

/// `raw` on one line, safe to type: line breaks and tabs become spaces, runs
/// of space become one, and every control character (C0, DEL and C1, which
/// is where ESC and CSI live) and every invisible format character (bidi
/// overrides and isolates, zero-width marks) is written out as `\u{..}`
/// rather than sent. Cut to `longest` characters, the last of them `…`.
pub(crate) fn one_line(raw: &str, longest: usize) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() || invisible(c) => out.extend(c.escape_unicode()),
            c => out.push(c),
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() <= longest {
        return out;
    }
    let cut: String = out.chars().take(longest.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// What Ask the Orchestrator pastes: `raw` on one line with every control and
/// invisible character written out (`one_line`), cut at 300 characters, and a
/// space after it if it had one, so the cursor sits past a colon. Never a line
/// break, so never an Enter.
pub(crate) fn draft_text(raw: &str) -> String {
    let line = one_line(raw, 300);
    if raw.ends_with(' ') && !line.is_empty() { format!("{line} ") } else { line }
}

/// Format characters that draw nothing and can disguise what's around them.
fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Why an answer is still waiting, as its "Not delivered" note says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Held {
    Busy,
    Prompt,
    Draft,
    Typing,
    NotAnAgent,
    Unfamiliar,
    Unproven,
}

impl Held {
    /// Why it's waiting now, as the "Waiting to tell" note says it.
    fn now(self) -> &'static str {
        match self {
            Held::Busy => "it's busy",
            Held::Prompt => "a question or menu is showing",
            Held::Draft => "there's a draft in its box",
            Held::Typing => "someone is typing there",
            Held::NotAnAgent => "no agent is running in its pane",
            Held::Unfamiliar => "its screen isn't one Far Cooler recognizes",
            // Only a tmux older than 3.7 leaves it unproven: say so now,
            // not first at the deadline.
            Held::Unproven => {
                "this runner’s tmux is older than 3.7, so Far Cooler can’t tell yet whether the agent accepts a paste. \
                 Restarting the agent in its pane lets it tell"
            }
        }
    }

    fn why(self) -> &'static str {
        match self {
            Held::Busy => "the agent stayed busy",
            Held::Prompt => "a question or menu was showing",
            Held::Draft => "there was a draft in the agent's box",
            Held::Typing => "someone kept typing there",
            Held::NotAnAgent => "no agent was running in the pane",
            Held::Unfamiliar => "the agent's screen wasn't one Far Cooler recognizes",
            Held::Unproven => {
                "Far Cooler couldn’t tell whether the agent accepts a paste. This runner’s tmux is older than \
                 3.7. Restart the agent in its pane, or update tmux to 3.7 or later"
            }
        }
    }
}

/// Where the agent was when it was typed to: between turns, so the message
/// is its next prompt, or mid-turn, so the message waits in its queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Turn {
    Between,
    During,
}

/// Whether an agent takes a message submitted mid-turn into a queue of its
/// own, and says through a hook when it raises a dialog that would take the
/// Enter (see this module's docs, "Mid-turn"). codex queues, but doesn't say.
pub(crate) fn queues_mid_turn(preset: &str) -> bool {
    preset.split(':').next() == Some("claude")
}

/// The agent in front of a pane, as checks 3 to 5 proved it.
pub(crate) struct Proven {
    pub(crate) preset: &'static str,
    pub(crate) tty: String,
    /// The agent's process, which names its session (`mid_turn`).
    pub(crate) pid: i32,
    pub(crate) turn: Turn,
}

/// What a wake came to on one pass.
#[derive(Debug, PartialEq, Eq)]
enum Pass {
    /// Told, or settled for good another way.
    Settled,
    /// Not yet, and why.
    Waiting(Held),
}

impl Watcher {
    /// An answer was written and queued (`Store::add_note_waking`): try it.
    pub fn answered(&self, _task: &Task, _note: &TaskNote, queued: bool) {
        if queued {
            self.wakes_hint.store(true, Ordering::SeqCst);
            self.spawn_wake_pump();
        }
    }

    /// `pump_wakes` off the caller's path, when there is a runtime.
    pub(super) fn spawn_wake_pump(&self) {
        if !self.wakes_hint.load(Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.me.upgrade() else { return };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move { me.pump_wakes().await });
        }
    }

    /// Try every queued answer once. One pass at a time: a second caller
    /// while one runs returns at once, so no answer is typed by two passes.
    pub async fn pump_wakes(&self) {
        let Ok(_one_pass) = self.wake_pump.try_lock() else { return };
        // Cleared before the read, so an answer queued during this pass sets
        // it again and the next tick reads it.
        self.wakes_hint.store(false, Ordering::SeqCst);
        let store = &self.service.store;
        let pending = match (store.pending_answer_wakes(), store.pending_hold_wakes()) {
            (Ok(mut pending), Ok(holds)) => {
                pending.extend(holds);
                pending.sort_by_key(|wake| wake.enqueued_at);
                pending
            }
            (Err(e), _) | (_, Err(e)) => {
                tracing::warn!(error = %e, "couldn't read the answers waiting to be told");
                self.wakes_hint.store(true, Ordering::SeqCst);
                return;
            }
        };
        for (n, wake) in pending.iter().enumerate() {
            // A newer answer on the same task is waiting too: tell only it.
            // Holds aren't answers, and one that ended is told on its own.
            if wake.kind == WakeKind::Answer
                && wake.claimed_at.is_none()
                && pending[n + 1..].iter().any(|later| later.kind == WakeKind::Answer && later.task == wake.task)
            {
                self.settle(wake, None, Some(SUPERSEDED.into()));
                continue;
            }
            if let Pass::Waiting(held) = self.wake(wake).await {
                self.wake_holds.lock().unwrap_or_else(|e| e.into_inner()).insert(wake.note, held);
                self.wakes_hint.store(true, Ordering::SeqCst);
            }
        }
    }

    async fn wake(&self, wake: &PendingWake) -> Pass {
        // Claimed and never finished: the daemon stopped mid-typing. It may
        // have reached the agent, so it's never typed again.
        if wake.claimed_at.is_some() {
            return self.settle(wake, None, Some(couldnt_confirm(wake.kind)));
        }
        let store = &self.service.store;
        let task = match store.get_task(wake.task) {
            Ok(task) => task,
            Err(DomainError::NotFound) => return self.settle(wake, None, None),
            Err(_) => return Pass::Waiting(Held::Busy),
        };
        // Turned off since it was queued: let it go, saying nothing. The one
        // switch covers a hold that ended: it's what lets this runner type
        // into an agent's pane at all.
        if !store.get_workspace(task.workspace_id).is_ok_and(|w| w.wake_on_answer) {
            return self.settle(wake, None, None);
        }
        // A hold that ended is news only while the task is still waiting to be
        // started: one started or closed since has nothing to be told.
        if wake.kind == WakeKind::HoldEnded && task.status != TaskStatus::Backlog {
            return self.settle(wake, None, None);
        }
        if now_millis() - wake.enqueued_at > GIVE_UP_AFTER_MS {
            let held = self.wake_holds.lock().unwrap_or_else(|e| e.into_inner()).remove(&wake.note);
            let why = held.unwrap_or(Held::Busy).why();
            return self.settle(wake, Some(&task), Some(format!("Not delivered: {why}.")));
        }
        let Some(to) = self.recipient(&task).await else {
            return self.settle(wake, Some(&task), Some(nobody(wake.kind)));
        };
        let pass = match self.ready(&to).await {
            Err(held) => Pass::Waiting(held),
            Ok(turn) => {
                let text = match wake.kind {
                    WakeKind::Answer => {
                        message_for(&task.key, &task.title, &wake.body, &self.subagents_to_pass_on(&task, &to))
                    }
                    WakeKind::HoldEnded => hold_message(&task.key, &task.title, &wake.body),
                };
                if to.pane_mode == PaneMode::Agent {
                    self.prompt(wake, &task, &to, &text, turn).await
                } else {
                    self.type_into(wake, &task, &to, &text).await
                }
            }
        };
        if let Pass::Waiting(held) = pass {
            self.say_waiting(wake, &task, &to, held);
        }
        pass
    }

    /// The first time an answer waits, say so on its task, once: whom it's
    /// waiting to reach, and why. Once per answer across restarts, read
    /// from the task's record.
    fn say_waiting(&self, wake: &PendingWake, task: &Task, to: &Terminal, held: Held) {
        // Held before on this run: said already.
        if self.wake_holds.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&wake.note) {
            return;
        }
        let store = &self.service.store;
        let Ok(notes) = store.notes_for(task.id, Some(NoteKind::Progress)) else { return };
        if notes.iter().any(|n| n.actor == Actor::Runner && n.at >= wake.enqueued_at && n.body.starts_with(WAITING)) {
            return;
        }
        let body = match wake.kind {
            WakeKind::Answer => format!("{WAITING} {} about the decision: {}.", spoken_name(to), held.now()),
            WakeKind::HoldEnded => format!("{WAITING} {} the hold ended: {}.", spoken_name(to), held.now()),
        };
        match store.add_note(task.id, NoteKind::Progress, Actor::Runner, &body, serde_json::json!({})) {
            Ok(_) => self.announce_task_changed(task, None, Actor::Runner),
            Err(e) => tracing::warn!(note = %wake.note, error = %e, "couldn't say an answer is waiting"),
        }
    }

    /// Check 1 and 2 of the gate: the watcher reads it Idle, Done or
    /// Working, newly since the last answer it was told, and nobody is
    /// typing. Working is only the watcher's word: for a TUI pane, check 4
    /// decides on a fresh capture whether a turn runs and whether that agent
    /// queues what it's sent.
    async fn ready(&self, to: &Terminal) -> std::result::Result<Turn, Held> {
        let (activity, _, _) = self.activity(to.id).await;
        let turn = match activity {
            AgentActivity::Idle | AgentActivity::Done => Turn::Between,
            AgentActivity::Working => Turn::During,
            AgentActivity::Blocked => return Err(Held::Prompt),
            _ => return Err(Held::Busy),
        };
        let told = self.told.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if told.is_some_and(|told| now_millis() - told < TOLD_SPACING_MS) {
            return Err(Held::Busy);
        }
        if self.typed_lately(to.id, now_millis()) {
            return Err(Held::Typing);
        }
        Ok(turn)
    }

    /// A chat pane: the answer as a prompt on its agent channel, which holds
    /// it in the pane's queue while a turn runs.
    async fn prompt(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str, turn: Turn) -> Pass {
        if !matches!(self.service.store.claim_wake(wake), Ok(true)) {
            return Pass::Settled;
        }
        let prompt = DaemonMessage::Prompt { text: text.to_string(), images: Vec::new() };
        if !self.service.agents().send(to.id, prompt) {
            return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind)));
        }
        self.mark_told(to.id);
        self.settle(wake, Some(task), Some(told(wake.kind, to, turn)))
    }

    /// A TUI pane: checks 3 to 5 (`proven_tui`), then claim, paste, read
    /// back, Enter.
    async fn type_into(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Pass {
        let proven = match self.proven_tui(to).await {
            Ok(proven) => proven,
            Err(held) => return Pass::Waiting(held),
        };
        let witness = match proven.turn {
            Turn::Between => None,
            Turn::During => match self.witness(&proven, text).await {
                Some(witness) => Some(witness),
                None => return Pass::Waiting(Held::Busy),
            },
        };
        let (preset, tty) = (proven.preset, proven.tty.as_str());
        match self.service.store.claim_wake(wake) {
            Ok(true) => {}
            Ok(false) => return Pass::Settled,
            Err(_) => return Pass::Waiting(Held::Busy),
        }
        // From here nothing is retried: whatever happens is settled.
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let paste: String = crate::pastes::encode_paste(true, text).iter().map(|b| format!("{b:02x}")).collect();
        let started = now_millis();
        if self.fail_sends_for_tests() || runtime.send_bytes_hex(to.id, &paste).await.is_err() {
            return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind)));
        }
        let deadline = tokio::time::Instant::now() + PASTE_SETTLES;
        let mut held_exactly = false;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
                break;
            }
            if let Ok(Ok((now, _))) = self.box_of(to, preset).await
                && composer::holds_exactly(&now, text)
            {
                held_exactly = true;
                break;
            }
        }
        // Someone typed between the matching capture and now: no Enter.
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
            held_exactly = false;
        }
        if !held_exactly {
            // The agent may have gone, and a shell taken the pane: say where
            // the text really is.
            if foreground_agent(tty).await != Some(preset) {
                return self.settle(wake, Some(task), Some(LEFT_AT_A_SHELL.into()));
            }
            return self.settle(wake, Some(task), Some(PASTE_LEFT.into()));
        }
        let entered = match &witness {
            None => runtime.send_bytes_hex(to.id, "0d").await.map_err(|_| mid_turn::NoEnter::Failed),
            Some(witness) => self.enter(to, preset, witness, text).await,
        };
        match entered {
            Ok(()) => {}
            Err(mid_turn::NoEnter::Failed) => return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind))),
            Err(_) => return self.settle(wake, Some(task), Some(PASTE_LEFT.into())),
        }
        self.mark_told(to.id);
        if let Some(witness) = witness
            && !self.queued(&witness, text).await
        {
            return self.settle(wake, Some(task), Some(couldnt_confirm(wake.kind)));
        }
        self.settle(wake, Some(task), Some(told(wake.kind, to, proven.turn)))
    }

    /// Checks 3 to 5 of the gate, for a TUI pane: the agent in front proven
    /// by its process, its box recognized and empty, and bracketed paste on.
    /// Returns the proven agent and whether a turn is running, or why not.
    /// Every check fails closed.
    async fn proven_tui(&self, to: &Terminal) -> std::result::Result<Proven, Held> {
        let launched = to.command_preset.split(':').next().unwrap_or_default();
        let tty = self
            .service
            .inventory_snapshot()
            .claimants(to.id)
            .into_iter()
            .find(|p| p.proves_life())
            .map(|p| p.tty.clone());
        let Some(tty) = tty else { return Err(Held::NotAnAgent) };
        // The agent in front, proven by its process. A pane launched as an
        // agent must be running that one. An adopted orchestrator launched
        // as anything else is read as the agent its process proves.
        let Some((preset, pid)) = foreground(&tty).await else { return Err(Held::NotAnAgent) };
        if launched != preset && (is_an_agent_preset(launched) || to.role != TerminalRole::Orchestrator) {
            return Err(Held::NotAnAgent);
        }
        let Ok(composer) = self.box_of(to, preset).await else { return Err(Held::Unfamiliar) };
        let turn = match composer {
            Ok((Composer::Empty, turn)) => turn,
            Ok((Composer::Holds(_), _)) => return Err(Held::Draft),
            Ok((Composer::Unrecognized, _)) => return Err(Held::Unfamiliar),
            Err(held) => return Err(held),
        };
        let bracketed = match self.service.pane_bracketed_paste(to.id).await {
            Ok(Some(on)) => on,
            // tmux can't say (older than 3.7): the pane's own output can, if
            // this daemon has followed it since the agent last set the mode.
            // Not known yet: it may be once the agent sets it again (a
            // respawn, a stream followed afresh), so the answer waits.
            Ok(None) => match self.service.streamed_bracketed_paste(to.id).await {
                Some(on) => on,
                None => return Err(Held::Unproven),
            },
            Err(_) => false,
        };
        if !bracketed {
            return Err(Held::Unfamiliar);
        }
        Ok(Proven { preset, tty, pid, turn })
    }

    /// Ask the Orchestrator (ov-184): leave `text` in a TUI pane's box as one
    /// bracketed paste, and NEVER press Enter, so the person finishes the
    /// sentence. The same gate as typing an answer (`ready`, then
    /// `proven_tui`): the pane is a proven agent, between turns or mid-turn
    /// in one that queues, nobody is typing, its box is empty, and bracketed
    /// paste is known to be on. Any failure is an error, and nothing is
    /// typed; the Mac then copies the text instead.
    ///
    /// Not recorded as someone typing (`marks: None`), as an answer isn't.
    pub(crate) async fn draft_into(&self, id: Uuid, text: &str) -> Result<()> {
        let to = self.service.store.get_terminal(id)?;
        if to.pane_mode == PaneMode::Agent {
            // A chat pane has a composer of the app's own; no TUI to paste to.
            return Err(DomainError::InvalidArgument { what: "terminal" });
        }
        if to.pane_mode == PaneMode::Changes
            || !may_be_typed_to(&to.command_preset, to.role)
            || !self.service.is_running(&to)
        {
            return Err(DomainError::Conflict { what: "not_pasteable" });
        }
        let text = draft_text(text);
        if text.trim().is_empty() {
            return Err(DomainError::InvalidArgument { what: "text" });
        }
        self.ready(&to).await.map_err(|_| DomainError::Conflict { what: "not_pasteable" })?;
        self.proven_tui(&to).await.map_err(|_| DomainError::Conflict { what: "not_pasteable" })?;
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let paste: String = crate::pastes::encode_paste(true, &text).iter().map(|b| format!("{b:02x}")).collect();
        if self.fail_sends_for_tests() {
            return Err(DomainError::OperationFailed);
        }
        runtime.send_bytes_hex(to.id, &paste).await
    }

    /// The pane's box as a fresh capture shows it, and whether a turn is
    /// running, or why it's no box to type into: a prompt up, or the agent
    /// working when it doesn't queue (`queues_mid_turn`). `Err` when the
    /// screen couldn't be read.
    async fn box_of(&self, to: &Terminal, preset: &str) -> Result<std::result::Result<(Composer, Turn), Held>> {
        let (screen, _, _) = self.service.screen(to.id).await?;
        let activity = self.service.registry().classify(preset, &screen);
        Ok(match activity {
            AgentActivity::Working if queues_mid_turn(preset) => Ok((composer::read(preset, &screen), Turn::During)),
            AgentActivity::Working => Err(Held::Busy),
            AgentActivity::Blocked => Err(Held::Prompt),
            AgentActivity::Idle => Ok((composer::read(preset, &screen), Turn::Between)),
            _ => Err(Held::Unfamiliar),
        })
    }

    /// Whether a test asked the next paste to fail as a send would.
    fn fail_sends_for_tests(&self) -> bool {
        #[cfg(test)]
        return self.fail_sends.swap(false, Ordering::SeqCst);
        #[cfg(not(test))]
        false
    }

    fn mark_told(&self, terminal: Uuid) {
        self.told.lock().unwrap_or_else(|e| e.into_inner()).insert(terminal, now_millis());
    }

    /// Mark `wake` done, with the note that says how, and announce the task
    /// so its feed shows the note.
    fn settle(&self, wake: &PendingWake, task: Option<&Task>, record: Option<String>) -> Pass {
        self.wake_holds.lock().unwrap_or_else(|e| e.into_inner()).remove(&wake.note);
        match self.service.store.finish_wake(wake, record.as_deref()) {
            Ok(Some(Some(_))) => {
                if let Some(task) = task {
                    self.announce_task_changed(task, None, Actor::Runner);
                }
                Pass::Settled
            }
            Ok(_) => Pass::Settled,
            Err(e) => {
                tracing::warn!(note = %wake.note, error = %e, "couldn't record an answer as settled");
                Pass::Waiting(Held::Busy)
            }
        }
    }

    /// The subagents to name in an answer to the orchestrator: those open on
    /// the task, when it's the orchestrator that's told. A pane told about
    /// its own task has no subagent to pass anything to.
    fn subagents_to_pass_on(&self, task: &Task, to: &Terminal) -> Vec<Subagent> {
        if to.role != TerminalRole::Orchestrator {
            return Vec::new();
        }
        let workers = self.service.store.workers_for(task.id).unwrap_or_default();
        workers.into_iter().filter(|w| w.ended_at.is_none()).take(3).map(|w| (w.agent_id, w.label)).collect()
    }

    /// Whom to tell about an answer on `task`. See this module's docs.
    async fn recipient(&self, task: &Task) -> Option<Terminal> {
        let store = &self.service.store;
        // A pane adopted as the workspace's orchestrator is one whatever it
        // was launched as: the Mac makes one of a shell someone ran claude
        // in by hand. Only that role; check 3 still proves the agent.
        let eligible = |t: &Terminal, role: TerminalRole| {
            t.workspace_id == Some(task.workspace_id)
                && t.role == role
                && t.pane_mode != PaneMode::Changes
                && (t.pane_mode == PaneMode::Agent || may_be_typed_to(&t.command_preset, t.role))
                && self.service.is_running(t)
        };
        let mut mine = store.terminals_for_task(task.id).ok()?;
        mine.reverse();
        if let Some(t) = mine.into_iter().find(|t| eligible(t, TerminalRole::Agent)) {
            return Some(t);
        }
        // The main checkout has no lane fallback: an agent opened by hand
        // there is nobody's (`task_link::task_of`, ov-112, ov-240).
        let lane = task.worktree_id.filter(|&w| store.get_worktree(w).is_ok_and(|w| !w.is_main_checkout));
        if let Some(lane) = lane {
            let lane: Vec<Terminal> = store
                .list_terminals_for_worktree(lane)
                .ok()?
                .into_iter()
                .filter(|t| t.task_id.is_none_or(|of| of == task.id) && eligible(t, TerminalRole::Agent))
                .collect();
            if let [only] = lane.as_slice() {
                return Some(only.clone());
            }
        }
        self.service
            .live_orchestrator(task.workspace_id)
            .ok()
            .flatten()
            .filter(|t| eligible(t, TerminalRole::Orchestrator))
    }

    /// Whether someone typed into `terminal` too recently to type over.
    fn typed_lately(&self, terminal: Uuid, now: i64) -> bool {
        let watched = {
            let watched = self.watched.lock().unwrap_or_else(|e| e.into_inner());
            anyone_watching(&watched, terminal, now)
        };
        let quiet = if watched { QUIET_WATCHED_MS } else { QUIET_MS };
        last_input(self.service.root_dir(), terminal).is_some_and(|at| now - at < quiet)
    }
}

/// Whether a preset launches an agent this module can type to.
pub(crate) fn is_an_agent_preset(preset: &str) -> bool {
    matches!(preset.split(':').next(), Some("claude" | "codex" | "cursor"))
}

/// Whether a terminal launched as `preset` in `role` may be typed an answer
/// in its TUI: launched as an agent, or adopted as the orchestrator
/// whatever it was launched as (`recipient`). What the daemon follows for
/// bracketed paste below tmux 3.7 (`Service::follow_paste_mode`): a pane
/// left out of that is never proven safe to paste into there.
pub(crate) fn may_be_typed_to(preset: &str, role: TerminalRole) -> bool {
    is_an_agent_preset(preset) || role == TerminalRole::Orchestrator
}

/// Which agent a program is, by its executable, or `None`.
///
/// claude renames its process to its version, so tmux reports `2.1.237`; its
/// executable is still `…/claude/versions/2.1.237`. codex's binary is
/// `codex`, or `codex-<target>` as tmux truncates it.
pub(crate) fn agent_of_executable(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next()?;
    if name == "claude" || path.contains("/claude/versions/") {
        return Some("claude");
    }
    if name == "codex" || name.starts_with("codex-") {
        return Some("codex");
    }
    if name == "cursor-agent" {
        return Some("cursor");
    }
    None
}

/// Which agent a process is, by its executable or, when that's Node, by the
/// script Node runs: the first argument that isn't a flag, `…/bin/claude`,
/// `…/@anthropic-ai/claude-code/cli.js`, `…/bin/codex` or
/// `…/@openai/codex/…`. Anything else is `None`.
pub(crate) fn agent_of_process(exe: &str, args: &str) -> Option<&'static str> {
    if let Some(agent) = agent_of_executable(exe) {
        return Some(agent);
    }
    let program = exe.rsplit('/').next().unwrap_or(exe);
    if program != "node" {
        return None;
    }
    let script = args.split_whitespace().skip(1).find(|a| !a.starts_with('-'))?;
    if script.contains("/@anthropic-ai/claude-code/") {
        return Some("claude");
    }
    if script.contains("/@openai/codex/") {
        return Some("codex");
    }
    match script.rsplit('/').next()? {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

/// The agent the foreground process of `tty` (`/dev/ttys012`) is, proven by
/// its executable and arguments (`agent_of_process`). `None` for a shell, for
/// anything else, and when it can't be read.
///
/// The foreground process group can hold the shell that launched the agent
/// (fish runs `-c` without job control, so the agent joins its group) and
/// the agent's own children. What's in front is the one process in that
/// group whose parent is outside it or is a shell, not counting a shell
/// that is only waiting on a child in it. A background job isn't in the
/// foreground group, so a shell claude backgrounded doesn't count.
pub(crate) async fn foreground_agent(tty: &str) -> Option<&'static str> {
    foreground(tty).await.map(|(agent, _)| agent)
}

/// `foreground_agent`, with the agent's pid.
pub(crate) async fn foreground(tty: &str) -> Option<(&'static str, i32)> {
    let name = tty.strip_prefix("/dev/").unwrap_or(tty);
    let out = tokio::process::Command::new("ps")
        .args(["-t", name, "-o", "pid=,ppid=,stat=,comm="])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    let (pid, comm) = in_front(&String::from_utf8_lossy(&out.stdout))?;
    // Linux's `comm` is a fifteen-byte name; the executable is the link.
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(comm);
    let args = tokio::process::Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    agent_of_process(&exe, String::from_utf8_lossy(&args.stdout).trim()).map(|agent| (agent, pid))
}

/// The process in front, from `ps -o pid=,ppid=,stat=,comm=` for one tty,
/// or `None` unless exactly one answers. See `foreground_agent`.
pub(crate) fn in_front(listing: &str) -> Option<(i32, String)> {
    let rows: Vec<(i32, i32, String)> = listing
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (pid, ppid, stat) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?);
            stat.contains('+').then(|| (pid, ppid, f.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    let shell = |comm: &str| {
        let name = comm.rsplit('/').next().unwrap_or(comm).trim_start_matches('-');
        matches!(name, "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "tcsh" | "csh" | "nu" | "elvish" | "xonsh")
    };
    let in_group = |pid: i32| rows.iter().find(|r| r.0 == pid);
    let mut front = rows.iter().filter(|(pid, ppid, comm)| {
        let parent_ok = in_group(*ppid).is_none_or(|p| shell(&p.2));
        let only_waiting = shell(comm) && rows.iter().any(|r| r.1 == *pid);
        parent_ok && !only_waiting
    });
    let (pid, _, comm) = front.next()?;
    if front.next().is_some() {
        return None;
    }
    Some((*pid, comm.clone()))
}

/// "Told the orchestrator about the decision", or that the hold ended; and
/// when it was working, that the message waits in its queue.
fn told(kind: WakeKind, to: &Terminal, turn: Turn) -> String {
    let told = match kind {
        WakeKind::Answer => format!("Told {} about the decision", spoken_name(to)),
        WakeKind::HoldEnded => format!("Told {} the hold ended", spoken_name(to)),
    };
    match turn {
        Turn::Between => told,
        Turn::During => format!("{told}. It was working, so it's queued for when it's ready"),
    }
}

fn couldnt_confirm(kind: WakeKind) -> String {
    match kind {
        WakeKind::Answer => COULDNT_CONFIRM.into(),
        WakeKind::HoldEnded => "Couldn't confirm the agent got the news that the hold ended".into(),
    }
}

fn nobody(kind: WakeKind) -> String {
    match kind {
        WakeKind::Answer => NOBODY.into(),
        WakeKind::HoldEnded => "Nobody to tell the hold ended".into(),
    }
}

/// How the record names a terminal: the orchestrator as such, an agent by
/// its title.
fn spoken_name(t: &Terminal) -> String {
    if t.role == TerminalRole::Orchestrator {
        return "the orchestrator".into();
    }
    let title = one_line(&t.title, LONGEST_TITLE);
    if title.is_empty() { "the agent".into() } else { title }
}

mod mid_turn;
mod tell;
#[cfg(test)]
mod tests;

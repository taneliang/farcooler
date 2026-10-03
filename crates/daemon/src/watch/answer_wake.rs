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
//! only agent pane in the task's worktree, else the workspace's
//! orchestrator. Never another workspace's terminal or a Changes pane. An
//! agent pane counts only if launched as an agent; a pane ADOPTED as the
//! orchestrator counts whatever it was launched as (a shell someone runs
//! claude in by hand), and check 3 proves what it runs. A shell pane not
//! adopted never counts, whatever runs in it. Nobody: "Nobody to tell about
//! the decision".
//!
//! **The gate, for a TUI pane.** Every check runs on this pass, against the
//! pane as it is now, and every one fails closed:
//! 1. The watcher reads the terminal Idle or Done, and this runner hasn't
//!    told it anything in the last `TOLD_SPACING_MS` (`told`): one answer
//!    at a time per terminal. Nothing waits on catching a transition; each
//!    tick looks at the pane as it is, and check 4 is what proves it idle.
//! 2. Nobody has typed there lately (`typed_lately`).
//! 3. The pane's foreground process is the agent its preset names (for an
//!    adopted orchestrator launched as a shell, any agent), proven by its
//!    executable, or for a Node install by the script Node runs
//!    (`foreground_agent`), never by screen text. Checks 4 and 5 read the
//!    pane as that proven agent. A shell, or anything not recognized, isn't
//!    typed into.
//! 4. A fresh capture classifies as neither Working nor Blocked, AND
//!    `composer::read` positively recognizes the agent's box and finds it
//!    empty. A menu, a picker, a prompt, an unfamiliar screen or a draft:
//!    not typed into.
//! 5. The agent has bracketed paste on, as tmux reports it. tmux older than
//!    3.7 (Ubuntu 24.04 has 3.4) can't report it, so there it's read from
//!    the pane's own output (`paste_mode`), which the daemon follows for
//!    every agent pane from when it starts the agent. Known only if the
//!    daemon has seen every byte since the agent last set or reset it, and
//!    the pane still runs that agent. Otherwise the answer waits, as for
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
//! **A chat pane** takes the answer as a prompt on its agent channel, after
//! check 1: the channel can't reach a shell, a menu or a draft.
//!
//! **Superseded.** A newer answer on the same task replaces an older one not
//! yet told: only the newest is told, and the older is noted "Not
//! delivered: a newer answer replaced it."
//!
//! **Waiting, said.** The first pass an answer waits on, its task says so
//! once: "Waiting to tell the orchestrator about the decision: it's busy."
//!
//! **Bounded.** An answer not told within `GIVE_UP_AFTER_MS` is noted "Not
//! delivered: <why it last waited>" and dropped.

use std::sync::atomic::Ordering;
use std::time::Duration;

use farcooler_agent::link::DaemonMessage;
use farcooler_core::composer::{self, Composer};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::PendingWake;
use farcooler_store::models::{Actor, NoteKind, PaneMode, Task, TaskNote, Terminal, TerminalRole};
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
pub(crate) fn message(key: &str, title: &str, answer: &str) -> String {
    let full = one_line(answer, usize::MAX);
    let cut = one_line(answer, LONGEST_ANSWER);
    let title = one_line(title, LONGEST_TITLE);
    let stop = if cut.ends_with(['.', '!', '?', '…']) { "" } else { "." };
    let more = if cut != full { format!(" (Full answer: farcooler task show {key}.)") } else { String::new() };
    // Curly quotes: in fish, `("…")` left at a prompt is a command
    // substitution, and these are no quote to any shell.
    format!("Decision on {key} (“{title}”): {cut}{stop}{more} Continue.")
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
            Held::Unproven => "Far Cooler can't tell yet whether it takes a paste",
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
                "Far Cooler couldn't tell whether the agent takes a paste. It can with tmux 3.7 or later, \
                 or for an agent started after Far Cooler"
            }
        }
    }
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
        let pending = match self.service.store.pending_answer_wakes() {
            Ok(pending) => pending,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read the answers waiting to be told");
                self.wakes_hint.store(true, Ordering::SeqCst);
                return;
            }
        };
        for (n, wake) in pending.iter().enumerate() {
            // A newer answer on the same task is waiting too: tell only it.
            if wake.claimed_at.is_none() && pending[n + 1..].iter().any(|later| later.task == wake.task) {
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
            return self.settle(wake, None, Some(COULDNT_CONFIRM.into()));
        }
        let store = &self.service.store;
        let task = match store.get_task(wake.task) {
            Ok(task) => task,
            Err(DomainError::NotFound) => return self.settle(wake, None, None),
            Err(_) => return Pass::Waiting(Held::Busy),
        };
        // Turned off since it was queued: let it go, saying nothing.
        if !store.get_workspace(task.workspace_id).is_ok_and(|w| w.wake_on_answer) {
            return self.settle(wake, None, None);
        }
        if now_millis() - wake.enqueued_at > GIVE_UP_AFTER_MS {
            let held = self.wake_holds.lock().unwrap_or_else(|e| e.into_inner()).remove(&wake.note);
            let why = held.unwrap_or(Held::Busy).why();
            return self.settle(wake, Some(&task), Some(format!("Not delivered: {why}.")));
        }
        let Some(to) = self.recipient(&task).await else {
            return self.settle(wake, Some(&task), Some(NOBODY.into()));
        };
        let pass = match self.ready(&to).await {
            Err(held) => Pass::Waiting(held),
            Ok(()) => {
                let text = message(&task.key, &task.title, &wake.body);
                if to.pane_mode == PaneMode::Agent {
                    self.prompt(wake, &task, &to, &text).await
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
        let body = format!("{WAITING} {} about the decision: {}.", spoken_name(to), held.now());
        match store.add_note(task.id, NoteKind::Progress, Actor::Runner, &body, serde_json::json!({})) {
            Ok(_) => self.announce_task_changed(task, None, Actor::Runner),
            Err(e) => tracing::warn!(note = %wake.note, error = %e, "couldn't say an answer is waiting"),
        }
    }

    /// Check 1 and 2 of the gate: the watcher reads it Idle or Done, newly
    /// since the last answer it was told, and nobody is typing.
    async fn ready(&self, to: &Terminal) -> std::result::Result<(), Held> {
        let (activity, _, _) = self.activity(to.id).await;
        match activity {
            AgentActivity::Idle | AgentActivity::Done => {}
            AgentActivity::Blocked => return Err(Held::Prompt),
            _ => return Err(Held::Busy),
        }
        let told = self.told.lock().unwrap_or_else(|e| e.into_inner()).get(&to.id).copied();
        if told.is_some_and(|told| now_millis() - told < TOLD_SPACING_MS) {
            return Err(Held::Busy);
        }
        if self.typed_lately(to.id, now_millis()) {
            return Err(Held::Typing);
        }
        Ok(())
    }

    /// A chat pane: the answer as a prompt on its agent channel.
    async fn prompt(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Pass {
        if !matches!(self.service.store.claim_answer_wake(wake.note), Ok(true)) {
            return Pass::Settled;
        }
        let prompt = DaemonMessage::Prompt { text: text.to_string(), images: Vec::new() };
        if !self.service.agents().send(to.id, prompt) {
            return self.settle(wake, Some(task), Some(COULDNT_CONFIRM.into()));
        }
        self.mark_told(to.id);
        self.settle(wake, Some(task), Some(format!("Told {} about the decision", spoken_name(to))))
    }

    /// A TUI pane: checks 3 to 5, then claim, paste, read back, Enter.
    async fn type_into(&self, wake: &PendingWake, task: &Task, to: &Terminal, text: &str) -> Pass {
        let launched = to.command_preset.split(':').next().unwrap_or_default();
        let tty = self
            .service
            .inventory_snapshot()
            .claimants(to.id)
            .into_iter()
            .find(|p| p.proves_life())
            .map(|p| p.tty.clone());
        let Some(tty) = tty else { return Pass::Waiting(Held::NotAnAgent) };
        // The agent in front, proven by its process. A pane launched as an
        // agent must be running that one. An adopted orchestrator launched
        // as anything else is read as the agent its process proves.
        let Some(preset) = foreground_agent(&tty).await else { return Pass::Waiting(Held::NotAnAgent) };
        if launched != preset && (is_an_agent_preset(launched) || to.role != TerminalRole::Orchestrator) {
            return Pass::Waiting(Held::NotAnAgent);
        }
        let Ok(composer) = self.box_of(to, preset).await else { return Pass::Waiting(Held::Unfamiliar) };
        match composer {
            Ok(Composer::Empty) => {}
            Ok(Composer::Holds(_)) => return Pass::Waiting(Held::Draft),
            Ok(Composer::Unrecognized) => return Pass::Waiting(Held::Unfamiliar),
            Err(held) => return Pass::Waiting(held),
        }
        let bracketed = match self.service.pane_bracketed_paste(to.id).await {
            Ok(Some(on)) => on,
            // tmux can't say (older than 3.7): the pane's own output can, if
            // this daemon has followed it since the agent last set the mode.
            // Not known yet: it may be once the agent sets it again (a
            // respawn, a stream followed afresh), so the answer waits.
            Ok(None) => match self.service.streamed_bracketed_paste(to.id).await {
                Some(on) => on,
                None => return Pass::Waiting(Held::Unproven),
            },
            Err(_) => false,
        };
        if !bracketed {
            return Pass::Waiting(Held::Unfamiliar);
        }
        match self.service.store.claim_answer_wake(wake.note) {
            Ok(true) => {}
            Ok(false) => return Pass::Settled,
            Err(_) => return Pass::Waiting(Held::Busy),
        }
        // From here nothing is retried: whatever happens is settled.
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let paste: String = crate::pastes::encode_paste(true, text).iter().map(|b| format!("{b:02x}")).collect();
        let started = now_millis();
        if self.fail_sends_for_tests() || runtime.send_bytes_hex(to.id, &paste).await.is_err() {
            return self.settle(wake, Some(task), Some(COULDNT_CONFIRM.into()));
        }
        let deadline = tokio::time::Instant::now() + PASTE_SETTLES;
        let mut held_exactly = false;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(PASTE_POLL).await;
            if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
                break;
            }
            if let Ok(Ok(now)) = self.box_of(to, preset).await
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
            if foreground_agent(&tty).await != Some(preset) {
                return self.settle(wake, Some(task), Some(LEFT_AT_A_SHELL.into()));
            }
            return self.settle(wake, Some(task), Some(PASTE_LEFT.into()));
        }
        if runtime.send_bytes_hex(to.id, "0d").await.is_err() {
            return self.settle(wake, Some(task), Some(COULDNT_CONFIRM.into()));
        }
        self.mark_told(to.id);
        self.settle(wake, Some(task), Some(format!("Told {} about the decision", spoken_name(to))))
    }

    /// The pane's box as a fresh capture shows it, or why it's no box to
    /// type into: the agent working, or a prompt up. `Err` when the screen
    /// couldn't be read.
    async fn box_of(&self, to: &Terminal, preset: &str) -> Result<std::result::Result<Composer, Held>> {
        let (screen, _, _) = self.service.screen(to.id).await?;
        let activity = self.service.registry().classify(preset, &screen);
        Ok(match activity {
            AgentActivity::Working => Err(Held::Busy),
            AgentActivity::Blocked => Err(Held::Prompt),
            AgentActivity::Idle => Ok(composer::read(preset, &screen)),
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
        match self.service.store.finish_answer_wake(wake.note, record.as_deref()) {
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
                && (t.pane_mode == PaneMode::Agent
                    || is_an_agent_preset(&t.command_preset)
                    || t.role == TerminalRole::Orchestrator)
                && self.service.is_running(t)
        };
        let mut mine = store.terminals_for_task(task.id).ok()?;
        mine.reverse();
        if let Some(t) = mine.into_iter().find(|t| eligible(t, TerminalRole::Agent)) {
            return Some(t);
        }
        if let Some(lane) = task.worktree_id {
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
    agent_of_process(&exe, String::from_utf8_lossy(&args.stdout).trim())
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

/// How the record names a terminal: the orchestrator as such, an agent by
/// its title.
fn spoken_name(t: &Terminal) -> String {
    if t.role == TerminalRole::Orchestrator {
        return "the orchestrator".into();
    }
    let title = one_line(&t.title, LONGEST_TITLE);
    if title.is_empty() { "the agent".into() } else { title }
}

#[cfg(test)]
mod tests;

//! A dispatched agent is seen to start (ov-493).
//!
//! Three lanes once sat for two hours on claude's "trust this folder?"
//! prompt in their new worktrees, and nothing said so. A worktree Far Cooler
//! makes lives under its own runtime directory, which no person opened
//! claude in, so a lane's agent can open on that prompt, and on any other
//! prompt or failure that comes before its first turn.
//!
//! **Watched.** Every agent pane opened in the last `START_WINDOW_MS`, as the
//! watcher first sees it (a terminal's id is a v7 uuid, stamped as it's
//! made), until it begins its first turn: Working, or a turn in its
//! session log. A pane inherited at a restart is old, and isn't watched.
//!
//! **Reported** at once when it stops on a prompt or exits, and when nothing
//! has happened by the end of the window: to the orchestrator through the
//! runner's own notice (ov-455), and in Needs You, with the reason
//! (`Observation::not_started`). Each reason once; a new one is told again,
//! and a cleared one starts the window over. A reason is read off this
//! tick's own sample, not the folded activity: the fold holds Blocked for a
//! tick after a prompt is answered, which would report a prompt that's gone.
//! A notice is never dropped behind one of the runner's still waiting for
//! the orchestrator: the two are told as one (`Store::send_runner_notice`).
//!
//! **The trust prompt** is an ask in Needs You with one answer, Trust
//! Folder (`needs_you::trust_ask_id`), which `terminal.agent_answer` brings
//! here (`answer_trust`): one Enter, only for the prompt the item was made
//! for, only while a fresh capture shows that prompt by its whole shape with
//! Yes selected (`activity::trust::read`), and only from a person's client:
//! the CLI says who calls (`AgentAnswer.actor`), and anything but `user` (an
//! agent, the orchestrator, a shell in a runner's pane) is refused. An empty
//! one is an app from before the field, whose caller is a person. That's a
//! guardrail, not a boundary: a local caller says who it is, and an agent
//! could still press Enter itself with `terminal send`. The runner never
//! answers the prompt on its own.

use std::time::{Duration, Instant};

use farcooler_core::activity::trust;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::TerminalRole;
use uuid::Uuid;

use super::Watcher;
use crate::needs_you::{NotStarted, TRUST_OPTION, trust_ask_id};
use crate::runtime::Runtime;

/// How long an agent pane has to begin its first turn before it's reported.
/// A claude given its opening prompt is working within seconds.
pub(crate) const START_WINDOW_MS: i64 = 60_000;

/// The least time between two answers pressed in one pane.
const LOCKOUT: Duration = Duration::from_millis(1_500);

/// How long the trust prompt has to leave the screen after the Enter.
const CONFIRM_WITHIN: Duration = Duration::from_secs(3);

/// Where a pane is on the way to its first turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum Start {
    /// Not watched: older than the window when first seen, or no agent.
    #[default]
    Unwatched,
    /// Watched since `since`, with nothing to report yet.
    Waiting { since: i64 },
    /// Reported for `why`; watched still, for its first turn.
    Reported { since: i64, why: NotStarted },
    /// Its first turn began.
    Started,
}

/// What one tick saw of a pane, for `Start::step`.
pub(super) struct Tick<'a> {
    /// The folded activity, after this tick: what says a turn began.
    pub activity: AgentActivity,
    /// This tick's own reading, before the fold: what says a prompt is up.
    pub sample: AgentActivity,
    /// The agent's session log has a turn.
    pub turn_logged: bool,
    /// Its pane exited, failed or was lost.
    pub ended: bool,
    /// The screen shows claude's trust prompt.
    pub trust: bool,
    pub question: Option<&'a str>,
    pub now: i64,
}

/// What `Start::step` asks of the watcher.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    Quiet,
    /// Tell the orchestrator and Needs You why it hasn't started.
    Report(NotStarted),
    /// A reason went away: Needs You moved.
    Cleared,
}

/// When `id` was made, in Unix milliseconds, for a v7 id.
fn made_at(id: Uuid) -> Option<i64> {
    let (secs, nanos) = id.get_timestamp()?.to_unix();
    Some(secs as i64 * 1_000 + i64::from(nanos) / 1_000_000)
}

impl Start {
    /// A pane seen for the first time, at `now`: watched when it was made
    /// within the window.
    pub(super) fn first_seen(id: Uuid, now: i64) -> Start {
        match made_at(id) {
            Some(at) if now - at < START_WINDOW_MS => Start::Waiting { since: at },
            _ => Start::Unwatched,
        }
    }

    /// Whether it's still waiting for its first turn.
    pub(super) fn watching(&self) -> bool {
        matches!(self, Start::Waiting { .. } | Start::Reported { .. })
    }

    /// Why it hasn't started, once that's been said.
    pub(super) fn not_started(&self) -> Option<NotStarted> {
        match self {
            Start::Reported { why, .. } => Some(why.clone()),
            _ => None,
        }
    }

    /// Fold one tick in.
    pub(super) fn step(&mut self, t: Tick<'_>) -> Step {
        let since = match self {
            Start::Waiting { since } | Start::Reported { since, .. } => *since,
            Start::Unwatched | Start::Started => return Step::Quiet,
        };
        if t.activity == AgentActivity::Working || t.turn_logged {
            let was = matches!(self, Start::Reported { .. });
            *self = Start::Started;
            return if was { Step::Cleared } else { Step::Quiet };
        }
        let why = if t.ended {
            Some(NotStarted::Exited)
        } else if t.trust && t.sample == AgentActivity::Blocked {
            Some(NotStarted::TrustPrompt)
        } else if t.sample == AgentActivity::Blocked {
            Some(NotStarted::Prompt(t.question.map(str::to_string)))
        } else if matches!(self, Start::Reported { why: NotStarted::TrustPrompt | NotStarted::Prompt(_), .. }) {
            // A prompt answered: the window starts over from here.
            None
        } else if t.now - since >= START_WINDOW_MS {
            Some(NotStarted::NoTurn)
        } else {
            None
        };
        match (&*self, why) {
            (Start::Reported { why: old, .. }, Some(why)) if std::mem::discriminant(old) == std::mem::discriminant(&why) => {
                Step::Quiet
            }
            (_, Some(why)) => {
                *self = Start::Reported { since, why: why.clone() };
                Step::Report(why)
            }
            // A prompt answered: the window starts over from here.
            (Start::Reported { .. }, None) => {
                *self = Start::Waiting { since: t.now };
                Step::Cleared
            }
            (_, None) => Step::Quiet,
        }
    }
}

/// The last eight of a terminal's id, as the CLI shows it.
fn short(id: Uuid) -> String {
    let s = id.simple().to_string();
    s[s.len() - 8..].to_string()
}

/// The runner's notice for an agent that hasn't started.
pub(super) fn notice(who: &str, terminal: Uuid, why: &NotStarted) -> String {
    let screen = format!("`farcooler terminal screen {}`", short(terminal));
    match why {
        NotStarted::TrustPrompt => format!(
            "{who} hasn't started: Claude Code is asking whether to trust its folder. That's the owner's \
             call, never an agent's: tell them it's waiting in Needs You."
        ),
        NotStarted::Prompt(Some(q)) => {
            format!("{who} hasn't started: it's stopped on a prompt, “{}”. See {screen}.", super::answer_wake::one_line(q, 120))
        }
        NotStarted::Prompt(None) => format!("{who} hasn't started: it's stopped on a prompt. See {screen}."),
        NotStarted::NoTurn => format!(
            "{who} hasn't begun its first turn in the {} seconds since it opened. See {screen}.",
            START_WINDOW_MS / 1_000
        ),
        NotStarted::Exited => format!("{who} exited before its first turn. See {screen} for why."),
    }
}

impl Watcher {
    /// How a pane seen for the first time at `now` starts: watched when it
    /// is an agent's, opened for a task within the window.
    pub(super) fn start_for(&self, terminal: Uuid, now: i64) -> Start {
        let store = &self.service.store;
        let dispatched = store.get_terminal(terminal).is_ok_and(|t| t.role == TerminalRole::Agent && t.task_id.is_some());
        if dispatched { Start::first_seen(terminal, now) } else { Start::Unwatched }
    }

    /// `terminal`'s agent hasn't started, for `why`: Needs You moved, and
    /// its orchestrator is told, in one notice with any of the runner's
    /// still waiting for it.
    pub(super) fn not_started(&self, terminal: Uuid, why: &NotStarted) {
        self.announce_needs_you();
        let store = &self.service.store;
        let Some(task) = store.get_terminal(terminal).ok().and_then(|row| crate::task_link::notice_task(store, &row)) else {
            return;
        };
        if !self.may_tell_orchestrator(terminal, &task) {
            return;
        }
        let body = notice(&self.lane_agent(terminal, &task), terminal, why);
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": terminal.to_string() } });
        match store.send_runner_notice(task.id, &body, extra) {
            Ok(_) => {
                self.announce_task_changed(&task, None, farcooler_store::models::Actor::Runner);
                self.message_queued();
            }
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => tracing::warn!(%terminal, error = %e, "couldn't tell the orchestrator a lane hasn't started"),
        }
    }

    /// `terminal.agent_answer` for the trust prompt (see this module's docs):
    /// `actor` must be a person's (`user`, or empty from an older app), `option` Trust, `ask` the prompt up now,
    /// and a fresh capture must show it with Yes selected. Then one Enter,
    /// answered once the prompt has left the screen, or `unconfirmed`: sent,
    /// but not yet seen to take.
    pub(crate) async fn answer_trust(&self, terminal: Uuid, ask: &str, option: &str, actor: &str) -> Result<()> {
        // Empty: an app from before `actor`, whose caller is a person.
        if !actor.is_empty() && actor != "user" {
            return Err(DomainError::Conflict { what: "not_a_person" });
        }
        if option != TRUST_OPTION {
            return Err(DomainError::InvalidArgument { what: "option_id" });
        }
        let up = self
            .state
            .lock()
            .await
            .get(&terminal)
            .filter(|o| o.trust_prompt && o.activity == AgentActivity::Blocked)
            .map(|o| trust_ask_id(terminal, o.state_since));
        if up.as_deref() != Some(ask) {
            return Err(DomainError::Conflict { what: "not_held" });
        }
        {
            let mut pressed = self.keys_pressed.lock().unwrap_or_else(|e| e.into_inner());
            if pressed.get(&terminal).is_some_and(|at| at.elapsed() < LOCKOUT) {
                return Err(DomainError::Conflict { what: "too_soon" });
            }
            pressed.insert(terminal, Instant::now());
        }
        let (screen, _, _) = self.service.screen(terminal).await?;
        match trust::read(&screen) {
            Some(prompt) if prompt.yes_selected => {}
            Some(_) => return Err(DomainError::Conflict { what: "not_on_yes" }),
            None => return Err(DomainError::Conflict { what: "not_held" }),
        }
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        runtime.send_bytes_hex(terminal, "0d").await?;
        let deadline = Instant::now() + CONFIRM_WITHIN;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.service.screen(terminal).await.is_ok_and(|(screen, _, _)| !trust::shown(&screen)) {
                return Ok(());
            }
        }
        Err(DomainError::Conflict { what: "unconfirmed" })
    }
}

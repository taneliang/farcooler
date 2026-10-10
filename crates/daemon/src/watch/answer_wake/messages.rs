//! Messages between a workspace's orchestrator and its lanes (ov-455).
//!
//! Delegated work must never stall: an agent that finishes, gets stuck or has
//! a question tells its orchestrator, and the orchestrator can steer any of
//! its lanes, across Claude Code, Codex, Cursor and anything else, with no
//! person in between. A message is the answer wake's third kind
//! (`WakeKind::Message`): filed as a note on a card and queued in one write
//! (`Store::send_message`), then typed into the recipient's box by the same
//! pump, under the same gate and the same rules, as an answer. So it reaches
//! a claude pane idle or working (claude queues it), a codex pane between
//! turns, and any chat pane; it is told once, in order, and survives a
//! restart. What is typed is one line, attributed: `[from mac-ux] PR is up`.
//!
//! **Addressing** (`message_send`). Hub and spoke (R-46): an agent messages
//! its orchestrator and nobody else; the orchestrator, or the owner, messages
//! a lane by its name or by any card on it, or a card on no lane through the
//! card's own agent. One runner (R-45): the board, its lanes and its
//! orchestrator are all this runner's. A message to the orchestrator names no
//! terminal, and the workspace's live orchestrator is found when it's told,
//! so one restarted meanwhile still gets it.
//!
//! **The runner's own notice** (`lane_stopped`). When a lane's agent ends a
//! turn without having reported (no note, move or message of its own since
//! the turn began), its turn fails, or it stops on a question, the runner
//! messages the orchestrator for it: `[Far Cooler] The lane mac-ux's agent
//! ended its turn without reporting. It last said: "…"`. Only an agent's
//! transitions do this, never the orchestrator's (`task_link::task_of`), so a
//! notice never causes another; and one waits untold at most once per card.
//!
//! **Safety.** Nobody messages themselves; a message is one line of at most
//! `LONGEST_TEXT` characters; at most `MOST_WAITING` wait for one recipient,
//! and one sender sends one recipient at most `MOST_PER_HOUR` an hour, so two
//! agents can't flood each other into a loop. The workspace's wake-on-answer
//! switch covers messages: it is what lets the runner type into a pane.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, AgentActivity, Request, request, result};
use farcooler_store::models::{Actor, PaneMode, Task, TaskStatus, Terminal, TerminalRole};
use farcooler_store::{PendingWake, WakeKind};
use uuid::Uuid;

use super::{Held, Pass, may_be_typed_to, nobody, one_line};
use crate::watch::{Watcher, now_millis};
use crate::watch::task_notice::AgentNews;

/// How long a message may wait to be told: long enough to outwait a long
/// turn in an agent that takes nothing mid-turn.
pub(crate) const GIVE_UP_AFTER_MS: i64 = 4 * 60 * 60 * 1_000;
/// The longest message, on one line. With its tag, under the 500 characters
/// a box can be read back from (`tell::LONGEST_MESSAGE`).
pub(crate) const LONGEST_TEXT: usize = 400;
/// The most messages that may wait for one recipient.
pub(crate) const MOST_WAITING: u32 = 20;
/// The most messages one sender may send one recipient in an hour.
pub(crate) const MOST_PER_HOUR: u32 = 30;
/// How far back a turn with no known start is read for a report.
const TURN_FALLBACK_MS: i64 = 30 * 60 * 1_000;
/// The widest an agent's last words or question are quoted in a notice.
const QUOTED: usize = 160;

/// Whom a message is for, once resolved: a pane, or the orchestrator.
struct Recipient {
    terminal: Option<Uuid>,
    /// As a person reads it: "the orchestrator", "the lane mac-ux".
    said: String,
}

impl Watcher {
    /// `message.send`: file a message and queue it. See this module's docs.
    pub(crate) async fn message_send(&self, req: Request) -> Result<result::Value> {
        let Some(request::Payload::MessageSend(p)) = req.payload else {
            return Err(DomainError::InvalidArgument { what: "payload" });
        };
        let store = &self.service.store;
        let actor = crate::task_ops::actor_from_wire(&p.actor)?;
        let text = one_line(&p.text, usize::MAX);
        if text.is_empty() {
            return Err(DomainError::InvalidArgument { what: "text" });
        }
        if text.chars().count() > LONGEST_TEXT {
            return Err(DomainError::Conflict { what: "too_long" });
        }
        let sender = match actor {
            Actor::Agent { terminal } => store.get_terminal(terminal).ok(),
            _ => None,
        };
        let workspace = match (Uuid::from_slice(&p.workspace_id).ok(), sender.as_ref().and_then(|t| t.workspace_id)) {
            (Some(named), _) => named,
            (None, Some(own)) => own,
            (None, None) => return Err(DomainError::InvalidArgument { what: "workspace" }),
        };
        let board = store.get_workspace(workspace)?;
        let to = p.to.trim();
        let (task, recipient) = if to.eq_ignore_ascii_case("orchestrator") {
            if actor == Actor::Manager || sender.as_ref().is_some_and(|t| t.role == TerminalRole::Orchestrator) {
                return Err(DomainError::Conflict { what: "self" });
            }
            let task = match sender.as_ref().and_then(|t| crate::task_link::task_of(store, t)) {
                Some(task) => task,
                None => self.card_named(&board, p.task.trim())?.ok_or(DomainError::Conflict { what: "no_task" })?,
            };
            (task, Recipient { terminal: None, said: "the orchestrator".into() })
        } else {
            // Hub and spoke (R-46): an agent talks to its orchestrator only.
            if matches!(actor, Actor::Agent { .. }) {
                return Err(DomainError::Conflict { what: "hub" });
            }
            self.lane_or_card(&board, to).await?
        };
        if task.workspace_id != workspace {
            return Err(DomainError::InvalidArgument { what: "to" });
        }
        if let (Some(from), Some(to)) = (sender.as_ref(), recipient.terminal)
            && from.id == to
        {
            return Err(DomainError::Conflict { what: "self" });
        }
        let hour_ago = now_millis() - 60 * 60 * 1_000;
        if store.messages_waiting(workspace, recipient.terminal)? >= MOST_WAITING
            || store.messages_sent_since(workspace, actor, recipient.terminal, hour_ago)? >= MOST_PER_HOUR
        {
            return Err(DomainError::Conflict { what: "flood" });
        }
        let extra = serde_json::json!({ "message": { "to": recipient.said } });
        let note = store.send_message(task.id, actor, &text, recipient.terminal, extra)?;
        self.announce_task_changed(&task, None, actor);
        self.message_queued();
        Ok(result::Value::MessageSent(pb::MessageSent {
            note_id: bytes::Bytes::copy_from_slice(note.id.as_bytes()),
            task_key: task.key.clone(),
            recipient: recipient.said,
        }))
    }

    /// A message was queued: have the pump look.
    fn message_queued(&self) {
        self.wakes_hint.store(true, std::sync::atomic::Ordering::SeqCst);
        self.spawn_wake_pump();
    }

    /// The card `key` names on `board`, if a key was given.
    fn card_named(&self, board: &farcooler_store::models::Workspace, key: &str) -> Result<Option<Task>> {
        if key.is_empty() {
            return Ok(None);
        }
        let found = self.service.store.tasks_with_key(Some(board.repository_id), key)?;
        Ok(found.into_iter().find(|t| t.workspace_id == board.id))
    }

    /// `to` as a live lane's name, or as a card: its lane's pane, or for a
    /// card on no lane its own agent. Refused as `nobody` when no agent is
    /// working it, and `to` when it names nothing on the board.
    async fn lane_or_card(&self, board: &farcooler_store::models::Workspace, to: &str) -> Result<(Task, Recipient)> {
        let store = &self.service.store;
        let nobody = DomainError::Conflict { what: "nobody" };
        let (lane, card) = match store.live_lane_named(board.id, to) {
            Ok(lane) => (Some(lane), None),
            Err(_) => {
                let card = self.card_named(board, to)?.ok_or(DomainError::InvalidArgument { what: "to" })?;
                (store.live_lane_of_card(card.id)?, Some(card))
            }
        };
        if let Some(lane) = lane {
            let pane = store.lane_pane(lane.id)?.ok_or(nobody.clone())?;
            let row = store.get_terminal(pane).map_err(|_| nobody.clone())?;
            let task = match card {
                Some(card) => card,
                None => row.task_id.and_then(|t| store.get_task(t).ok()).ok_or(nobody)?,
            };
            return Ok((task, Recipient { terminal: Some(pane), said: format!("the lane {}", lane.name) }));
        }
        let card = card.ok_or(nobody.clone())?;
        let agent = self.recipient(&card).await.filter(|t| t.role != TerminalRole::Orchestrator).ok_or(nobody)?;
        let said = format!("{}'s agent", card.key);
        Ok((card, Recipient { terminal: Some(agent.id), said }))
    }

    /// Whom a queued message goes to now: its pane, or the workspace's live
    /// orchestrator. One gone for good settles it; one not running yet, or
    /// no orchestrator yet, waits.
    pub(super) async fn message_recipient(&self, wake: &PendingWake, task: &Task) -> std::result::Result<Terminal, Pass> {
        let store = &self.service.store;
        let typable = |t: &Terminal| {
            !t.pane_mode.is_client_drawn()
                && (t.pane_mode == PaneMode::Agent || may_be_typed_to(&t.command_preset, t.role))
                && self.service.is_running(t)
        };
        let found = match wake.to {
            Some(id) => match store.get_terminal(id) {
                Ok(t) => Some(t),
                Err(DomainError::NotFound) => {
                    return Err(self.settle(wake, Some(task), Some(nobody(WakeKind::Message))));
                }
                Err(_) => None,
            },
            None => self.service.live_orchestrator(task.workspace_id).ok().flatten(),
        };
        found.filter(typable).ok_or(Pass::Waiting(Held::NotAnAgent))
    }

    /// What a message is typed as: its tag, then its text, on one line.
    pub(super) fn message_text(&self, wake: &PendingWake, task: &Task) -> String {
        let tag = match wake.actor {
            Actor::Agent { terminal } => format!("[from {}]", self.lane_or_key(terminal, task)),
            Actor::Manager => "[from the orchestrator]".to_string(),
            Actor::User => "[from the owner]".to_string(),
            Actor::Runner => "[Far Cooler]".to_string(),
            Actor::Unknown => "[from an agent]".to_string(),
        };
        format!("{tag} {}", one_line(&wake.body, LONGEST_TEXT))
    }

    /// A pane's lane by name, or the card it works by key.
    fn lane_or_key(&self, terminal: Uuid, task: &Task) -> String {
        match self.service.store.lane_of_pane(terminal) {
            Ok(Some(lane)) => lane.name,
            _ => task.key.clone(),
        }
    }

    /// An agent's activity moved to `next` on a sample: when it stopped
    /// working a card (`task_link::notice_task`), `lane_stopped`. Whether a
    /// person is watching the pane doesn't matter: the orchestrator isn't.
    pub(crate) fn lane_moved(
        &self,
        terminal: Uuid,
        next: AgentActivity,
        question: Option<&str>,
        said: Option<&str>,
        failed: bool,
        started_at: Option<i64>,
    ) {
        let store = &self.service.store;
        let Some(task) = store.get_terminal(terminal).ok().and_then(|row| crate::task_link::notice_task(store, &row)) else {
            return;
        };
        let news = match next {
            AgentActivity::Blocked => {
                AgentNews::Blocked { terminal, label: String::new(), question: question.map(str::to_string) }
            }
            AgentActivity::Done if failed => AgentNews::Failed { label: String::new() },
            AgentActivity::Done => AgentNews::Finished { label: String::new(), said: said.map(str::to_string) },
            _ => return,
        };
        self.lane_stopped(terminal, &task, &news, started_at);
    }

    /// A lane's agent stopped (`lane_moved`): tell its orchestrator when it
    /// didn't report. See this module's docs.
    pub(crate) fn lane_stopped(&self, terminal: Uuid, task: &Task, news: &AgentNews, started_at: Option<i64>) {
        let store = &self.service.store;
        let Ok(row) = store.get_terminal(terminal) else { return };
        if row.role != TerminalRole::Agent || matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            return;
        }
        if !self.service.live_orchestrator(task.workspace_id).ok().flatten().is_some_and(|o| self.service.is_running(&o)) {
            return;
        }
        let me = Actor::Agent { terminal };
        let since = started_at.unwrap_or_else(|| now_millis() - TURN_FALLBACK_MS);
        // A notice already waiting says enough; an agent that wrote on its
        // card this turn has reported, unless it is stuck now.
        let waiting = store.reported_since(task.id, Actor::Runner, i64::MAX).unwrap_or(true);
        let reported = store.reported_since(task.id, me, since).unwrap_or(true);
        let who = match store.lane_of_pane(terminal) {
            Ok(Some(lane)) => format!("The lane {}'s agent, on {},", lane.name, task.key),
            _ => format!("{}'s agent", task.key),
        };
        let quoted = |text: &str| one_line(text, QUOTED);
        let body = match news {
            AgentNews::Finished { said, .. } if !waiting && !reported => match said.as_deref().map(quoted) {
                Some(said) if !said.is_empty() => format!("{who} ended its turn without reporting. It last said: “{said}”"),
                _ => format!("{who} ended its turn without reporting."),
            },
            AgentNews::Failed { .. } if !waiting => format!("{who} stopped: its turn failed."),
            AgentNews::Blocked { question, .. } if !waiting => match question.as_deref().map(quoted) {
                Some(q) if !q.is_empty() => format!("{who} is stopped on a question: “{q}”"),
                _ => format!("{who} is stopped on a question or a permission prompt."),
            },
            _ => return,
        };
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": terminal.to_string() } });
        match store.send_message(task.id, Actor::Runner, &body, None, extra) {
            Ok(_) => {
                self.announce_task_changed(task, None, Actor::Runner);
                self.message_queued();
            }
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => tracing::warn!(%terminal, error = %e, "couldn't tell the orchestrator a lane stopped"),
        }
    }
}

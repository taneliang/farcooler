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
//! turns; it is told once, in order, and survives a
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
//! **People and reps** (ov-494, `crate::people`). A member's rep is an agent
//! like a lane's: it messages its orchestrator and nobody else, about its
//! person's desk card unless it names another, tagged `[from Alice's rep]`.
//! The orchestrator, or the lead, reaches it as `rep:<handle>`. A person's own
//! message (actor `user`) comes only from the workspace's lead, except to
//! their own rep; one from a member's device is tagged with their name, and
//! never claims to be an agent or the orchestrator. A message for a rep
//! that isn't running resumes it first, on its own conversation
//! (`wake_rep`), and is then told as any other: there's no idle parking, so
//! a rep comes back on demand (R-66). Only from someone who could have
//! started it (`people::may_start_rep`); anyone else's waits.
//!
//! **Safety.** Nobody messages themselves; a message is one line of at most
//! `LONGEST_TEXT` characters; at most `MOST_WAITING` wait for one recipient,
//! and one sender sends one recipient at most `MOST_PER_HOUR` an hour, so two
//! agents can't flood each other into a loop. The workspace's wake-on-answer
//! switch covers messages: it is what lets the runner type into a pane.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, AgentActivity, Request, request, result};
use farcooler_store::models::{Actor, Task, TaskStatus, Terminal, TerminalRole};
use farcooler_store::{PendingWake, WakeKind};
use uuid::Uuid;

use super::{Held, Pass, may_be_typed_to, nobody, one_line};
use crate::people::Person;
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

/// The recipient a message waits in line for: its pane, or its workspace's
/// orchestrator. `None` for anything that isn't a message. The pump tells a
/// recipient's messages in the order sent: one still waiting holds back
/// every later one for the same recipient in that pass.
pub(super) fn in_line(store: &farcooler_store::Store, wake: &PendingWake) -> Option<(Option<Uuid>, Uuid)> {
    if wake.kind != WakeKind::Message {
        return None;
    }
    let workspace = store.get_task(wake.task).map(|t| t.workspace_id).unwrap_or_default();
    Some((wake.to, workspace))
}

/// Notes a test holds waiting, as an agent too busy to be typed to would.
#[cfg(test)]
static HELD_FOR_TESTS: std::sync::Mutex<Vec<Uuid>> = std::sync::Mutex::new(Vec::new());

/// Whether a test holds `note` waiting (`hold_for_tests`).
#[cfg(test)]
pub(super) fn held_for_tests(note: Uuid) -> bool {
    HELD_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner()).contains(&note)
}

/// Hold `note` waiting on every pass, until `release_for_tests`.
#[cfg(test)]
pub(super) fn hold_for_tests(note: Uuid) {
    HELD_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner()).push(note);
}

/// Let `note` be told again.
#[cfg(test)]
pub(super) fn release_for_tests(note: Uuid) {
    HELD_FOR_TESTS.lock().unwrap_or_else(|e| e.into_inner()).retain(|n| *n != note);
}

/// Whom a message is for, once resolved: a pane, or the orchestrator.
struct Recipient {
    terminal: Option<Uuid>,
    /// As a person reads it: "the orchestrator", "the lane mac-ux".
    said: String,
}

impl Watcher {
    /// `message.send`: file a message and queue it. See this module's docs.
    ///
    /// **Who sent it is the caller's word.** `actor` comes from the CLI, which
    /// reads it from the pane's `FARCOOLER_ACTOR`, and anything that can run
    /// a command in a pane can set that variable. So the hub rule below, and
    /// the tag a message is typed with, are a guardrail against an agent's
    /// mistake, not a security boundary: `message.send` needs Control scope,
    /// which can already type into any pane (`terminal.write`). The CLI
    /// refuses `--actor` in an agent's pane, so a mistake can't name another
    /// sender, and no message is ever tagged as the owner's (`message_text`).
    ///
    /// `person` is who the connection is (`people::person_of`), which a member's
    /// device can't choose: the lead rule and a person's tag come from it.
    ///
    /// `scope` is the connection's, for whether this message may also wake a
    /// stopped rep (`people::may_start_rep`).
    pub(crate) async fn message_send(&self, req: Request, person: Person, scope: pb::Scope) -> Result<result::Value> {
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
        // A member's device speaks as that person, never as a pane or the orchestrator.
        if person.is_someone_else() && actor != Actor::User {
            return Err(DomainError::InvalidArgument { what: "actor" });
        }
        let sender = match actor {
            Actor::Agent { terminal } => store.get_terminal(terminal).ok(),
            _ => None,
        };
        let rep = match &sender {
            Some(pane) => store.rep_of_terminal(pane.id)?,
            None => None,
        };
        let workspace = match (Uuid::from_slice(&p.workspace_id).ok(), sender.as_ref().and_then(|t| t.workspace_id)) {
            (Some(named), _) => named,
            (None, Some(own)) => own,
            (None, None) => return Err(DomainError::InvalidArgument { what: "workspace" }),
        };
        // A rep speaks only on its own board, and only to its orchestrator
        // (ov-527): the runner holds it to its gate's grammar too.
        if rep.as_ref().is_some_and(|r| r.workspace != workspace || !p.to.trim().eq_ignore_ascii_case("orchestrator")) {
            return Err(DomainError::Conflict { what: "rep_messenger" });
        }
        let board = store.get_workspace(workspace)?;
        let to = p.to.trim();
        let to_rep = to.strip_prefix("rep:").map(str::trim);
        // Only the lead, and the owner (R-54), talk to the orchestrator and its
        // lanes as a person (ov-479 D1); anyone may message their own rep.
        if actor == Actor::User {
            let own = matches!((&person, to_rep), (Person::Member(me), Some(handle)) if me == handle);
            if !own && !crate::people::talks_to_orchestrator(store, &person, workspace)? {
                return Err(DomainError::NotYours { what: "not_lead" });
            }
        }
        let (task, recipient) = if to.eq_ignore_ascii_case("orchestrator") {
            if actor == Actor::Manager || sender.as_ref().is_some_and(|t| t.role == TerminalRole::Orchestrator) {
                return Err(DomainError::Conflict { what: "self" });
            }
            let task = match (&rep, sender.as_ref().and_then(|t| crate::task_link::task_of(store, t))) {
                (Some(rep), _) => self.rep_card(&board, &rep.handle, p.task.trim())?,
                (None, Some(task)) => task,
                (None, None) => {
                    self.card_named(&board, p.task.trim())?.ok_or(DomainError::Conflict { what: "no_task" })?
                }
            };
            (task, Recipient { terminal: None, said: "the orchestrator".into() })
        } else if let Some(handle) = to_rep {
            if matches!(actor, Actor::Agent { .. }) {
                return Err(DomainError::Conflict { what: "hub" });
            }
            self.rep_named(&board, handle, p.task.trim())?
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
        let hour_ago = now_millis() - 60 * 60 * 1_000;
        if store.messages_waiting(workspace, recipient.terminal)? >= MOST_WAITING
            || store.messages_sent_since(workspace, actor, recipient.terminal, hour_ago)? >= MOST_PER_HOUR
        {
            return Err(DomainError::Conflict { what: "flood" });
        }
        // Only a sender who could have started the rep wakes it (`rep.start`'s
        // rule); anyone else's message waits for its next start.
        // A rep never wakes one, whatever its connection's scope.
        if let (Some(handle), Some(pane), None) = (to_rep, recipient.terminal, &rep)
            && crate::people::may_start_rep(&person, scope, handle)
        {
            self.wake_rep(pane).await;
        }
        let mut extra = serde_json::json!({ "message": { "to": recipient.said } });
        if let Some(from) = self.author(&person, actor, rep.as_ref()) {
            extra["message"]["from"] = serde_json::Value::String(from);
        }
        let note = store.send_message(task.id, actor, &text, recipient.terminal, extra)?;
        self.announce_task_changed(&task, None, actor);
        self.message_queued();
        Ok(result::Value::MessageSent(pb::MessageSent {
            note_id: bytes::Bytes::copy_from_slice(note.id.as_bytes()),
            task_key: task.key.clone(),
            recipient: recipient.said,
        }))
    }

    /// Resume the rep in `pane` when it isn't running, so the message about
    /// to be queued for it is told (`Service::resume_rep`). One that can't be
    /// resumed is logged, and the message waits for its next start, as it
    /// always did.
    async fn wake_rep(&self, pane: Uuid) {
        let Ok(row) = self.service.store.get_terminal(pane) else { return };
        self.service.inventory.refresh().await;
        if self.service.is_running(&row) {
            return;
        }
        if let Err(e) = self.service.resume_rep(pane).await {
            tracing::warn!(terminal = %pane, error = %e, "couldn't resume a rep for its message");
        }
    }

    /// A message was queued: have the pump look.
    pub(crate) fn message_queued(&self) {
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

    /// Who a message is from, as the runner tags it, when its own records
    /// say: a rep by its member's name, or a member's own device by theirs.
    /// `None` for everyone else, who is tagged as before (`message_text`).
    fn author(&self, person: &Person, actor: Actor, rep: Option<&farcooler_store::people::Rep>) -> Option<String> {
        let name = |handle: &str| self.service.store.member(handle).ok().map(|m| m.name);
        match (rep, person, actor) {
            (Some(rep), _, _) => name(&rep.handle).map(|n| format!("{n}'s rep")),
            (None, Person::Member(handle), Actor::User) => name(handle),
            _ => None,
        }
    }

    /// The card a message for or from `handle`'s rep is filed on: `key` when
    /// it names one on `board`, else their desk.
    fn rep_card(&self, board: &farcooler_store::models::Workspace, handle: &str, key: &str) -> Result<Task> {
        if let Some(card) = self.card_named(board, key)? {
            return Ok(card);
        }
        let desk = self.service.store.rep_desk(board.id, handle)?.ok_or(DomainError::Conflict { what: "no_task" })?;
        self.service.store.get_task(desk)
    }

    /// `handle`'s rep on `board`, for `rep:<handle>`: refused as `to` for
    /// nobody by that handle, and `no_rep` for a member with no rep there.
    fn rep_named(&self, board: &farcooler_store::models::Workspace, handle: &str, key: &str) -> Result<(Task, Recipient)> {
        let store = &self.service.store;
        let member = store.member(handle).map_err(|_| DomainError::InvalidArgument { what: "to" })?;
        let rep = store.rep_for(board.id, handle)?.ok_or(DomainError::Conflict { what: "no_rep" })?;
        let task = self.rep_card(board, handle, key)?;
        Ok((task, Recipient { terminal: Some(rep.terminal), said: format!("{}'s rep", member.name) }))
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
            // A lane's pane since made the orchestrator is the sender itself.
            if row.role == TerminalRole::Orchestrator {
                return Err(DomainError::Conflict { what: "self" });
            }
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
                && may_be_typed_to(&t.command_preset, t.role)
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
        // Who the runner said it's from, when it said (ov-494): a rep, a member.
        if let Ok(Some(from)) = self.service.store.message_from(wake.note) {
            return format!("[from {from}] {}", one_line(&wake.body, LONGEST_TEXT));
        }
        let tag = match wake.actor {
            Actor::Agent { terminal } => format!("[from {}]", self.lane_or_key(terminal, task)),
            Actor::Manager => "[from the orchestrator]".to_string(),
            // `user` is only what a pane with no agent in it says, and a
            // person's own message comes from the apps' paths, never this
            // one: so nobody is named the owner (see `message_send`).
            Actor::User => "[from the terminal]".to_string(),
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
        if !self.may_tell_orchestrator(terminal, task) {
            return;
        }
        let me = Actor::Agent { terminal };
        let since = started_at.unwrap_or_else(|| now_millis() - TURN_FALLBACK_MS);
        // A notice already waiting says enough; an agent that wrote on its
        // card this turn has reported, unless it is stuck now.
        let waiting = self.notice_waiting(task);
        let reported = store.reported_since(task.id, me, since).unwrap_or(true);
        let who = self.lane_agent(terminal, task);
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
        self.queue_notice(terminal, task, &body);
    }

    /// Whether the runner tells an orchestrator about `terminal`: an agent's
    /// pane, on an open card, in a workspace whose orchestrator is running.
    pub(crate) fn may_tell_orchestrator(&self, terminal: Uuid, task: &Task) -> bool {
        let Ok(row) = self.service.store.get_terminal(terminal) else { return false };
        row.role == TerminalRole::Agent
            && !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled)
            && self.service.live_orchestrator(task.workspace_id).ok().flatten().is_some_and(|o| self.service.is_running(&o))
    }

    /// Whether a runner notice about `task` waits untold: it says enough.
    pub(crate) fn notice_waiting(&self, task: &Task) -> bool {
        self.service.store.reported_since(task.id, Actor::Runner, i64::MAX).unwrap_or(true)
    }

    /// How a notice names `terminal`'s agent: by its lane and card, or by
    /// its card.
    pub(crate) fn lane_agent(&self, terminal: Uuid, task: &Task) -> String {
        match self.service.store.lane_of_pane(terminal) {
            Ok(Some(lane)) => format!("The lane {}'s agent, on {},", lane.name, task.key),
            _ => format!("{}'s agent", task.key),
        }
    }

    /// File the runner's `body` about `terminal` on `task`, for the
    /// orchestrator, and have the pump tell it.
    pub(crate) fn queue_notice(&self, terminal: Uuid, task: &Task, body: &str) {
        let extra = serde_json::json!({ "message": { "to": "the orchestrator", "about": terminal.to_string() } });
        match self.service.store.send_message(task.id, Actor::Runner, body, None, extra) {
            Ok(_) => {
                self.announce_task_changed(task, None, Actor::Runner);
                self.message_queued();
            }
            Err(DomainError::Conflict { what: "typing_off" }) => {}
            Err(e) => tracing::warn!(%terminal, error = %e, "couldn't tell the orchestrator about a lane"),
        }
    }
}

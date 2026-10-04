//! Notifications about tasks, not agents (ov-94).
//!
//! One thread per task on every device: a notice is titled `<key> <title>`,
//! says the task's news in its body (`Moved to In Review · 3 files changed`,
//! `Needs your decision · Which PDF library?`, `Blocked on ov-88`, `Done`),
//! and is filed under `t:<runner id>:<task key>`, so a newer notice about a
//! task replaces the older one on the Mac, on an iPhone and on Android.
//! `docs/superpowers/specs/2026-10-02-task-notifications-design.md` is the
//! design; this is its runner half.
//!
//! **Intake.** The board's writes (`task_ops`) call `task_event`, and an
//! agent working on a task (`task_link::notice_task`) calls `agent_event` from
//! its transition in place of its own alert. Each marks the task pending.
//!
//! **The window.** Per task: open on the first event, closed 3 s after the
//! last (`QUIET_FOR`), and at most 10 s after the first (`AT_MOST`), so a
//! steady trickle still sends. A decision or a blocked agent on a task with
//! nothing pending sends at once: waiting on "it's stopped for you" buys
//! nothing.
//!
//! **Compose at the close.** From the task as it is then, not the events, so
//! In Progress, In Review then Done in one burst says `Done`, and a decision
//! answered inside its window says nothing. The most urgent class still true
//! wins: decision, blocked, review, done, new (`Class`).
//!
//! **What never notifies** (Q6): your own writes (`Actor::User`) and the
//! runner's (`Actor::Runner`); moves into Backlog, Todo, In Progress or
//! Cancelled; edits; every note but a QUESTION; a cleared block, or one on a
//! task that's done; a restart, since only an intake opens a window; and a
//! status already told (`task_told`, by class and `status_since`).
//!
//! **Where it goes.** To every connected client as the `notice` event (the
//! Mac posts it when the relay won't), and to the relay as `kind: "task"`
//! when paired, whose devices each filter by the classes they kept on.

use std::collections::HashMap;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::{Actor, NoteKind, Task, TaskNote, TaskStatus};
use sha2::Digest;
use tokio::time::{Duration, Instant};
use uuid::Uuid;

use super::answer_wake::one_line;
use super::{Tapped, Watcher};

/// How long a task's window waits for quiet before it closes.
pub(crate) const QUIET_FOR: Duration = Duration::from_secs(3);
/// The longest a window stays open, however busy the task is.
pub(crate) const AT_MOST: Duration = Duration::from_secs(10);
/// A title's width after its key.
const TITLE_WIDTH: usize = 60;
/// What a decision's buttons carry: the first three options, each at most
/// forty characters, as the relay accepts them.
const OPTIONS: usize = 3;
const OPTION_WIDTH: usize = 40;
/// APNs's limit on `apns-collapse-id`, in bytes.
const NOTICE_ID_MAX: usize = 64;

/// What happened on the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskEvent {
    /// `task.create`.
    Created,
    /// `task.set_status`, to a status the task wasn't in.
    Moved { to: TaskStatus },
    /// A QUESTION note.
    Asked,
    /// A new `task.block` edge (never a clear).
    Blocked { by: Uuid },
}

/// What an agent working on a task did, as its transition saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentNews {
    /// It stopped for a person: its question, else none.
    Blocked { terminal: Uuid, label: String, question: Option<String> },
    /// Its turn ended, and what it said.
    Finished { label: String, said: Option<String> },
    /// Its turn died.
    Failed { label: String },
}

/// A notice's class: which of a device's five switches it answers to. In
/// order of urgency, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Class {
    Decision,
    Blocked,
    Review,
    Done,
    New,
}

impl Class {
    /// The wire word: `event` on the relay's notice and the `notice` event.
    pub(crate) fn event(self) -> &'static str {
        match self {
            Class::Decision => "decision",
            Class::Blocked => "blocked",
            Class::Review => "review",
            Class::Done => "done",
            Class::New => "new",
        }
    }

    /// How hard it interrupts: a decision breaks a Focus; a review or a block
    /// sounds; a task finishing or filed lands quietly.
    pub(crate) fn level(self) -> &'static str {
        match self {
            Class::Decision => "time-sensitive",
            Class::Blocked | Class::Review => "active",
            Class::Done | Class::New => "passive",
        }
    }
}

/// One task's events inside its open window.
#[derive(Debug)]
pub(crate) struct Pending {
    opened: Instant,
    last: Instant,
    moved: Vec<TaskStatus>,
    created_by: Option<Actor>,
    asked: bool,
    blockers: Vec<Uuid>,
    agent: Option<AgentNews>,
}

impl Pending {
    fn new(now: Instant) -> Pending {
        Pending {
            opened: now,
            last: now,
            moved: Vec::new(),
            created_by: None,
            asked: false,
            blockers: Vec::new(),
            agent: None,
        }
    }

    /// When the window closes, as things stand.
    fn closes_at(&self) -> Instant {
        (self.last + QUIET_FOR).min(self.opened + AT_MOST)
    }
}

/// A notice, composed at a window's close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Composed {
    pub class: Class,
    pub body: String,
    /// A task decision's answer options; empty for anything else.
    pub options: Vec<String>,
    /// Whether this is news about the task's status, told once per status:
    /// an agent's news isn't, and may come again.
    pub of_status: bool,
    /// What, within the status, this news is about: a decision's QUESTION
    /// note, a block's blocker. Part of what is told once, so a follow-up
    /// question, or a second blocker, in the same status is news again.
    pub about: Option<Uuid>,
}

/// Whether a status decision also goes out as `kind: "decision"`, carrying
/// the task notice's fields: a relay older than ov-94 alerts only on that
/// kind, and an older app's tap opens a task only from it. A relay of this
/// era reads it as a task notice, so it alerts once either way. An agent's
/// fold stays `kind: "task"`: an old relay already alerts on its agent
/// notice, and a decision as well would be twice.
///
/// TODO(ov-94): remove after one stable release has shipped the relay and
/// apps that read `kind: "task"`, with the relay's `legacyDecision` reading.
pub(crate) const LEGACY_DECISION: bool = true;

/// The wire kind `composed` goes out as. See `LEGACY_DECISION`.
pub(crate) fn wire_kind(composed: &Composed) -> &'static str {
    if LEGACY_DECISION && composed.of_status && composed.class == Class::Decision { "decision" } else { "task" }
}

/// The id every platform files `task`'s notices under: `t:<runner>:<key>`,
/// or, past APNs's 64 bytes, `t:` and the first 16 hex characters of
/// `sha256(runner + ":" + task id)`.
pub(crate) fn notice_id(runner: &str, task: &Task) -> String {
    let plain = format!("t:{runner}:{}", task.key);
    if plain.len() <= NOTICE_ID_MAX
        && plain.bytes().all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b))
    {
        return plain;
    }
    let digest = sha2::Sha256::digest(format!("{runner}:{}", task.id).as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("t:{hex}")
}

/// `<key> <title>`, the title cut to `TITLE_WIDTH`.
pub(crate) fn title(task: &Task) -> String {
    format!("{} {}", task.key, one_line(&task.title, TITLE_WIDTH))
}

/// A body's quoted half, on one line and cut to a banner's width.
fn quoted(text: &str) -> String {
    one_line(text, farcooler_core::feed::SAID_WIDTH)
}

/// `Moved to In Review`, with how many files changed when that's known and
/// not none.
fn review_body(files: Option<u32>) -> String {
    match files {
        Some(1) => "Moved to In Review · 1 file changed".to_string(),
        Some(n) if n > 0 => format!("Moved to In Review · {n} files changed"),
        _ => "Moved to In Review".to_string(),
    }
}

/// A decision's options as its buttons carry them: from the QUESTION note's
/// `extra.options`, each on one line, none longer than `OPTION_WIDTH`, the
/// first `OPTIONS`.
fn options_of(question: &TaskNote) -> Vec<String> {
    question.extra["options"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|o| o.as_str())
                .map(|o| one_line(o, usize::MAX))
                .filter(|o| !o.is_empty() && o.chars().count() <= OPTION_WIDTH)
                .take(OPTIONS)
                .collect()
        })
        .unwrap_or_default()
}

/// Who filed a task, as its New Task notice says.
fn filer(actor: Actor) -> &'static str {
    match actor {
        Actor::Agent { .. } => "an agent",
        _ => "the manager",
    }
}

impl Watcher {
    /// The board moved: mark `task` pending with `event`, unless `actor`'s
    /// own write or a move nobody is told about. See this module's docs.
    pub(crate) fn task_event(&self, task: &Task, event: TaskEvent, actor: Actor) {
        if matches!(actor, Actor::User | Actor::Runner) {
            return;
        }
        let urgent = match event {
            TaskEvent::Moved { to: TaskStatus::NeedsDecision } | TaskEvent::Asked => true,
            TaskEvent::Moved { to: TaskStatus::InReview | TaskStatus::Done } => false,
            TaskEvent::Moved { .. } => return,
            TaskEvent::Created | TaskEvent::Blocked { .. } => false,
        };
        self.intake(task.id, urgent, |pending| match event {
            TaskEvent::Created => pending.created_by = Some(actor),
            TaskEvent::Moved { to } => pending.moved.push(to),
            TaskEvent::Asked => pending.asked = true,
            TaskEvent::Blocked { by } => pending.blockers.push(by),
        });
    }

    /// An agent working on `task` stopped, finished or failed: its news goes
    /// out on the task's thread, worded by task.
    pub(crate) fn agent_event(&self, task: &Task, news: AgentNews) {
        let urgent = matches!(news, AgentNews::Blocked { .. });
        self.intake(task.id, urgent, |pending| pending.agent = Some(news));
    }

    /// Add to `task`'s window, opening one if none is, and close it when it
    /// has been quiet `QUIET_FOR` or open `AT_MOST`, or at once when `urgent`
    /// opened it.
    fn intake(&self, task: Uuid, urgent: bool, add: impl FnOnce(&mut Pending)) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        let now = Instant::now();
        let fresh = {
            let mut pending = self.task_notices.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = !pending.contains_key(&task);
            let entry = pending.entry(task).or_insert_with(|| Pending::new(now));
            entry.last = now;
            add(entry);
            fresh
        };
        // A window already open closes on its own clock, and composes
        // whatever this added.
        if !fresh {
            return;
        }
        let me = self.me.clone();
        runtime.spawn(async move {
            if !urgent {
                loop {
                    let closes = {
                        let Some(watcher) = me.upgrade() else { return };
                        let pending = watcher.task_notices.lock().unwrap_or_else(|e| e.into_inner());
                        let Some(open) = pending.get(&task) else { return };
                        open.closes_at()
                    };
                    if Instant::now() >= closes {
                        break;
                    }
                    tokio::time::sleep_until(closes).await;
                }
            }
            let Some(watcher) = me.upgrade() else { return };
            let closed = watcher.task_notices.lock().unwrap_or_else(|e| e.into_inner()).remove(&task);
            if let Some(closed) = closed {
                watcher.close_task_window(task, closed).await;
            }
        });
    }

    /// Compose `id`'s notice from the task as it is now, and send it unless
    /// it says nothing or something already told.
    async fn close_task_window(&self, id: Uuid, pending: Pending) {
        let Ok(task) = self.service.store.get_task(id) else { return };
        let Some(composed) = self.compose(&task, &pending).await else { return };
        if composed.of_status {
            let mut told = self.task_told.lock().unwrap_or_else(|e| e.into_inner());
            // Told before it is sent, on purpose: a relay that refused it
            // isn't asked again for the same news, which would buzz twice if
            // the refusal was only a slow 200.
            let this = (composed.class, task.status_since, composed.about);
            if told.get(&id) == Some(&this) {
                return;
            }
            told.insert(id, this);
        }
        self.send_task_notice(&task, composed).await;
    }

    /// What `pending` adds up to for `task` as it is now: the most urgent
    /// class still true, or nothing.
    async fn compose(&self, task: &Task, pending: &Pending) -> Option<Composed> {
        let store = &self.service.store;
        let mut found: Vec<Composed> = Vec::new();
        let status = |class, body: String| Composed { class, body, options: Vec::new(), of_status: true, about: None };
        let agent = |class, body: String| Composed { class, body, options: Vec::new(), of_status: false, about: None };

        if (pending.asked || pending.moved.contains(&TaskStatus::NeedsDecision))
            && task.status == TaskStatus::NeedsDecision
        {
            let notes: Vec<TaskNote> = store
                .notes_for(task.id, None)
                .ok()?
                .into_iter()
                .filter(|n| matches!(n.kind, NoteKind::Question | NoteKind::Answer))
                .collect();
            if let Some(question) = crate::needs_you::waiting_question(task, &notes) {
                let asked = question.map(|q| q.body.trim()).filter(|b| !b.is_empty());
                found.push(Composed {
                    class: Class::Decision,
                    body: match asked {
                        Some(q) => format!("Needs your decision · {}", quoted(q)),
                        None => "Needs your decision".to_string(),
                    },
                    options: question.map(options_of).unwrap_or_default(),
                    of_status: true,
                    about: question.map(|q| q.id),
                });
            }
        }
        match &pending.agent {
            Some(AgentNews::Blocked { terminal, label, question }) => {
                let still = self
                    .state
                    .lock()
                    .await
                    .get(terminal)
                    .is_none_or(|o| o.activity == farcooler_protocol::v1::AgentActivity::Blocked);
                if still {
                    let asked = question.as_deref().map(str::trim).filter(|q| !q.is_empty());
                    found.push(agent(
                        Class::Decision,
                        format!("{label} needs you · {}", asked.map_or("Waiting for your answer".to_string(), quoted)),
                    ));
                }
            }
            Some(AgentNews::Failed { label }) => {
                found.push(agent(Class::Blocked, format!("{label}’s last turn didn’t finish")));
            }
            Some(AgentNews::Finished { label, said }) if pending.moved.is_empty() => {
                let said = said.as_deref().map(str::trim).filter(|s| !s.is_empty());
                found.push(agent(
                    Class::Done,
                    match said {
                        Some(s) => format!("{label} finished · {}", quoted(s)),
                        None => format!("{label} finished"),
                    },
                ));
            }
            _ => {}
        }
        if !pending.blockers.is_empty() {
            let edges = store.blocks_for(task.id).unwrap_or_default();
            for by in &pending.blockers {
                let Some(edge) = edges.iter().find(|e| e.blocked_by == *by) else { continue };
                let Ok(blocker) = store.get_task(*by) else { continue };
                if blocker.status == TaskStatus::Done {
                    continue;
                }
                let reason = edge.reason.trim();
                found.push(Composed {
                    about: Some(blocker.id),
                    ..status(
                        Class::Blocked,
                        if reason.is_empty() {
                            format!("Blocked on {}", blocker.key)
                        } else {
                            format!("Blocked on {} · {}", blocker.key, quoted(reason))
                        },
                    )
                });
                break;
            }
        }
        if pending.moved.contains(&TaskStatus::InReview) && task.status == TaskStatus::InReview {
            let files = task.worktree_id.and_then(|lane| match self.service.review_cache.counts(lane) {
                crate::review::Counts::Known(files, _, _) => Some(files),
                _ => None,
            });
            found.push(status(Class::Review, review_body(files)));
        }
        if pending.moved.contains(&TaskStatus::Done) && task.status == TaskStatus::Done {
            found.push(status(Class::Done, done_body(&store.notes_for(task.id, None).unwrap_or_default())));
        }
        if let Some(actor) = pending.created_by {
            let workspace = store.get_workspace(task.workspace_id).ok().map(|w| w.name);
            found.push(status(
                Class::New,
                match workspace {
                    Some(w) => format!("New in {w} · filed by {}", filer(actor)),
                    None => format!("New · filed by {}", filer(actor)),
                },
            ));
        }
        found.into_iter().min_by_key(|c| c.class)
    }

    /// Send `composed` about `task`: to every client as the `notice` event,
    /// and to the relay when paired.
    async fn send_task_notice(&self, task: &Task, composed: Composed) {
        let workspace = self.service.store.get_workspace(task.workspace_id).ok().map(|w| w.name);
        let runner = crate::service::stable_host_id(self.service.install_id()).to_string();
        let id = notice_id(&runner, task);
        let title = title(task);
        let _ = self.events.send(pb::Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(pb::event::Payload::Notice(pb::Notice {
                notice_id: id.clone(),
                event: composed.class.event().to_string(),
                level: composed.class.level().to_string(),
                title: title.clone(),
                body: composed.body.clone(),
                task_key: task.key.clone(),
                task_id: crate::wire::id_bytes(task.id),
                runner_id: runner,
                options: composed.options.clone(),
                workspace: workspace.clone().unwrap_or_default(),
                repository_id: crate::wire::id_bytes(task.repository_id),
            })),
        });
        let Some(pairing) = self.audience() else { return };
        let count = self.needs_you_count().await;
        let kind = wire_kind(&composed);
        self.tap(Tapped {
            kind: Some(kind),
            title: title.clone(),
            task: Some(task.key.clone()),
            workspace: workspace.clone(),
            needs_you: count,
            event: Some(composed.class.event()),
            level: Some(composed.class.level()),
            notice_id: Some(id.clone()),
            subtitle: composed.body.clone(),
            options: composed.options.clone(),
            ..Tapped::default()
        });
        let outgoing = crate::push::Outgoing {
            kind: Some(kind),
            title: &title,
            subtitle: &composed.body,
            task: Some(&task.key),
            workspace: workspace.as_deref(),
            needs_you: count,
            notice_id: Some(&id),
            event: Some(composed.class.event()),
            level: Some(composed.class.level()),
            options: &composed.options,
            ..Default::default()
        };
        if self.deliver(pairing, outgoing).await
            && let Some(count) = count
        {
            self.told(count);
        }
    }
}

/// `Done`, with the last DECISION note written since the task last entered
/// In Review, when there is one.
fn done_body(notes: &[TaskNote]) -> String {
    let reviewed = notes
        .iter()
        .filter(|n| {
            n.kind == NoteKind::StatusChange && n.extra["to"].as_str() == Some(TaskStatus::InReview.as_str())
        })
        .map(|n| n.at)
        .max();
    let decided = reviewed.and_then(|since| {
        notes.iter().filter(|n| n.kind == NoteKind::Decision && n.at >= since).max_by_key(|n| n.at)
    });
    match decided.map(|n| n.body.trim()).filter(|b| !b.is_empty()) {
        Some(decision) => format!("Done · {}", quoted(decision)),
        None => "Done".to_string(),
    }
}

/// The two maps the composer keeps on the watcher.
pub(crate) type Windows = HashMap<Uuid, Pending>;
pub(crate) type Told = HashMap<Uuid, (Class, i64, Option<Uuid>)>;

#[cfg(test)]
mod tests;

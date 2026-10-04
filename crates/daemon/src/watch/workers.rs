//! Following the subagents recorded on tasks (ov-213).
//!
//! `task worker` records which task a subagent works, with the orchestrator's
//! session and directory. Each tick, for every session holding a subagent
//! that is open or ended in the last day, this reads what the session and
//! the subagents' own transcripts wrote (`session_log::worker_follow`, which
//! needs no pane and does not go through `log_join`), and writes back what
//! only the transcript knows:
//!
//! - **A stop** (`<task-notification>`, or a foreground result): the subagent
//!   is ended on its task, as `finished`, `failed` or `stopped`, with a
//!   `worker` note. Notifications for anything not recorded are ignored.
//! - **A resume** (`resumedAgentId`, or new transcript lines later than the
//!   stop): the same row is reopened, with a note.
//! - **A launch whose description starts with a task key**: linked to that
//!   task if it's open on the orchestrator's board and nothing recorded
//!   says otherwise (`Store::link_worker_by_description`). "After ov-92
//!   lands" doesn't link; the key must open the description.
//! - **Spend**: each subagent's run, filed to its task.
//!
//! What isn't written (whether it's running, what it's doing and when it last
//! moved) is kept in `WorkerSeen` and put on each task as it's sent.

use std::collections::BTreeMap;
use std::path::PathBuf;

use farcooler_core::session_log::SubagentStatus;
use farcooler_core::session_log::worker_follow::{Followed, Seen, Wanted, WorkerFollow};
use farcooler_store::models::Actor;
use farcooler_store::workers::{EndReason, LinkedBy, TaskWorker, WorkerRecord};
use uuid::Uuid;

use super::Watcher;

/// How long after a subagent ended it's still followed, so that a message
/// that wakes it again is seen.
const FOLLOW_ENDED_MS: i64 = 24 * 60 * 60 * 1_000;

/// The follower, kept on the watcher.
pub(super) struct Follower {
    home: PathBuf,
    follow: Option<WorkerFollow>,
}

impl Follower {
    pub(super) fn new(home: PathBuf) -> Follower {
        Follower { follow: Some(WorkerFollow::new(home.clone())), home }
    }

    /// A follower reading under `home`, for a test.
    #[cfg(test)]
    pub(super) fn under(&mut self, home: PathBuf) {
        *self = Follower::new(home);
    }
}

fn end_reason(status: SubagentStatus) -> EndReason {
    match status {
        SubagentStatus::Completed => EndReason::Finished,
        SubagentStatus::Failed => EndReason::Failed,
        SubagentStatus::Killed | SubagentStatus::Stopped => EndReason::Stopped,
    }
}

impl Watcher {
    /// One pass: read what's new, and write back what it says. `now` is Unix
    /// milliseconds. Cheap when no subagent has a session recorded: one
    /// indexed read of the store.
    pub(crate) async fn follow_workers(&self, now: i64) {
        let store = &self.service.store;
        let rows = match store.followed_workers(now - FOLLOW_ENDED_MS) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "couldn't read the subagents to follow");
                return;
            }
        };
        let mut sessions: BTreeMap<String, (Wanted, Uuid)> = BTreeMap::new();
        for (worker, workspace) in &rows {
            let Some(session) = worker.session_id.clone() else { continue };
            let (wanted, _) = sessions.entry(session.clone()).or_insert_with(|| {
                let cwd = worker.session_cwd.clone().unwrap_or_default();
                (Wanted { session_id: session, cwd, agents: Vec::new() }, *workspace)
            });
            wanted.agents.push(worker.agent_id.clone());
        }
        let wanted: Vec<Wanted> = sessions.values().map(|(w, _)| w.clone()).collect();
        let (taken, home) = {
            let mut held = self.worker_follow.lock().unwrap_or_else(|e| e.into_inner());
            (held.follow.take(), held.home.clone())
        };
        let mut taken = taken.unwrap_or_else(|| WorkerFollow::new(home.clone()));
        if wanted.is_empty() {
            // Nothing to follow: let go of every session, with no hop.
            taken.follow(&[], now);
            self.service.worker_seen.observing([]);
            self.worker_follow.lock().unwrap_or_else(|e| e.into_inner()).follow = Some(taken);
            return;
        }
        let read = tokio::task::spawn_blocking(move || {
            let followed = taken.follow(&wanted, now);
            (taken, followed)
        })
        .await;
        // A failed join loses the file offsets; the next pass starts afresh.
        let Ok((taken, followed)) = read else {
            self.worker_follow.lock().unwrap_or_else(|e| e.into_inner()).follow = Some(WorkerFollow::new(home));
            return;
        };
        self.worker_follow.lock().unwrap_or_else(|e| e.into_inner()).follow = Some(taken);
        self.apply_followed(followed, &rows, &sessions);
    }

    fn apply_followed(
        &self,
        followed: Followed,
        rows: &[(TaskWorker, Uuid)],
        sessions: &BTreeMap<String, (Wanted, Uuid)>,
    ) {
        let seen_workers = &self.service.worker_seen;
        let located: Vec<String> = rows
            .iter()
            .filter(|(w, _)| w.session_id.as_ref().is_some_and(|s| followed.located.contains(s)))
            .map(|(w, _)| w.agent_id.clone())
            .collect();
        seen_workers.observing(located);
        for seen in followed.seen {
            match seen {
                Seen::Active { agent_id, at_ms, doing, model, .. } => {
                    let doing = doing.map(|(verb, object)| {
                        farcooler_core::feed::Phrase::action(&verb, &object).as_str().to_string()
                    });
                    seen_workers.note(&agent_id, at_ms, doing, model);
                    if let Some(at) = at_ms {
                        self.resume_if_later(&agent_id, at);
                    }
                }
                Seen::Ended { agent_id, status, .. } => self.stopped(&agent_id, end_reason(status)),
                Seen::Resumed { agent_id, .. } => self.resumed(&agent_id),
                Seen::Spawned { session, agent_id, description, .. } => {
                    let Some((wanted, workspace)) = sessions.get(&session) else { continue };
                    self.link_from_description(&wanted.session_id, &wanted.cwd, *workspace, &agent_id, &description);
                }
            }
        }
        crate::usage::record_subagents(
            &self.service.store,
            followed.spend.into_iter().map(|(_, turn)| turn).collect(),
        );
    }

    /// A subagent stopped: end it where it's open.
    fn stopped(&self, agent: &str, reason: EndReason) {
        let store = &self.service.store;
        let Ok(rows) = store.workers_of_agent("claude", agent) else { return };
        for worker in rows.into_iter().filter(|w| w.ended_at.is_none()) {
            match store.end_worker(worker.task_id, "claude", Some(agent), reason, Actor::Runner) {
                Ok(task) => self.announce_task_changed(&task, None, Actor::Runner),
                Err(e) => tracing::warn!(agent, error = %e, "couldn't end a subagent that stopped"),
            }
        }
    }

    /// A subagent is working again: reopen the row where it had ended.
    fn resumed(&self, agent: &str) {
        let store = &self.service.store;
        let Ok(rows) = store.workers_of_agent("claude", agent) else { return };
        for worker in rows.into_iter().filter(|w| w.ended_at.is_some()) {
            let record = WorkerRecord {
                harness: worker.harness.clone(),
                agent_id: worker.agent_id.clone(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: None,
                label: None,
                model: None,
                linked_by: LinkedBy::Description,
            };
            match store.record_worker(worker.task_id, &record, Actor::Runner) {
                Ok(task) => self.announce_task_changed(&task, None, Actor::Runner),
                // Its task is done or cancelled: it stays closed with it.
                Err(farcooler_core::DomainError::InvalidArgument { .. }) => {}
                Err(e) => tracing::warn!(agent, error = %e, "couldn't reopen a subagent that was resumed"),
            }
        }
    }

    /// A subagent wrote a line later than the stop that ended it: it was
    /// resumed, however it was woken. A line the stop itself came after (a
    /// transcript read late) isn't.
    fn resume_if_later(&self, agent: &str, at_ms: i64) {
        let Ok(rows) = self.service.store.workers_of_agent("claude", agent) else { return };
        if rows.iter().any(|w| w.ended_at.is_some_and(|ended| at_ms > ended)) {
            self.resumed(agent);
        }
    }

    fn link_from_description(&self, session: &str, cwd: &str, workspace: Uuid, agent: &str, description: &str) {
        let record = WorkerRecord {
            harness: "claude".into(),
            agent_id: agent.to_string(),
            session_id: Some(session.to_string()),
            session_cwd: Some(cwd.to_string()),
            orchestrator_terminal: None,
            label: Some(description.to_string()),
            model: None,
            linked_by: LinkedBy::Description,
        };
        match self.service.store.link_worker_by_description(workspace, description, &record) {
            Ok(Some(task)) => self.announce_task_changed(&task, None, Actor::Runner),
            Ok(None) => {}
            Err(e) => tracing::warn!(agent, error = %e, "couldn't link a subagent from its description"),
        }
    }
}

#[cfg(test)]
#[path = "workers_tests.rs"]
mod tests;

//! `landing.start` with `run` (ov-872, slice 5): read the charter's
//! `## Landing` section, start the landing, and hand it to the landing job
//! (`landing_job`) in the background. The answer is the landing as started;
//! the job's steps go on its timeline, which `farcooler landing show` reads.
//!
//! **One job per board at a time.** A second start while one runs is refused
//! `landing_job_running`: two jobs would push over each other's base.
//!
//! **Land is yours.** A start that says to land tickets the charter asked
//! about (`land`) comes only from a person: the owner, or a member. An agent
//! that says it is refused `land_is_yours`.
//!
//! CI is read through `gh`, as the CI watch reads it (`ci_watch::read_subject`),
//! and a canceled run is run again with `gh run rerun`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::board_ci::{CiStatus, sha_subject};
use farcooler_store::landings::Landing;
use farcooler_store::models::Actor;
use uuid::Uuid;

use crate::landing_job::{self, Ci, Outcome, Setup};
use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;

/// How long between CI reads while the job waits.
const POLL: Duration = Duration::from_secs(60);
/// The longest the job waits on one CI: a slow run plus a queue.
const CI_PATIENCE: Duration = Duration::from_secs(3 * 60 * 60);
/// The longest one gate command runs.
const GATE_TIMEOUT: Duration = Duration::from_secs(3 * 60 * 60);

/// The boards with a job running.
fn running() -> &'static Mutex<HashSet<Uuid>> {
    static RUNNING: OnceLock<Mutex<HashSet<Uuid>>> = OnceLock::new();
    RUNNING.get_or_init(Default::default)
}

/// A board's place in `running`, given back when dropped.
struct Running(Uuid);

impl Running {
    fn take(workspace: Uuid) -> Option<Running> {
        running().lock().unwrap_or_else(|e| e.into_inner()).insert(workspace).then_some(Running(workspace))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        running().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// `landing.start` with `run`.
pub(crate) async fn start(svc: &Arc<Service>, watcher: &Arc<Watcher>, p: pb::LandingStart) -> Result<Landing> {
    let workspace = required_id(&p.workspace_id)?;
    let actor = actor_from_wire(&p.actor)?;
    if p.land && !matches!(actor, Actor::User | Actor::Member { .. }) {
        return Err(DomainError::InvalidArgument { what: "land_is_yours" });
    }
    let charter = crate::charter_read::current(svc, workspace)?.unwrap_or_default();
    let rules = farcooler_core::charter_landing::parse(&String::from_utf8_lossy(&charter))
        .map_err(|_| DomainError::InvalidArgument { what: "charter_landing" })?
        .ok_or(DomainError::InvalidArgument { what: "no_landing_rules" })?;
    let (repository, repo) = crate::ci_watch::tree_of(svc, workspace).ok_or(DomainError::NotFound)?;
    let tickets = p.task_ids.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>()?;
    if tickets.is_empty() {
        return Err(DomainError::InvalidArgument { what: "no_tickets" });
    }
    let lander = p.lander_id.as_deref().map(required_id).transpose()?;
    let guard = Running::take(workspace).ok_or(DomainError::InvalidArgument { what: "landing_job_running" })?;
    let base = format!("{}/{}", rules.remote, rules.base);
    let landing = landing_job::prepare(&svc.store, workspace, &tickets, &base, &p.title, lander, p.land, actor)?;
    watcher.announce_plan_changed(workspace, actor);
    let repo_name = svc.store.get_repository(repository).map(|r| r.display_name).unwrap_or_default();
    let scratch: PathBuf = svc.root_dir().join("landings").join(farcooler_core::names::slug(&repo_name));
    let (svc, watcher, id, land) = (svc.clone(), watcher.clone(), landing.id, p.land);
    tokio::spawn(async move {
        let _guard = guard;
        let announce = |ws: Uuid| watcher.announce_plan_changed(ws, Actor::Runner);
        let setup = Setup {
            store: &svc.store,
            repo: repo.clone(),
            scratch,
            rules,
            poll: POLL,
            ci_patience: CI_PATIENCE,
            gate_timeout: GATE_TIMEOUT,
            announce: &announce,
        };
        let ci = GhCi { svc: svc.clone(), repository, tree: repo, memo: tokio::sync::Mutex::new(Default::default()) };
        for outcome in landing_job::run(&setup, &ci, id, land).await {
            match outcome {
                Outcome::Landed { landing, sha, .. } => tracing::info!(landing, sha = %sha, "a landing job landed"),
                Outcome::Stopped { landing, why, .. } => tracing::warn!(landing, why = %why, "a landing job stopped"),
                Outcome::Asked { landing, .. } => tracing::info!(landing, "a landing waits on its owner"),
                Outcome::PullRequest { landing, .. } => tracing::info!(landing, "a landing goes by pull request"),
            }
        }
        watcher.tell_departures();
    });
    Ok(landing)
}

/// CI through `gh`, as the CI watch reads it.
struct GhCi {
    svc: Arc<Service>,
    repository: Uuid,
    tree: PathBuf,
    memo: tokio::sync::Mutex<crate::ci_watch::Memo>,
}

impl Ci for GhCi {
    async fn status(&self, sha: &str) -> CiStatus {
        let mut memo = self.memo.lock().await;
        crate::ci_watch::read_subject(&self.svc, self.repository, &self.tree, &sha_subject(sha), &mut memo).await.status
    }

    async fn rerun(&self, sha: &str) -> bool {
        let read = {
            let mut memo = self.memo.lock().await;
            crate::ci_watch::read_subject(&self.svc, self.repository, &self.tree, &sha_subject(sha), &mut memo).await
        };
        let mut runs: Vec<String> = read.jobs.iter().filter(|j| j.state == "canceled").filter_map(|j| run_id(&j.url)).collect();
        runs.dedup();
        let mut any = false;
        for run in runs {
            let Ok(mut gh) = crate::stack::gh(&self.tree).await else { return false };
            let done = tokio::time::timeout(
                crate::stack::GH_TIMEOUT,
                gh.current_dir(&self.tree).args(["run", "rerun", &run]).stdin(std::process::Stdio::null()).kill_on_drop(true).output(),
            )
            .await;
            any |= matches!(done, Ok(Ok(o)) if o.status.success());
        }
        any
    }
}

/// The run id in a run's or job's page: `…/actions/runs/<id>[/job/<id>]`.
pub(crate) fn run_id(url: &str) -> Option<String> {
    let rest = url.split("/actions/runs/").nth(1)?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_is_read_from_a_run_or_job_page() {
        assert_eq!(run_id("https://github.com/o/r/actions/runs/123/job/456").as_deref(), Some("123"));
        assert_eq!(run_id("https://github.com/o/r/actions/runs/789").as_deref(), Some("789"));
        assert_eq!(run_id("https://github.com/o/r/pull/1"), None);
    }

    /// One job per board: a second start waits for the first to give it back.
    #[test]
    fn one_job_runs_per_board() {
        let board = Uuid::now_v7();
        let first = Running::take(board).expect("free");
        assert!(Running::take(board).is_none());
        drop(first);
        assert!(Running::take(board).is_some());
    }
}

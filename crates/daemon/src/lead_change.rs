//! A lead change starts the orchestrator afresh on the new lead's Claude
//! login (ov-585, phase 3 of `reports/ov-581/design.md`; the owner's Oct 16
//! decision).
//!
//! A pane's login is fixed at its first launch (`claude_logins`), and a
//! conversation lives in its login's directory, so an orchestrator can't be
//! moved to another login. When a workspace's lead is handed on
//! (`workspace.set_settings`), or goes back to the owner because the member
//! who led it is removed (`member.remove`), its live orchestrator is stopped
//! and a new one started the way `start_orchestrator` starts one: a new
//! conversation, on the new lead's login. A relaunch of an orchestrator that
//! finds an old lead's login recorded does the same, instead of refusing.
//!
//! **One at a time per workspace** (review 1, M1). Every start, relaunch and
//! promotion of a workspace's orchestrator, and every lead change with the
//! restart that follows it, holds the workspace's lock (`Locks`). Without
//! it, a start that had read the old lead's login could seat its pane after
//! the lead change had looked and found nothing to replace. A start that
//! still finds the lead changed once it's seated is refused `lead_changed`
//! (`refuse_a_stale_orchestrator`), the backstop.
//!
//! **What carries over:** everything the orchestrator is told to read, none
//! of which lives in a login's directory.
//!
//! - The charter: `<runtime>/workspaces/<workspace>/charter.md`
//!   (`workspace_home::charter_path`), by workspace id, handed to the pane as
//!   `FARCOOLER_CHARTER`. The rest of the workspace home carries too: it's
//!   the workspace's, not a person's (review 1, L1, accepted).
//! - Its memory (review 1, M4, the coordinator's ruling): an owner-led
//!   workspace keeps the repository's memory directory in the runner's home
//!   (`orchestrator::claude_memory_dir`), as before. A member-led one uses
//!   the workspace's own, `<home>/memory`, 0700 (`workspace_memory`), so a
//!   member's orchestrator never reads the owner's personal notes or another
//!   workspace's. That one carries from lead to lead; the first member lead
//!   starts with it empty.
//! - The board, the mailbox and the manager skill, which are the runner's.
//!
//! The old conversation itself is not carried: it's in the old lead's
//! directory, and resuming it would run it on their login.
//!
//! **Lanes already running are left running** on the login they started on
//! (review 1, L2, ruled): stopping them would throw away work. Lanes the new
//! orchestrator opens follow its lead (`login_of_opener`). A removed
//! member's running panes are stopped by member removal (ov-588), not here.
//!
//! **Never the old lead's login, never the owner's for a member lead:** the
//! new start takes its login from the store's lead (`lead_login`), the same
//! way every orchestrator start does, and nothing a pane says. A new lead
//! whose login can't be launched (a claude too old) is refused before the
//! lead is written (`ready_for_lead`), so the live orchestrator stays as it
//! was. A fresh start that fails after that stops the old orchestrator
//! rather than leave it running on the old lead's login, and the lead change
//! still answers with what it wrote (review 1, M3).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::{self, TerminalRole};
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use crate::claude_logins::Login;
use crate::service::Service;
use crate::watch::Watcher;

/// One lock per workspace, held across everything that seats or replaces
/// its orchestrator.
#[derive(Debug, Default)]
pub struct Locks {
    locks: Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    /// Told each time a caller finds a workspace's lock held and waits, for
    /// a test to know the second caller is queued.
    #[cfg(test)]
    pub(crate) waiting: tokio::sync::Notify,
    /// A start to stop in `open_terminal` once its login is decided, before
    /// it's seated, by workspace: (reached, release).
    #[cfg(test)]
    pauses: Mutex<HashMap<Uuid, Arc<(tokio::sync::Notify, tokio::sync::Notify)>>>,
}

impl Locks {
    /// Hold `workspace`'s lock until the guard drops.
    pub(crate) async fn hold(&self, workspace: Uuid) -> OwnedMutexGuard<()> {
        let lock = self.locks.lock().unwrap_or_else(|e| e.into_inner()).entry(workspace).or_default().clone();
        match lock.clone().try_lock_owned() {
            Ok(guard) => guard,
            Err(_) => {
                #[cfg(test)]
                self.waiting.notify_one();
                lock.lock_owned().await
            }
        }
    }

    /// Stop the next orchestrator start in `workspace` once its login is
    /// decided: `.0` is told when it gets there, and `.1` lets it go on.
    #[cfg(test)]
    pub(crate) fn pause_at(&self, workspace: Uuid) -> Arc<(tokio::sync::Notify, tokio::sync::Notify)> {
        let pause = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
        self.pauses.lock().unwrap().insert(workspace, pause.clone());
        pause
    }

    /// Where `open_terminal` stops when a test asked (`pause_at`).
    #[cfg(test)]
    pub(crate) async fn paused(&self, seat: Option<&crate::reps::Seat>) {
        let Some(crate::reps::Seat::Orchestrator(workspace)) = seat else { return };
        let pause = self.pauses.lock().unwrap().remove(workspace);
        if let Some(pause) = pause {
            pause.0.notify_one();
            pause.1.notified().await;
        }
    }
}

/// A member-led workspace's orchestrator memory: `<home>/memory`.
pub fn workspace_memory(root: &Path, workspace: Uuid) -> PathBuf {
    crate::workspace_home::home(root, workspace).join("memory")
}

impl Service {
    /// `start_orchestrator_held` under `workspace`'s lock.
    pub async fn start_orchestrator(
        &self,
        workspace: Uuid,
        harness: &str,
        replace: bool,
        handoff: Option<&str>,
    ) -> Result<models::Terminal> {
        let _seat = self.orchestrator_locks.hold(workspace).await;
        self.start_orchestrator_held(workspace, harness, replace, handoff).await
    }

    /// The lock a launch of terminal `id` holds: its workspace's when it's
    /// an orchestrator there, or one being made one (`role`).
    pub(crate) async fn seat_lock(&self, id: Uuid, role: Option<TerminalRole>) -> Result<Option<OwnedMutexGuard<()>>> {
        let term = self.store.get_terminal(id)?;
        Ok(match (role.unwrap_or(term.role), term.workspace_id) {
            (TerminalRole::Orchestrator, Some(workspace)) => Some(self.orchestrator_locks.hold(workspace).await),
            _ => None,
        })
    }

    /// `launch` with a member-led workspace's own memory directory in place
    /// of the repository's in the runner's home. Made 0700 if missing; one
    /// that can't be made is left unnamed (claude then keeps memory in the
    /// lead's own config directory), never the owner's.
    pub(crate) fn lead_memory(&self, mut launch: crate::orchestrator::OrchestratorLaunch) -> crate::orchestrator::OrchestratorLaunch {
        match self.store.workspace_lead(launch.workspace) {
            Ok(None) => {}
            Ok(Some(_)) => {
                let dir = workspace_memory(self.root_dir(), launch.workspace);
                let home = crate::workspace_home::home(self.root_dir(), launch.workspace);
                let made = std::fs::create_dir_all(home).and_then(|()| crate::claude_logins::private_dir(&dir));
                launch.memory_dir = match made {
                    Ok(()) => Some(dir),
                    Err(e) => {
                        tracing::warn!(kind = ?e.kind(), "couldn't make a workspace's memory directory; its orchestrator names none");
                        None
                    }
                };
            }
            Err(_) => launch.memory_dir = None,
        }
        launch
    }

    /// Before `workspace`'s lead becomes `lead` (`None`: the owner): refuse
    /// what a fresh orchestrator on their login would be refused
    /// (`claude_too_old`, `login_gone`), so a refusal writes nothing. Only
    /// when there's a live orchestrator on another login to start afresh.
    /// The caller holds the workspace's lock.
    pub(crate) async fn ready_for_lead(&self, workspace: Uuid, lead: Option<&str>) -> Result<()> {
        let Some(live) = self.orchestrator_seat(workspace).await?.0 else { return Ok(()) };
        let next = lead.map_or(Login::Runner, |h| Login::member(h.to_string()));
        if self.same_login(&self.recorded_login(&live)?, &next)? {
            return Ok(());
        }
        let ws = self.store.get_worktree(live.worktree_id)?;
        self.launch_dir(&next, &live.command_preset, Path::new(&ws.worktree_path)).await.map(drop)
    }

    /// After `workspace`'s lead changed: its live orchestrator, when it's on
    /// another login, replaced by a fresh one on the lead's. The new
    /// orchestrator, or `None` when there was nothing to replace. A start
    /// that fails stops the old one: it isn't left running on a login that
    /// isn't its lead's. But when an orchestrator on the lead's login holds
    /// the seat by then, that's no failure (review 1, M3). The caller holds
    /// the workspace's lock.
    pub(crate) async fn follow_lead(&self, workspace: Uuid) -> Result<Option<models::Terminal>> {
        let Some(live) = self.orchestrator_seat(workspace).await?.0 else { return Ok(None) };
        let failed = match self.fresh_on_new_lead(&live).await {
            Ok(fresh) => return Ok(fresh),
            Err(e) => e,
        };
        if let Some(now) = self.orchestrator_seat(workspace).await?.0 {
            if self.same_login(&self.recorded_login(&now)?, &self.lead_login(workspace)?)? {
                return Ok(None);
            }
            if let Err(stop) = self.stop_terminal(now.id).await {
                tracing::warn!(terminal = %now.id, error = %stop.what(), "couldn't stop an orchestrator on an old lead's login");
            }
        }
        Err(failed)
    }

    /// `workspace.set_settings`' lead: write it and follow it. The caller
    /// holds the workspace's lock, and has passed `ready_for_lead` under it.
    /// A restart that fails after the write isn't a refusal: its word comes
    /// back as the second value, and is logged.
    pub(crate) async fn set_lead(&self, workspace: Uuid, lead: Option<&str>) -> Result<(Option<models::Terminal>, Option<&'static str>)> {
        self.store.set_workspace_lead(workspace, lead)?;
        match self.follow_lead(workspace).await {
            Ok(fresh) => Ok((fresh, None)),
            Err(e) => {
                tracing::warn!(%workspace, error = %e.what(), "the lead changed, and its orchestrator couldn't start afresh on the new lead's login");
                Ok((None, Some(e.what())))
            }
        }
    }

    /// The workspaces `handle` leads, for `member.remove` to follow once
    /// their lead has gone back to the owner.
    pub fn led_by(&self, handle: &str) -> Result<Vec<Uuid>> {
        let mut led = Vec::new();
        for workspace in self.store.list_workspaces(None)? {
            if self.store.workspace_lead(workspace.id)?.as_deref() == Some(handle) {
                led.push(workspace.id);
            }
        }
        Ok(led)
    }

    /// `follow_lead` for every workspace in `workspaces`, each under its
    /// lock, after their member lead was removed: the orchestrators started.
    /// A failure is logged, not returned: the member is gone either way, and
    /// `follow_lead` has stopped what it couldn't replace.
    pub async fn follow_leads(&self, workspaces: &[Uuid]) -> Vec<models::Terminal> {
        let mut started = Vec::new();
        for &workspace in workspaces {
            let _seat = self.orchestrator_locks.hold(workspace).await;
            match self.follow_lead(workspace).await {
                Ok(fresh) => started.extend(fresh),
                Err(e) => {
                    tracing::warn!(%workspace, error = %e.what(), "couldn't start an orchestrator afresh on its new lead's login")
                }
            }
        }
        started
    }

    /// `term` started afresh on its lead's login when it's an orchestrator on
    /// another's, in place of relaunching it (`relaunch_held`), or `None` to
    /// relaunch it as it is. Replaced when it holds the seat; when it's down,
    /// the new one takes the seat and its record goes. When another
    /// orchestrator holds the seat, `None`: the relaunch refuses as it would
    /// for any second orchestrator. The caller holds the workspace's lock.
    pub(crate) async fn fresh_on_new_lead(&self, term: &models::Terminal) -> Result<Option<models::Terminal>> {
        let (TerminalRole::Orchestrator, Some(workspace)) = (term.role, term.workspace_id) else { return Ok(None) };
        if self.same_login(&self.recorded_login(term)?, &self.lead_login(workspace)?)? {
            return Ok(None);
        }
        let live = self.orchestrator_seat(workspace).await?.0;
        if live.as_ref().is_some_and(|l| l.id != term.id) {
            return Ok(None);
        }
        tracing::info!(terminal = %term.id, %workspace, "starting an orchestrator afresh on its new lead's login");
        let fresh = self.start_orchestrator_held(workspace, &term.command_preset, live.is_some(), None).await?;
        if live.is_none() {
            match self.remove_terminal(term.id).await {
                Ok(()) | Err(DomainError::NotFound) => {}
                Err(e) => tracing::warn!(terminal = %term.id, error = %e.what(), "couldn't remove an old lead's orchestrator"),
            }
        }
        Ok(Some(fresh))
    }

    /// Refuse `lead_changed` for `term`, just seated as its workspace's
    /// orchestrator, when its login isn't its lead's now: the backstop to the
    /// workspace's lock, for a lead written without it.
    pub(crate) fn refuse_a_stale_orchestrator(&self, term: &models::Terminal) -> Result<()> {
        match (term.role, term.workspace_id) {
            (TerminalRole::Orchestrator, Some(workspace)) => self.refuse_another_leads_login(term, workspace),
            _ => Ok(()),
        }
    }
}

/// `terminal.restart`, and the restarted pane's layout published: a restart
/// that started an orchestrator afresh, or rebuilt a lost pane, made a new
/// window, and the Mac reads layouts only from these events (review 1, M2).
pub(crate) async fn restart_published(svc: &Service, watcher: &Watcher, id: Uuid) -> Result<models::Terminal> {
    let restarted = svc.restart_terminal(id).await?;
    publish_layouts(svc, watcher, std::slice::from_ref(&restarted)).await;
    Ok(restarted)
}

/// Publish the layout of each of `started`'s worktrees.
pub(crate) async fn publish_layouts(svc: &Service, watcher: &Watcher, started: &[models::Terminal]) {
    for terminal in started {
        if let Ok(groups) = svc.layout(terminal.worktree_id).await {
            watcher.publish_layout(terminal.worktree_id, &groups);
        }
    }
}

/// `answer` with a failed orchestrator restart's word on it.
pub(crate) fn with_restart_failure(mut answer: pb::Workspace, failed: Option<&'static str>) -> pb::Workspace {
    answer.orchestrator_not_restarted = failed.map(str::to_string);
    answer
}

#[cfg(test)]
#[path = "lead_change_tests.rs"]
mod tests;

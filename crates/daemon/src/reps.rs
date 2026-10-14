//! A member's representative (ov-494): a Claude Code pane the runner binds to
//! one person in one workspace, which relays that person's asks to the
//! orchestrator and the orchestrator's questions back, and never answers for
//! them (ov-479 D1 and D3; `people` has the rules).
//!
//! It runs in the repository's main checkout, as the orchestrator does, with
//! the plugin's `rep` skill as its first message, told who it's for and whose
//! Claude account the runner uses (ov-479 D2). Its messages are filed on the
//! member's desk card in that workspace, made the first time and kept across
//! reps, so the card's thread is that person's conversation with the
//! orchestrator.
//!
//! Claude Code only: the account it runs on is the point of D2, and the skill
//! is the plugin's.

use farcooler_core::{DomainError, Result};
use farcooler_store::models::{self, Actor, TaskUpdate};
use uuid::Uuid;

use crate::service::Service;

/// Where a new pane sits before its first launch (`Service::open_terminal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seat {
    /// Its workspace's orchestrator.
    Orchestrator(Uuid),
    /// A member's rep in a workspace, by handle.
    Rep(Uuid, String),
}

/// The label every desk card carries, which the manager skill reads as "a
/// person's thread, never work to dispatch".
pub const DESK_LABEL: &str = "rep";

/// A rep that started: its pane, and its desk card.
#[derive(Debug, Clone)]
pub struct Started {
    pub terminal: models::Terminal,
    pub desk: models::Task,
}

/// The first message a rep starts on: the skill, by name, then who it's for
/// and whose account it runs on, which it tells its person first.
pub fn first_prompt(name: &str, handle: &str, workspace: &str, account: Option<&str>) -> String {
    let account = match account {
        Some(email) => format!("the Claude account {email}, which everyone on this runner shares"),
        None => "the Claude account signed in on this runner, which everyone on it shares".to_string(),
    };
    format!("/farcooler:rep You represent {name} ({handle}) in the {workspace} workspace. Work here runs on {account}.")
}

/// `FARCOOLER_REP` for a rep's pane: its member's handle, which a handle's
/// own rule keeps plain (`valid_handle`).
pub(crate) fn env(store: &farcooler_store::Store, term: &models::Terminal) -> Option<(String, String)> {
    let rep = store.rep_of_terminal(term.id).ok().flatten()?;
    Some((farcooler_core::pane_env::REP.to_string(), rep.handle))
}

impl Service {
    /// Open `handle`'s rep in `workspace`, running `harness` (`claude`, or
    /// `claude:<model>`). One per member per workspace: a running one is
    /// refused (`rep_running`), and a stopped one is removed first.
    pub async fn start_rep(&self, workspace: Uuid, handle: &str, harness: &str) -> Result<Started> {
        let harness = match harness.trim() {
            "" => "claude",
            h => h,
        };
        if crate::orchestrator::harness_of(harness) != Some(crate::orchestrator::Harness::Claude) {
            return Err(DomainError::InvalidArgument { what: "command_preset" });
        }
        let member = self.store.member(handle)?;
        let board = self.store.get_workspace(workspace)?;
        if let Some(rep) = self.store.rep_for(workspace, handle)? {
            let pane = self.store.get_terminal(rep.terminal)?;
            if self.is_running(&pane) {
                return Err(DomainError::Conflict { what: "rep_running" });
            }
            self.stop_terminal(pane.id).await?;
            self.remove_terminal(pane.id).await?;
        }
        let main = self
            .store
            .list_worktrees_for_repository(board.repository_id)?
            .into_iter()
            .find(|w| w.is_main_checkout)
            .ok_or(DomainError::NotFound)?;
        let desk = self.desk(&board, &member)?;
        let account = self.claude_account();
        let prompt = first_prompt(&member.name, handle, &board.name, account.as_deref());
        let title = format!("{}'s rep", member.name);
        let seat = Some(Seat::Rep(workspace, handle.to_string()));
        let terminal = self.open_terminal(main.id, &title, harness, Some(&prompt), None, seat, None).await?;
        Ok(Started { terminal, desk })
    }

    /// `member`'s desk card on `board`: the one made before, or a new one,
    /// labeled `DESK_LABEL` and saying what it's for.
    fn desk(&self, board: &models::Workspace, member: &farcooler_store::people::Member) -> Result<models::Task> {
        if let Some(id) = self.store.rep_desk(board.id, &member.handle)? {
            if let Ok(task) = self.store.get_task(id) {
                return Ok(task);
            }
        }
        let made = self.store.create_task(board.id, &format!("{}'s desk", member.name), Actor::Runner)?;
        let update = TaskUpdate {
            title: made.title.clone(),
            intent: format!(
                "Where {}'s rep files their asks and answers for the orchestrator, and where the orchestrator's \
                 questions for them are kept. A person's thread, not work to dispatch.",
                member.name
            ),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            labels: vec![DESK_LABEL.to_string()],
            worktree_id: None,
        };
        let task = self.store.update_task(made.id, made.resource_version, &update)?;
        self.store.set_rep_desk(board.id, &member.handle, task.id)?;
        Ok(task)
    }

    /// Stop and remove every rep `handle` has, before they're removed.
    pub async fn stop_reps_of(&self, handle: &str) -> Result<()> {
        for rep in self.store.reps_of(handle)? {
            if self.store.get_terminal(rep.terminal).is_ok() {
                self.stop_terminal(rep.terminal).await?;
                self.remove_terminal(rep.terminal).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "reps_tests.rs"]
mod tests;

//! ov-958: a landing group shown as a Landing (ov-950, lane 7). Read-time only: the group item
//! in the engine's state is the one truth, and nothing here is stored. A live group renders as
//! the board's `Landing` and its current verb maps to a `LandingPhase`, so the Mac, the phones
//! and the CLI need no change. Old landing rows stay as history.
//!
//! | The group is at | Phase |
//! |---|---|
//! | Blocked, the target's turn not yet its own | Waiting for the earlier landing |
//! | `await-base` | Waiting for main |
//! | `fold` | Integrating |
//! | `run`, `push` | Gating locally |
//! | `await-ci` | Waiting for CI (CI failed, when the SHA's read says so) |
//! | A wait on a person | Ready to land |
//! | Failed after a push, CI's read of that commit failed | CI failed |
//! | Any other failure, or uncertain | Stalled |
//!
//! A group that landed leaves live state on the next step but keeps an archive entry (name,
//! tickets, pushed commit, time), so it stays drawn as Landed until its tickets leave the board.
//! A ticket draws in one group only: a live group wins, then the newest finished one.
use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::Result;
use farcooler_workflow::{Item, Phase, State, StageKind, digest};

use crate::board_ci::{CiRead, CiStatus, sha_subject};
use crate::error::map_err;
use crate::landing_phase::LandingPhase;
use crate::landings::{Landing, LandingState, Route};
use crate::models::get_uuid;
use crate::store::Store;

/// A live group as the board draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupLanding {
    /// The group's item id in the engine.
    pub group: String,
    /// Its name in the definition ("landing").
    pub name: String,
    /// The tickets it holds, in the order they joined.
    pub tickets: Vec<Uuid>,
    /// The commit it pushed, once a push has passed. `await-ci` follows this.
    pub pushed_sha: Option<String>,
    pub since: i64,
    phase: Phase,
    verb: Option<StageKind>,
    after_push: bool,
}

impl GroupLanding {
    /// Landed, failed or cancelled: it will not move again.
    pub fn is_finished(&self) -> bool { matches!(self.phase, Phase::Done | Phase::Failed | Phase::Cancelled) }

    /// What it is doing, given what CI says of its pushed commit.
    pub fn phase(&self, ci: Option<&CiRead>) -> LandingPhase {
        match (&self.phase, &self.verb) {
            (Phase::Done, _) => LandingPhase::Landed,
            (Phase::Cancelled, _) => LandingPhase::Dropped,
            (Phase::Failed, _) if self.after_push && ci.is_some_and(|c| c.status == CiStatus::Failed) => LandingPhase::CiFailed,
            (Phase::Failed | Phase::Uncertain, _) => LandingPhase::Stalled,
            (Phase::Blocked, _) => LandingPhase::WaitingForPreviousLanding,
            (Phase::WaitingApproval | Phase::WaitingReview, _) => LandingPhase::ReadyToLand,
            (_, Some(StageKind::AwaitBase)) => LandingPhase::WaitingForMain,
            (_, Some(StageKind::Fold)) => LandingPhase::Integrating,
            (_, Some(StageKind::AwaitCi)) if ci.is_some_and(|c| c.status == CiStatus::Failed) => LandingPhase::CiFailed,
            (_, Some(StageKind::AwaitCi)) => LandingPhase::WaitingForCi,
            _ => LandingPhase::GatingLocally,
        }
    }

    /// The stored-state word a landing in that phase would carry.
    pub fn state(&self, ci: Option<&CiRead>) -> LandingState {
        match self.phase(ci) {
            LandingPhase::Landed => LandingState::Landed,
            LandingPhase::Dropped => LandingState::Dropped,
            LandingPhase::CiFailed => LandingState::Red,
            LandingPhase::WaitingForCi if ci.is_some_and(|c| c.status == CiStatus::Passed) => LandingState::Green,
            LandingPhase::WaitingForCi => LandingState::Pushed,
            LandingPhase::GatingLocally => LandingState::Gating,
            _ => LandingState::Open,
        }
    }

    /// The `Landing` row it renders as. Its id is stable for the group, and its number is 0:
    /// the title is what a person reads.
    pub fn landing(&self, workspace: Uuid, ci: Option<&CiRead>) -> Landing {
        let id = Uuid::parse_str(&digest(&("overlay", workspace, &self.group))[4..36]).unwrap_or_else(|_| Uuid::nil());
        Landing {
            id, workspace_id: workspace, number: 0, route: Route::Push, base: String::new(),
            pushed_sha: self.pushed_sha.clone(), state: self.state(ci), state_since: self.since,
            lander_id: None, title: title_of(&self.name), collecting: false, actor: "runner".into(),
            created_at: self.since, landed_at: None, resource_version: 0, tickets: self.tickets.clone(),
        }
    }
}

/// "landing" reads as "Landing"; a longer name keeps its words.
fn title_of(name: &str) -> String {
    let words = name.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(|| "Landing".to_string(), |c| c.to_uppercase().chain(chars).collect())
}

fn is_group(item: &Item) -> bool { item.group_of.is_some() }

/// The commit a group has pushed: its candidate, once a push step is behind it.
fn pushed(item: &Item) -> Option<String> {
    let push = item.definition.stages.iter().position(|s| s.kind == StageKind::Push)?;
    (item.stage > push).then(|| item.candidate.clone()).flatten()
}

/// Every group in `state` worth drawing, with tickets resolved by `task_of`: the live ones, any
/// that failed and wait to be cancelled, and the ones that landed (kept in the archive). A ticket
/// draws in one group only: a live group first, then the newest finished one, so a rejected
/// ticket that rebuilt shows on its new group and not on the stale one. A finished group left
/// with no ticket is not drawn.
pub fn groups_in(state: &State, task_of: &dyn Fn(&str) -> Option<Uuid>, since: &dyn Fn(&str) -> i64) -> Vec<GroupLanding> {
    let mut all: Vec<GroupLanding> = state.items.iter().filter(|(_, i)| is_group(i)).map(|(id, item)| {
        let name = item.group_of.as_ref().map(|g| g.name.clone()).unwrap_or_default();
        let sha = pushed(item);
        GroupLanding {
            group: id.clone(), name,
            tickets: item.members.iter().map(|m| &m.item).chain(&item.handed).filter_map(|i| task_of(i)).collect(),
            after_push: sha.is_some(), pushed_sha: sha, since: if item.since_ms > 0 { item.since_ms } else { since(id) },
            phase: item.phase.clone(), verb: item.definition.stages.get(item.stage).map(|s| s.kind.clone()),
        }
    }).collect();
    // A group that landed has left live state; its archive entry keeps what the board draws.
    for (id, archived) in &state.archive {
        let (Phase::Done, Some(group)) = (&archived.phase, &archived.group) else { continue };
        all.push(GroupLanding {
            group: id.clone(), name: group.name.clone(),
            tickets: group.tickets.iter().filter_map(|i| task_of(i)).collect(),
            pushed_sha: group.pushed.clone(), after_push: group.pushed.is_some(),
            since: if group.since_ms > 0 { group.since_ms } else { since(id) },
            phase: Phase::Done, verb: None,
        });
    }
    let finished = GroupLanding::is_finished;
    let mut order: Vec<usize> = (0..all.len()).collect();
    order.sort_by_key(|&i| (finished(&all[i]), std::cmp::Reverse(all[i].since)));
    let mut claimed = std::collections::BTreeSet::new();
    let mut keep = vec![true; all.len()];
    for i in order {
        let had = all[i].tickets.len();
        all[i].tickets.retain(|t| claimed.insert(*t));
        keep[i] = !(finished(&all[i]) && had > 0 && all[i].tickets.is_empty());
    }
    all.into_iter().zip(keep).filter_map(|(g, k)| k.then_some(g)).collect()
}

impl Store {
    /// The live landing groups on `workspace`, drawn as landings.
    pub fn workflow_groups(&self, workspace: Uuid) -> Result<Vec<GroupLanding>> {
        let name = workspace.to_string();
        let Some(state) = self.workflow_state(&name).map_err(|e| { tracing::warn!(error = %e, "workflow state unreadable"); farcooler_core::DomainError::OperationFailed })? else {
            return Ok(Vec::new());
        };
        if !state.items.values().any(is_group) && !state.archive.values().any(|a| a.group.is_some()) { return Ok(Vec::new()); }
        let conn = self.conn();
        let owned: BTreeMap<String, Uuid> = {
            let mut stmt = conn
                .prepare("SELECT item, task_id FROM workflow_owned_items WHERE workspace = ?1 AND task_id IS NOT NULL")
                .map_err(map_err)?;
            let rows = stmt.query_map(params![name], |r| Ok((r.get::<_, String>(0)?, get_uuid(r, 1)?))).map_err(map_err)?;
            rows.collect::<rusqlite::Result<_>>().map_err(map_err)?
        };
        let moved = |group: &str| -> i64 {
            // The newest event that changed the group: its notice line names it.
            let found: Option<String> = conn
                .query_row(
                    "SELECT payload FROM workflow_events WHERE workspace = ?1 AND notices LIKE ?2
                      ORDER BY revision DESC LIMIT 1",
                    params![name, format!("%\"{group}:%")], |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten();
            found
                .and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
                .and_then(|v| v.get("now_ms").and_then(serde_json::Value::as_i64))
                .unwrap_or(0)
        };
        Ok(groups_in(&state, &|item| owned.get(item).copied(), &moved))
    }

    /// Every board's pushed group commits, for the CI watch to read (`ci_watch::wanted`).
    pub fn workflow_ci_subjects(&self) -> Result<Vec<(Uuid, String)>> {
        let mut out = Vec::new();
        for name in self.workflow_workspaces().map_err(|e| { tracing::warn!(error = %e, "workflow state unreadable"); farcooler_core::DomainError::OperationFailed })? {
            let Ok(workspace) = Uuid::parse_str(&name) else { continue };
            for group in self.workflow_groups(workspace)? {
                if group.phase != Phase::Done && let Some(sha) = group.pushed_sha { out.push((workspace, sha_subject(&sha))); }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;

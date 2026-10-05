//! A lane's pull requests, found and staged (ov-312): which of the repository's
//! PRs belong to which card of which lane, the stage of each (`pr_stage`), and
//! the lane's rolled-up stage, laid onto the plan as it is read.
//!
//! The owner's ruling is one PR per card, not per lane: a lane that carries
//! three cards carries up to three PRs. A card's PR is the one whose head branch
//! names the card's key (`ov-312/pr-stage`), or whose title opens with it
//! ("ov-312: lanes show their PR stage"); a card nothing names takes the PR on
//! the lane's own branch only when it is the lane's one card.
//!
//! Part of the plan layer: only `rpc_plan` and `pr_watch` call it.

use std::collections::HashMap;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::TaskStatus;
use farcooler_store::plan::LaneState;
use farcooler_store::plan_read::{CardRef, LaneView, Plan};
use uuid::Uuid;

use crate::pr_stage::{self, Kind, Phase, Stage};
use crate::service::Service;
use crate::stack::{CheckState, PrInfo, PrState};

/// What the runner last heard of a repository's pull requests.
#[derive(Debug, Clone, Default)]
pub struct Reads {
    /// Whether `gh` has ever answered. False is "could not ask", which is not
    /// an empty list.
    pub known: bool,
    pub prs: Vec<PrInfo>,
}

impl Reads {
    pub fn of(svc: &Service, repository: Uuid) -> Reads {
        Reads { known: svc.pr_answer_is_known(repository), prs: svc.pr_cache_get(repository).unwrap_or_default() }
    }
}

/// A lane's stages: its own, and each card's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneStages {
    pub lane: Option<Stage>,
    pub cards: HashMap<Uuid, Stage>,
}

pub fn phase_of(state: LaneState) -> Phase {
    match state {
        LaneState::Queued => Phase::Queued,
        LaneState::Building => Phase::Building,
        LaneState::Review => Phase::Review,
        LaneState::Fixing => Phase::Fixing,
        LaneState::Landing => Phase::Landing,
        LaneState::Landed => Phase::Landed,
        LaneState::Dropped => Phase::Dropped,
    }
}

/// Whether `text` names `key` as a word of its own: `ov-31` is not in
/// `ov-312/stage`, and `ov-312` is.
fn mentions(text: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let (text, key) = (text.to_ascii_lowercase(), key.to_ascii_lowercase());
    let word = |c: char| c.is_ascii_alphanumeric();
    text.match_indices(&key).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + key.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// Whether a title opens with the card's key: `ov-312: ...`, `[ov-312] ...`,
/// `ov-312 ...`. A key further in is a mention, and a PR that cites a card
/// is not that card's PR.
fn title_starts_with(title: &str, key: &str) -> bool {
    let title = title.trim_start().trim_start_matches(['[', '(']).to_ascii_lowercase();
    title
        .strip_prefix(&key.to_ascii_lowercase())
        .is_some_and(|rest| !rest.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()))
}

/// A pull request nobody here opened for a card: a fork's, or a revert of one
/// that was.
fn is_noise(p: &PrInfo) -> bool {
    p.is_fork || p.title.trim_start().to_ascii_lowercase().starts_with("revert")
}

/// The best of several PRs that all claim a card: one still open over one
/// merged over one closed (a branch's earlier, abandoned PR must not hide the
/// live one), then the newest.
fn best<'a>(candidates: impl Iterator<Item = &'a PrInfo>) -> Option<&'a PrInfo> {
    let rank = |p: &PrInfo| match p.status.state {
        PrState::Open | PrState::Draft | PrState::Unknown => 2,
        PrState::Merged => 1,
        PrState::Closed => 0,
    };
    candidates.max_by_key(|p| (rank(p), p.status.number))
}

/// The PR of a card: the one whose head branch names its key, or whose title
/// opens with it. `only_card` says the lane works nothing else, so the PR on the
/// lane's own branch is its too; with several cards on a lane that branch
/// holds all of their work, and so says nothing of any one.
fn pr_of<'a>(prs: &'a [PrInfo], key: &str, branch: &str, only_card: bool) -> Option<&'a PrInfo> {
    let mine = || prs.iter().filter(|p| !is_noise(p));
    best(mine().filter(|p| mentions(&p.head_ref, key) || title_starts_with(&p.title, key))).or_else(|| {
        if only_card && !branch.is_empty() { best(mine().filter(|p| p.head_ref == branch)) } else { None }
    })
}

/// One lane's stages against one repository's reads.
pub fn lane_stages(lane: &LaneView, cards: &[CardRef], reads: &Reads) -> LaneStages {
    let phase = phase_of(lane.lane.state);
    if phase == Phase::Queued {
        return LaneStages::default();
    }
    let finished = |status: TaskStatus| matches!(status, TaskStatus::Done | TaskStatus::Cancelled);
    let mut out = LaneStages::default();
    let mut matched: Vec<(Uuid, Option<&PrInfo>)> = Vec::new();
    for c in &lane.cards {
        let Some(card) = cards.iter().find(|r| r.task_id == c.task_id) else { continue };
        let pr = if reads.known { pr_of(&reads.prs, &card.key, &lane.lane.branch, lane.cards.len() == 1) } else { None };
        if pr.is_none() && finished(card.status) {
            continue;
        }
        matched.push((card.task_id, pr));
    }
    let any_pr = matched.iter().any(|(_, pr)| pr.is_some());
    for (task, pr) in matched {
        let stage = if !reads.known {
            // Could not ask. Said where a pull request is expected, and
            // silent while the lane is still building one.
            phase.is_active().then(Stage::unknown)
        } else if pr.is_none() && any_pr {
            // A sibling card has a PR and this one has none yet: it is the
            // least advanced of the lane.
            pr_stage::derive(None, Phase::Building)
        } else {
            pr_stage::derive(pr, phase)
        };
        if let Some(stage) = stage {
            out.cards.insert(task, stage);
        }
    }
    let ordered: Vec<Stage> = lane.cards.iter().filter_map(|c| out.cards.get(&c.task_id).cloned()).collect();
    out.lane = pr_stage::rollup(&ordered);
    out
}

/// The numbers of the pull requests a lane's cards work.
pub fn prs_of(lane: &LaneView, cards: &[CardRef], reads: &Reads) -> Vec<u32> {
    let mut found: Vec<u32> = lane
        .cards
        .iter()
        .filter_map(|c| cards.iter().find(|r| r.task_id == c.task_id))
        .filter_map(|card| pr_of(&reads.prs, &card.key, &lane.lane.branch, lane.cards.len() == 1))
        .map(|p| p.status.number)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

pub fn pb_stage(s: &Stage) -> pb::PrStage {
    pb::PrStage {
        kind: (match s.kind {
            Kind::Building => pb::PrStageKind::Building,
            Kind::AgentReview => pb::PrStageKind::AgentReview,
            Kind::Fixing => pb::PrStageKind::Fixing,
            Kind::WaitingOnReviewer => pb::PrStageKind::WaitingOnReviewer,
            Kind::ChangesRequested => pb::PrStageKind::ChangesRequested,
            Kind::ApprovedChecksRunning => pb::PrStageKind::ApprovedChecksRunning,
            Kind::ApprovedChecksFailing => pb::PrStageKind::ApprovedChecksFailing,
            Kind::Approved => pb::PrStageKind::Approved,
            Kind::ApprovedConflicts => pb::PrStageKind::ApprovedConflicts,
            Kind::Queued => pb::PrStageKind::Queued,
            Kind::Merged => pb::PrStageKind::Merged,
            Kind::Closed => pb::PrStageKind::Closed,
            Kind::Unknown => pb::PrStageKind::Unknown,
        }) as i32,
        label: s.label(),
        reviewers: s.reviewers.clone(),
        queue_position: s.queue_position,
        pr_number: s.pr_number,
        pr_url: s.pr_url.clone(),
        unresolved_threads: s.unresolved_threads,
        read_at: s.read_at,
        checks: (match s.checks {
            CheckState::Unknown => pb::CheckState::Unknown,
            CheckState::Passing => pb::CheckState::Passing,
            CheckState::Failing => pb::CheckState::Failing,
            CheckState::Pending => pb::CheckState::Pending,
        }) as i32,
    }
}

/// Lay each lane's stage, and each of its cards', onto the wire plan built
/// from `plan`. Reads only what the runner has cached; never waits on `gh`.
pub fn annotate(svc: &Service, plan: &Plan, wire: &mut pb::Plan) {
    let mut reads: HashMap<Uuid, Reads> = HashMap::new();
    for (view, lane) in plan.lanes.iter().zip(wire.lanes.iter_mut()) {
        let workspace = view.lane.workspace_id;
        let read = match reads.entry(workspace) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Ok(ws) = svc.store.get_workspace(workspace) else { continue };
                e.insert(Reads::of(svc, ws.repository_id))
            }
        };
        let stages = lane_stages(view, &plan.cards, read);
        lane.stage = stages.lane.as_ref().map(pb_stage);
        for card in lane.cards.iter_mut() {
            let task = Uuid::from_slice(&card.task_id).ok();
            card.stage = task.and_then(|t| stages.cards.get(&t)).map(pb_stage);
        }
    }
}

#[cfg(test)]
#[path = "plan_stage_tests.rs"]
mod tests;

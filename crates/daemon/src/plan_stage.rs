//! A ticket's pull request, found and staged (ov-312, ov-872): which of the
//! repository's PRs belongs to which ticket on the line, and the stage of each
//! (`pr_stage`), laid onto the plan as it is read.
//!
//! One PR per ticket: the one whose head branch names the ticket's key
//! (`ov-312/pr-stage`), or whose title opens with it ("ov-312: tickets show
//! their PR stage"), else the PR on the ticket's own branch. A ticket carried
//! over from a lane of several cards shares that branch, so the branch says
//! nothing of it alone.
//!
//! Part of the plan layer: only `rpc_plan` and `pr_watch` call it.

use std::collections::HashMap;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::TaskStatus;
use farcooler_store::plan_read::{CardRef, LineView, Plan};
use farcooler_store::ticket_lines::Stage as LineStage;
use farcooler_store::ticket_work::WorkRole;
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

/// Whether a ticket's move takes it into review, a fix round or landing: its
/// new stage is one of those and is not the stage it was already in.
pub fn enters_active(was: LineStage, to: Option<LineStage>) -> bool {
    to.is_some_and(|to| to != was && phase_of(to, false).is_active())
}

/// Where a ticket is, as the stage reads it: building is a fix round while a
/// fix is open on it.
pub fn phase_of(stage: LineStage, fixing: bool) -> Phase {
    match stage {
        LineStage::Waiting => Phase::Queued,
        LineStage::Building if fixing => Phase::Fixing,
        LineStage::Building => Phase::Building,
        LineStage::Reviewing | LineStage::NeedsYou => Phase::Review,
        LineStage::Landing => Phase::Landing,
        LineStage::Landed => Phase::Landed,
        LineStage::Dropped => Phase::Dropped,
    }
}

/// A ticket on the line, as the stage reads it.
pub fn line_phase(line: &LineView) -> Phase {
    let fixing = line.work.iter().any(|w| w.role == WorkRole::Fix && w.ended_at.is_none());
    phase_of(line.line.stage, fixing)
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
/// that was. A revert is GitHub's own form, a title that starts `Revert "` or a
/// head branch `revert-<number>-...`; a card's own PR that happens to be titled
/// "Revert the inbox color" is that card's.
fn is_noise(p: &PrInfo) -> bool {
    let revert_branch = p.head_ref.strip_prefix("revert-").is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    p.is_fork || p.title.trim_start().starts_with("Revert \"") || revert_branch
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

/// The PR of a ticket: the one whose head branch names its key, or whose title
/// opens with it, else the one on its own branch when it has a branch of its
/// own (`alone`).
fn pr_of<'a>(prs: &'a [PrInfo], key: &str, branch: &str, alone: bool) -> Option<&'a PrInfo> {
    let mine = || prs.iter().filter(|p| !is_noise(p));
    best(mine().filter(|p| mentions(&p.head_ref, key) || title_starts_with(&p.title, key))).or_else(|| {
        if alone && !branch.is_empty() { best(mine().filter(|p| p.head_ref == branch)) } else { None }
    })
}

/// The pull request a ticket works, when the runner has read one.
pub fn pr_of_line<'a>(line: &LineView, cards: &[CardRef], reads: &'a Reads) -> Option<&'a PrInfo> {
    let card = cards.iter().find(|c| c.task_id == line.line.task_id)?;
    pr_of(&reads.prs, &card.key, &line.line.branch, line.line.same_branch.is_none())
}

/// One ticket's stage against one repository's reads. `None` for a ticket
/// not yet started, and one finished with no pull request.
pub fn line_stage(line: &LineView, cards: &[CardRef], reads: &Reads) -> Option<Stage> {
    let phase = line_phase(line);
    if phase == Phase::Queued {
        return None;
    }
    let card = cards.iter().find(|c| c.task_id == line.line.task_id)?;
    let pr = if reads.known { pr_of_line(line, cards, reads) } else { None };
    let finished = matches!(card.status, TaskStatus::Done | TaskStatus::Cancelled);
    if pr.is_none() && finished {
        return None;
    }
    if !reads.known {
        // Could not ask. Said where a pull request is expected, and silent
        // while the ticket is still building one.
        return phase.is_active().then(Stage::unknown);
    }
    pr_stage::derive(pr, phase)
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

/// Lay each ticket's stage onto the wire plan built from `plan`, in step with
/// `plan.lines`. Reads only what the runner has cached; never waits on `gh`.
pub fn annotate(svc: &Service, plan: &Plan, wire: &mut pb::Plan) {
    let mut reads: HashMap<Uuid, Reads> = HashMap::new();
    for (view, line) in plan.lines.iter().zip(wire.lines.iter_mut()) {
        let Ok(task) = svc.store.get_task(view.line.task_id) else { continue };
        let read = match reads.entry(task.workspace_id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let Ok(ws) = svc.store.get_workspace(task.workspace_id) else { continue };
                e.insert(Reads::of(svc, ws.repository_id))
            }
        };
        line.pr_stage = line_stage(view, &plan.cards, read).as_ref().map(pb_stage);
    }
}

/// Who the live tickets' ready pull requests wait on, and how long each
/// usually takes (`pr_timing`), laid onto `wire.review_waits`. The history is
/// every PR the runner has read of the board's repository, not only the
/// tickets' own.
pub fn annotate_waits(svc: &Service, workspace: Uuid, plan: &Plan, wire: &mut pb::Plan) {
    let Ok(ws) = svc.store.get_workspace(workspace) else { return };
    let reads = Reads::of(svc, ws.repository_id);
    if !reads.known {
        return;
    }
    let numbers: Vec<u32> = plan
        .lines
        .iter()
        .filter(|v| line_phase(v).is_active())
        .filter_map(|v| pr_of_line(v, &plan.cards, &reads))
        .map(|p| p.status.number)
        .collect();
    let waiting: Vec<&PrInfo> = reads.prs.iter().filter(|p| numbers.contains(&p.status.number)).collect();
    let mut all = crate::pr_timing::waits(&waiting, &reads.prs);
    all.sort_by(|a, b| b.prs.cmp(&a.prs).then_with(|| a.reviewer.to_lowercase().cmp(&b.reviewer.to_lowercase())));
    wire.review_waits = all
        .into_iter()
        .map(|w| pb::ReviewerWait { reviewer: w.reviewer, prs: w.prs, median_wait_ms: w.median_ms, samples: w.samples })
        .collect();
}

#[cfg(test)]
#[path = "plan_stage_tests.rs"]
mod tests;

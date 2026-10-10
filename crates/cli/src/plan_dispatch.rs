//! `farcooler plan lane dispatch` (ov-457): put an agent on a lane, not a card.
//!
//! A lane is the unit of execution: one agent, or a chain of them, in one
//! worktree on one branch, working one or more cards. This takes an existing
//! lane by name, or makes one from `--card`, and opens a pane on its first
//! card that isn't done, exactly as `task dispatch` would (same refusals,
//! warnings and board moves: it IS `task dispatch`), with the lane named on
//! the pane. The runner does the rest (`farcooler_daemon::lane_panes`): it
//! briefs the agent on every card on the lane, records the pane as the lane's
//! build agent, moves the lane to building, and exports `FARCOOLER_LANE`.
//! Nothing is recorded by hand.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use super::{Failed, cards_of, find_lane, is_live, send, unreadable};
use crate::tasks::{Board, DispatchLink, Lane, actor_for, board_in};

/// `plan lane dispatch`'s arguments.
#[derive(Debug, Clone, Args)]
pub(super) struct LaneDispatchArgs {
    /// The lane: an existing one, or one to make from `--card`.
    name: String,
    /// A card the lane works, `KEY` or `KEY:SLICE`, in order. Repeat for more.
    /// Added to an existing lane that doesn't have it yet.
    #[arg(long = "card", value_name = "KEY[:SLICE]")]
    cards: Vec<String>,
    /// The worktree to work in, by name or id.
    #[arg(long, required_unless_present = "new", conflicts_with = "new")]
    worktree: Option<String>,
    /// Make a new worktree with this name for the lane.
    #[arg(long, requires = "branch")]
    new: Option<String>,
    /// The new worktree's branch.
    #[arg(long, requires = "new")]
    branch: Option<String>,
    /// What the new worktree's branch starts from.
    #[arg(long, default_value = "HEAD")]
    base: String,
    /// The agent to start: claude, codex or cursor, with an optional `:model`.
    #[arg(long, default_value = "claude")]
    preset: String,
    /// One line: why a new lane is in the plan.
    #[arg(long)]
    reason: Option<String>,
    /// Dispatch even though the lane's first card already has an agent.
    #[arg(long)]
    again: bool,
}

/// `plan lane dispatch`, answering what to print.
pub(super) async fn dispatch<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    plan: &pb::Plan,
    args: LaneDispatchArgs,
    actor: &str,
    json: bool,
) -> Result<String, Failed> {
    if !link.capabilities().iter().any(|c| c == capability::TERMINAL_LANE) {
        return Err("This runner's Far Cooler can't open a pane for a lane yet. Update it and try again.".into());
    }
    let items = board_in(link, board, None, None).await?.items;
    let wanted = cards_of(link, board, &items, &args.cards).await?;
    let existing = find_lane(plan, &args.name).ok().filter(|l| is_live(l.state)).cloned();
    let lane = match existing {
        Some(lane) => {
            let add: Vec<pb::LaneCard> =
                wanted.into_iter().filter(|w| !lane.cards.iter().any(|c| c.task_id == w.task_id && c.slice == w.slice)).collect();
            if add.is_empty() {
                lane
            } else {
                let p = request::Payload::LaneCards(pb::LaneCards { lane_id: lane.id.clone(), add, remove: Vec::new(), actor: actor.into() });
                let result::Value::Lane(changed) = send(link, board, "lane.cards", p).await? else { return Err(unreadable()) };
                changed
            }
        }
        None if wanted.is_empty() => {
            return Err(format!("No lane here is called {:?}. Name its cards with --card to make it.", args.name).into());
        }
        None => {
            let p = request::Payload::LaneCreate(pb::LaneCreate {
                workspace_id: ws,
                name: args.name.clone(),
                reason: args.reason.clone().unwrap_or_default(),
                cards: wanted,
                actor: actor.into(),
                ..Default::default()
            });
            let result::Value::Lane(made) = send(link, board, "lane.create", p).await? else { return Err(unreadable()) };
            made
        }
    };
    let first = first_open(&lane, &items)
        .ok_or_else(|| -> Failed { format!("Every card on {} is done. Add one with --card.", lane.name).into() })?;
    let worktree = match (args.worktree, args.new, args.branch) {
        (Some(named), _, _) => Lane::Existing(named),
        (None, Some(name), Some(branch)) => Lane::New { name, branch, base: args.base },
        _ => return Err("Name a worktree with --worktree, or a new one with --new and --branch.".into()),
    };
    let asked = crate::tasks::Dispatch {
        task: first,
        lane: worktree,
        preset: &args.preset,
        actor: actor_for(Some(actor))?,
        again: args.again,
        plan_lane: Some(&lane.name),
    };
    let done = crate::tasks::dispatch(link, asked, &mut |w| eprintln!("{w}")).await?;
    let said = crate::tasks::dispatched_output(&first.key, &done, json);
    if json {
        let mut out: serde_json::Value = serde_json::from_str(&said).map_err(|_| unreadable())?;
        out["lane"] = serde_json::Value::String(lane.name.clone());
        return Ok(out.to_string());
    }
    Ok(format!("Lane {} is building, starting on {}.\n{said}", lane.name, first.key))
}

/// The lane's first card, in its order, that isn't done or cancelled.
fn first_open<'a>(lane: &pb::Lane, items: &'a [pb::Task]) -> Option<&'a pb::Task> {
    let open = |t: &&pb::Task| {
        !matches!(pb::TaskStatus::try_from(t.status), Ok(pb::TaskStatus::Done | pb::TaskStatus::Cancelled))
    };
    lane.cards.iter().find_map(|c| items.iter().filter(open).find(|t| t.id == c.task_id))
}

#[cfg(test)]
#[path = "plan_dispatch_tests.rs"]
mod tests;

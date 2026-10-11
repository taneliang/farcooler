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
//!
//! **Seen to start** (ov-493). Then it watches the pane for up to
//! `FIRST_TURN_LOOKS` seconds and says whether the agent began its first
//! turn, or what stopped it: claude's folder-trust prompt, which is the
//! owner's to answer, another prompt, or nothing yet. The runner watches on
//! past that (`farcooler_daemon::watch::lane_start`) and messages the
//! orchestrator if it never starts.

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
    let turn = if done.confirmed() { first_turn(link, done.worktree, done.terminal).await } else { FirstTurn::Unseen };
    if json {
        let mut out: serde_json::Value = serde_json::from_str(&said).map_err(|_| unreadable())?;
        out["lane"] = serde_json::Value::String(lane.name.clone());
        out["first_turn"] = serde_json::Value::String(turn.word().into());
        if let FirstTurn::Stopped(q) = &turn {
            out["stopped_on"] = q.clone().map_or(serde_json::Value::Null, serde_json::Value::String);
        }
        return Ok(out.to_string());
    }
    let started = turn.said(&done.terminal_short).map(|s| format!("\n{s}")).unwrap_or_default();
    Ok(format!("Lane {} is building, starting on {}.\n{said}{started}", lane.name, first.key))
}

/// How many times dispatch looks for the agent's first turn, a second
/// apart: a claude given its opening prompt is working within a few.
const FIRST_TURN_LOOKS: usize = 20;

/// What dispatch saw of the agent's first turn.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FirstTurn {
    /// It's working, or already finished a turn.
    Began,
    /// It stopped on a prompt first, quoted when the runner read one.
    Stopped(Option<String>),
    /// Not yet, when dispatch stopped looking.
    NotYet,
    /// Its pane wasn't confirmed, so it wasn't looked for.
    Unseen,
}

impl FirstTurn {
    /// `first_turn` under `--json`.
    fn word(&self) -> &'static str {
        match self {
            FirstTurn::Began => "began",
            FirstTurn::Stopped(_) => "stopped",
            FirstTurn::NotYet => "not_yet",
            FirstTurn::Unseen => "unseen",
        }
    }

    /// The line it adds, naming the pane by `short`.
    fn said(&self, short: &str) -> Option<String> {
        let screen = format!("`farcooler terminal screen {short}`");
        Some(match self {
            FirstTurn::Began => "  its agent began its first turn".to_string(),
            FirstTurn::Stopped(Some(q)) if q == farcooler_core::activity::trust::QUESTION => {
                "  its agent hasn't started: Claude Code is asking whether to trust its folder. that's the \
                 owner's call, never yours: tell them it's waiting in Needs You"
                    .to_string()
            }
            FirstTurn::Stopped(Some(q)) => format!("  its agent hasn't started: it's stopped on a prompt, “{q}”. see {screen}"),
            FirstTurn::Stopped(None) => format!("  its agent hasn't started: it's stopped on a prompt. see {screen}"),
            FirstTurn::NotYet => format!(
                "  its agent hasn't begun its first turn yet: see {screen}. the runner messages you if it doesn't start"
            ),
            FirstTurn::Unseen => return None,
        })
    }
}

/// Look at `terminal` in `worktree` up to `FIRST_TURN_LOOKS` times until its
/// agent is working, has finished a turn, or is stopped on a prompt.
async fn first_turn<L: DispatchLink>(link: &mut L, worktree: uuid::Uuid, terminal: uuid::Uuid) -> FirstTurn {
    for look in 0..FIRST_TURN_LOOKS {
        if look > 0 {
            link.pause(std::time::Duration::from_secs(1)).await;
        }
        let Ok(r) = link.call(crate::req_for("terminal.list", worktree)).await else { continue };
        let Some(result::Value::TerminalList(l)) = r.value else { continue };
        let Some(t) = l.items.iter().find(|t| t.id.as_ref() == terminal.as_bytes()) else { continue };
        match pb::AgentActivity::try_from(t.activity) {
            Ok(pb::AgentActivity::Working | pb::AgentActivity::Done) => return FirstTurn::Began,
            Ok(pb::AgentActivity::Blocked) => return FirstTurn::Stopped(t.blocked_question.clone()),
            _ => {}
        }
    }
    FirstTurn::NotYet
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

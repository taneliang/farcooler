//! `farcooler plan ticket`: one ticket on the line (ov-872).
//!
//! ```text
//! farcooler plan ticket set KEY [--stage S] [--reason R] [--branch B] [--pr N] [--sha SHA]
//!     [--stack-on KEY | --no-stack] [--budget N | --no-budget]
//! farcooler plan ticket show KEY
//! farcooler plan ticket pr-body KEY [--base REF] [--apply PR [--force]]
//! ```
//!
//! A ticket is the landing unit: its own branch, stacked on the ticket it
//! builds on, one pull request, landed alone once its parent has. `set` puts a
//! card on the line, waiting, when it isn't there yet (`line.set` does that in
//! the same write), and moves it along the line's arrows: waiting, building,
//! in review, landing, landed, with building again for a fix, and any open
//! ticket may be dropped or wait on you. A move the line refuses is said
//! before anything is sent, with where the ticket can go. A ticket that
//! reaches landing joins the board's collecting landing by itself.
//!
//! A child of `plan.rs`, whose tests (`plan_tests.rs`) hold it.

use clap::{Subcommand, ValueEnum};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::ticket_lines::Stage;

use super::overview_text::{agent_name, key_of, ticket_label, ticket_status};
use super::{
    Failed, age, ago, board_in, card, cost, events_json, events_of, find_line, line_json, pr_body, send, spend_words,
    stage, unreadable,
};
use crate::tasks::{Board, DispatchLink};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum StageArg {
    Waiting,
    Building,
    Reviewing,
    #[value(name = "needs_you", alias = "needs-you")]
    NeedsYou,
    Landing,
    Landed,
    Dropped,
}

impl StageArg {
    fn wire(self) -> pb::TicketStage {
        match self {
            StageArg::Waiting => pb::TicketStage::Waiting,
            StageArg::Building => pb::TicketStage::Building,
            StageArg::Reviewing => pb::TicketStage::Reviewing,
            StageArg::NeedsYou => pb::TicketStage::NeedsYou,
            StageArg::Landing => pb::TicketStage::Landing,
            StageArg::Landed => pb::TicketStage::Landed,
            StageArg::Dropped => pb::TicketStage::Dropped,
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub(super) enum TicketCmd {
    /// Put a ticket on the line, or change it there.
    ///
    /// A card not on the line goes on it, waiting. A move follows the line:
    /// waiting, building, reviewing, landing, landed, with building again for
    /// a fix. Any open ticket can be dropped, or wait on you (needs_you). A
    /// ticket that reaches landing joins the board's next landing by itself.
    Set {
        /// The card's key.
        key: String,
        /// Waiting, building, reviewing, needs_you, landing, landed or dropped.
        #[arg(long, value_enum)]
        stage: Option<StageArg>,
        /// One line: why it's in the plan, or why it's where it is now.
        #[arg(long)]
        reason: Option<String>,
        /// Its own branch.
        #[arg(long)]
        branch: Option<String>,
        /// Its pull request's number. 0 takes it away.
        #[arg(long, value_name = "N")]
        pr: Option<u32>,
        /// The commit it landed as.
        #[arg(long)]
        sha: Option<String>,
        /// The ticket its branch is stacked on, by key. That one lands first.
        #[arg(long, value_name = "KEY", conflicts_with = "no_stack")]
        stack_on: Option<String>,
        /// It isn't stacked on another ticket.
        #[arg(long)]
        no_stack: bool,
        /// A token budget: 5000000, 800k or 5M. The plan flags the ticket when it goes over.
        #[arg(long, value_parser = cost::parse_budget, conflicts_with = "no_budget")]
        budget: Option<u64>,
        /// Take the budget away.
        #[arg(long)]
        no_budget: bool,
    },
    /// One ticket: its stage, branch, stack, pull request, who worked it, its spend and timeline.
    Show {
        /// The card's key.
        key: String,
    },
    /// A pull request's description, from the card: what and why, acceptance, review, rulings.
    PrBody(pr_body::PrBodyArgs),
}

/// A stage as a sentence says it: "in review", "waiting on you".
pub(super) fn stage_phrase(stage: i32) -> &'static str {
    match pb::TicketStage::try_from(stage) {
        Ok(pb::TicketStage::Waiting) => "waiting",
        Ok(pb::TicketStage::Building) => "building",
        Ok(pb::TicketStage::Reviewing) => "in review",
        Ok(pb::TicketStage::NeedsYou) => "waiting on you",
        Ok(pb::TicketStage::Landing) => "landing",
        Ok(pb::TicketStage::Landed) => "landed",
        Ok(pb::TicketStage::Dropped) => "dropped",
        _ => "somewhere unknown",
    }
}

/// One `plan ticket` command on `board`, answering what to print.
pub(super) async fn ticket<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    cmd: TicketCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    let plan = super::get_plan(link, &ws, true).await?;
    match cmd {
        TicketCmd::Show { key } => {
            let (_, line) = find_line(&plan, &key)?;
            let events = events_of(link, board, pb::plan_events_request::Subject::TaskId(line.task_id.clone())).await?;
            Ok(if json {
                let mut out = line_json(&plan, line, &[]);
                out["events"] = events_json(&events);
                out.to_string()
            } else {
                ticket_text(&plan, line, &events, now)
            })
        }
        TicketCmd::PrBody(args) => pr_body::pr_body(link, board, &plan, args, json).await,
        TicketCmd::Set { key, stage, reason, branch, pr, sha, stack_on, no_stack, budget, no_budget } => {
            let budget = cost::asked(budget, no_budget);
            if budget.is_some() {
                cost::needs_cost(link)?;
            }
            let items = board_in(link, board, None, None).await?.items;
            let task = card(link, board, &items, &key).await?;
            let on = plan.lines.iter().find(|l| l.task_id == task.id);
            let nothing = stage.is_none()
                && reason.is_none()
                && branch.is_none()
                && pr.is_none()
                && sha.is_none()
                && stack_on.is_none()
                && !no_stack
                && budget.is_none();
            if on.is_some() && nothing {
                return Err(format!(
                    "{} is already on the line. Say what to change: --stage, --reason, --branch, --pr, --sha, --stack-on, --no-stack or --budget.",
                    task.key
                )
                .into());
            }
            if let (Some(line), Some(to)) = (on, stage) {
                refuse_a_move(&task.key, line, to)?;
            }
            let stack_parent = match (stack_on, no_stack) {
                (Some(parent), _) => Some(card(link, board, &items, &parent).await?.id),
                (None, true) => Some(bytes::Bytes::new()),
                (None, false) => None,
            };
            let p = request::Payload::LineSet(pb::TicketLineSet {
                task_id: task.id.clone(),
                stage: stage.map(|s| s.wire() as i32),
                reason,
                branch,
                worktree_id: None,
                worktree_path: None,
                stack_parent,
                pr_number: pr,
                landed_sha: sha,
                budget_tokens: budget,
                actor: actor.into(),
            });
            let result::Value::TicketLine(line) = send(link, board, "line.set", p).await? else {
                return Err(unreadable());
            };
            if json {
                return Ok(line_json(&plan, &line, &items).to_string());
            }
            let mut said = if on.is_some() {
                format!("{} is {}.", task.key, stage_phrase(line.stage))
            } else {
                format!("{} is on the line, {}.", task.key, stage_phrase(line.stage))
            };
            if let Some(n) = line.landing.filter(|_| line.stage == pb::TicketStage::Landing as i32) {
                said.push_str(&format!(" It's on Landing {n}."));
            }
            Ok(said)
        }
    }
}

/// A move the line refuses, said before sending with where the ticket is and
/// where it can go. The arrows are the store's (`Stage::can_move_to`), so
/// this can't disagree with the runner.
fn refuse_a_move(key: &str, line: &pb::TicketLine, to: StageArg) -> Result<(), Failed> {
    let word = |s: i32| Stage::parse(farcooler_client::plan_json::stage_word(s));
    let (Some(from), Some(target)) = (word(line.stage), word(to.wire() as i32)) else { return Ok(()) };
    if from.can_move_to(target) {
        return Ok(());
    }
    if from.is_closed() {
        return Err(format!("{key} is {}, so it takes no more moves.", stage_phrase(line.stage)).into());
    }
    let go: Vec<String> = Stage::WORDS
        .iter()
        .filter_map(|w| Stage::parse(w))
        .filter(|s| *s != from && from.can_move_to(*s))
        .map(|s| s.as_str().to_string())
        .collect();
    Err(match go.as_slice() {
        [] => format!("{key} is {}, so it takes no more moves.", stage_phrase(line.stage)),
        [only] => format!("{key} is {}. It can only move to {only}.", stage_phrase(line.stage)),
        [init @ .., last] => {
            format!("{key} is {}. It can move to {} or {last}.", stage_phrase(line.stage), init.join(", "))
        }
    }
    .into())
}

/// How a piece of work ended, as a person reads it; "working" while it's open.
fn verdict_words(w: &pb::TicketWork) -> &str {
    match w.verdict.as_str() {
        "" => "working",
        "handed_back" => "handed back",
        "task_closed" => "card closed",
        "relinked" => "moved to another ticket",
        "changes_asked" => "asked for changes",
        other => other,
    }
}

fn work_words(role: i32) -> &'static str {
    match pb::WorkRole::try_from(role) {
        Ok(pb::WorkRole::Build) => "building",
        Ok(pb::WorkRole::Fix) => "fixing",
        Ok(pb::WorkRole::Review) => "reviewing",
        Ok(pb::WorkRole::Land) => "landing",
        _ => "working",
    }
}

/// `plan ticket show`, in words.
pub(super) fn ticket_text(plan: &pb::Plan, l: &pb::TicketLine, events: &[pb::PlanEvent], now: i64) -> String {
    let mut out = vec![format!("{} · {}", ticket_label(plan, &l.task_id), ticket_status(plan, l, now))];
    if !l.reason.is_empty() {
        out.push(l.reason.clone());
    }
    let mut place: Vec<String> = Vec::new();
    if !l.branch.is_empty() {
        place.push(format!("Branch {}", l.branch));
    }
    if !l.worktree_path.is_empty() {
        place.push(l.worktree_path.clone());
    }
    if let Some(parent) = &l.stack_parent {
        place.push(format!("stacked on {}", key_of(plan, parent)));
    }
    if !place.is_empty() {
        out.push(place.join(" · "));
    }
    let pr = stage::pr_words(l.pr_stage.as_ref());
    if !pr.is_empty() {
        out.push(format!("Pull request  {pr}"));
    }
    if let Some(sha) = &l.landed_sha {
        out.push(format!("Landed as {sha}"));
    }
    if !l.work.is_empty() {
        out.push(String::new());
        out.push("Worked by".into());
        for w in &l.work {
            let span = w.ended_at.unwrap_or(now) - w.started_at;
            out.push(format!(
                "  {} · {}, {} · {}",
                agent_name(plan, &w.agent_id),
                work_words(w.role),
                age(span),
                verdict_words(w)
            ));
        }
    }
    let spend = l.spend.unwrap_or_default();
    out.push(String::new());
    out.push(format!("Spend  {}", spend_words(&spend)));
    out.extend(cost::budget_words(&spend, l.budget_tokens).map(|b| format!("Budget  {b}")));
    if !events.is_empty() {
        out.push(String::new());
        out.push("Timeline".into());
        out.extend(events.iter().map(|e| format!("  {}  {}", ago(now - e.at), e.body)));
    }
    out.join("\n")
}

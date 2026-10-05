//! `farcooler plan ruling`: decided for you (ov-304).
//!
//! A ruling is a reversible call the orchestrator made on the owner's behalf:
//! what was decided, why, what reversing costs, the cards and theme it
//! touches, and whether it stands. The orchestrator records one when it makes
//! the call, and confirms or reverses it when the owner says, citing its short
//! id. The apps show them and never edit them.
//!
//! ```text
//! farcooler plan ruling add "The inbox is amber." --why "..." --reversal "..." [--card ov-1]... [--theme NAME]
//! farcooler plan ruling set R-12 --state confirmed|reversed [--note "..."]
//! farcooler plan ruling keep R-12 | --all          (the owner's mark, ov-333)
//! farcooler plan ruling reverse R-12 [--sha <sha>] [--note "..."]
//! farcooler plan ruling list [--state open|kept|reversed]
//! ```
//!
//! The owner keeps a ruling, which changes nothing in the plan. Reversing is
//! the orchestrator's: the owner asks it in chat, it does the work, then marks
//! the ruling `reverse --sha <the commit>`. The words the owner reads are
//! open, kept and reversed; the store's `standing` and `confirmed` are the
//! same states, kept for the wire's sake.
//!
//! A child of `plan.rs`, whose naming and refusal helpers it shares, and
//! whose tests (`plan_tests.rs`) hold it. Behind
//! `board_rulings` as well as `board_plan`: a runner with the plan and no
//! rulings is told so before anything is sent.

use clap::{Subcommand, ValueEnum};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};

use super::{Failed, Keys, ago, board_in, expect_value, find_theme, get_plan, id_text, ids_of, refused_here, unreadable};
use crate::tasks::{Board, DispatchLink, Refused};
use crate::{req_for, with};

/// What a runner with the plan and no rulings is told.
const NEEDS_UPDATE: &str = "This runner needs an update to keep rulings.";
/// What a runner with rulings and none of the owner's actions is told.
const NEEDS_ACTIONS: &str = "This runner needs an update to keep or reverse rulings.";

#[derive(Debug, Clone, Subcommand)]
pub(super) enum RulingCmd {
    /// Record a call made for the owner. It stands until they say otherwise.
    Add {
        /// What was decided, in one line.
        decision: String,
        /// Why, in one or two lines.
        #[arg(long)]
        why: String,
        /// What reversing it would cost.
        #[arg(long)]
        reversal: String,
        /// A card it touches, by key. Repeat for more.
        #[arg(long = "card", value_name = "KEY")]
        cards: Vec<String>,
        /// The theme it touches, by name or short id.
        #[arg(long)]
        theme: Option<String>,
    },
    /// Mark a ruling reversed, or kept (the owner's own call: `keep` is the
    /// usual word). Confirming as anyone but the owner is refused.
    Set {
        /// The ruling's short id: R-12, or 12.
        id: String,
        #[arg(long, value_enum)]
        state: RulingStateArg,
        /// The owner's words, or why it moved.
        #[arg(long)]
        note: Option<String>,
    },
    /// The owner keeps a ruling, or every open one with `--all`. The
    /// owner's own mark: the orchestrator doesn't keep rulings for them.
    Keep {
        /// The ruling's short id: R-12, or 12.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        id: Option<String>,
        /// Keep every open ruling on this board.
        #[arg(long)]
        all: bool,
    },
    /// Mark a ruling reversed once the work is done, with the commit that did
    /// it. The orchestrator's verb: reversing itself is a request in chat.
    Reverse {
        /// The ruling's short id: R-12, or 12.
        id: String,
        /// The commit that reversed it. Leave it out when there isn't one.
        #[arg(long)]
        sha: Option<String>,
        /// Anything worth knowing about how it went.
        #[arg(long)]
        note: Option<String>,
    },
    /// Every ruling on this board: the open ones first, newest first.
    List {
        /// Only the rulings in this state.
        #[arg(long, value_enum)]
        state: Option<RulingListState>,
    },
}

/// The states `list --state` filters by, in the words the owner reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum RulingListState {
    Open,
    Kept,
    Reversed,
}

impl RulingListState {
    fn holds(self, r: &pb::BoardRuling) -> bool {
        match self {
            RulingListState::Open => is_standing(r),
            RulingListState::Kept => r.state == pb::BoardRulingState::Confirmed as i32,
            RulingListState::Reversed => r.state == pb::BoardRulingState::Reversed as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum RulingStateArg {
    Confirmed,
    Reversed,
}

/// This module's sentences for the words the runner refuses a ruling with.
pub(super) fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "decision" => "Say what was decided, in one line of up to 300 characters.",
        "why" => "Say why with --why, in one or two lines of up to 600 characters.",
        "reversal" => "Say what reversing it costs with --reversal, in one line of up to 300 characters.",
        "note" => "That note is too long for one line. Shorten it.",
        "ruling_state" => "A ruling can't make that move. An open ruling is kept or reversed, a kept one can still be reversed, and a reversed one is final.",
        "reversed_sha" => "Give the commit that reversed it with --sha: 4 to 64 hex digits.",
        _ => return None,
    })
}

fn needs_rulings<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_RULINGS) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

fn needs_actions<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_RULING_ACTIONS) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_ACTIONS.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

async fn call<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload, needs: &str) -> Result<result::Value, Failed> {
    let mut r = with(req_for(method, board.repository), p);
    r.required_capabilities.push(needs.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, "The runner couldn't record that ruling. Try again."))?;
    expect_value(answer.value)
}

async fn send<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload) -> Result<pb::BoardRuling, Failed> {
    let needs = if method == "ruling.set" || method == "ruling.add" { capability::BOARD_RULINGS } else { capability::BOARD_RULING_ACTIONS };
    match call(link, board, method, p, needs).await? {
        result::Value::BoardRuling(r) => Ok(r),
        _ => Err(unreadable()),
    }
}

/// The ruling `id` names on this board, or the sentence saying there's none.
fn find_ruling<'a>(plan: &'a pb::Plan, id: &str) -> Result<&'a pb::BoardRuling, Failed> {
    number_of(id)
        .and_then(|n| plan.rulings.iter().find(|r| r.number == n))
        .ok_or_else(|| -> Failed { format!("There's no ruling {id} on this board. `plan ruling list` shows them.").into() })
}

/// `R-12`, `r-12`, `R12`, `#12` or `12`, as the number.
fn number_of(id: &str) -> Option<u32> {
    let id = id.trim().trim_start_matches('#');
    let id = id.strip_prefix(['R', 'r']).unwrap_or(id);
    id.strip_prefix('-').unwrap_or(id).parse().ok()
}

/// One `plan ruling` command on `board`, answering what to print.
pub(super) async fn ruling<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    cmd: RulingCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_rulings(link)?;
    if matches!(cmd, RulingCmd::Keep { .. } | RulingCmd::Reverse { .. }) {
        needs_actions(link)?;
    }
    // Keep is the owner's mark, so the owner's actor: the orchestrator is
    // asked in chat for what changes the plan, and keeps nothing for them.
    let keeps = matches!(cmd, RulingCmd::Keep { .. })
        || matches!(cmd, RulingCmd::Set { state: RulingStateArg::Confirmed, .. });
    if keeps && actor != "user" {
        return Err("Keeping a ruling is the owner's call. Ask them to keep it from Decided For You.".into());
    }
    let plan = get_plan(link, &ws, true).await?;
    let mut keys = Keys::of_plan(&plan);
    match cmd {
        RulingCmd::List { state } => {
            let shown: Vec<&pb::BoardRuling> = plan.rulings.iter().filter(|r| state.is_none_or(|s| s.holds(r))).collect();
            Ok(if json {
                json!({ "rulings": shown.iter().map(|r| ruling_json(&plan, r, &keys)).collect::<Vec<_>>() }).to_string()
            } else {
                list_text(&plan, &shown, &keys, now, state)
            })
        }
        RulingCmd::Keep { id, all } => {
            if all {
                let p = request::Payload::RulingKeepAll(pb::RulingKeepAll { workspace_id: ws, actor: actor.into() });
                let kept = match call(link, board, "ruling.keep_all", p, capability::BOARD_RULING_ACTIONS).await? {
                    result::Value::RulingsKept(k) => k.rulings,
                    _ => return Err(unreadable()),
                };
                return Ok(if json {
                    json!({ "kept": kept.iter().map(|r| ruling_json(&plan, r, &keys)).collect::<Vec<_>>() }).to_string()
                } else if kept.is_empty() {
                    "No ruling is open.".to_string()
                } else {
                    format!("Kept {}.", kept.iter().map(|r| format!("R-{}", r.number)).collect::<Vec<_>>().join(", "))
                });
            }
            let found = find_ruling(&plan, id.as_deref().unwrap_or_default())?;
            if found.state == pb::BoardRulingState::Confirmed as i32 {
                return Ok(format!("R-{} is already kept.", found.number));
            }
            let p = request::Payload::RulingSet(pb::RulingSet {
                ruling_id: found.id.clone(),
                state: pb::BoardRulingState::Confirmed as i32,
                note: None,
                actor: actor.into(),
                sha: None,
            });
            let set = send(link, board, "ruling.set", p).await?;
            Ok(if json { ruling_json(&plan, &set, &keys).to_string() } else { format!("Kept R-{}.", set.number) })
        }
        RulingCmd::Reverse { id, sha, note } => {
            let found = find_ruling(&plan, &id)?;
            let p = request::Payload::RulingSet(pb::RulingSet {
                ruling_id: found.id.clone(),
                state: pb::BoardRulingState::Reversed as i32,
                note,
                actor: actor.into(),
                sha,
            });
            let set = send(link, board, "ruling.set", p).await?;
            Ok(if json {
                ruling_json(&plan, &set, &keys).to_string()
            } else {
                match set.reversed_sha.as_deref() {
                    Some(sha) => format!("R-{} is reversed in {sha}.", set.number),
                    None => format!("R-{} is reversed.", set.number),
                }
            })
        }
        RulingCmd::Add { decision, why, reversal, cards, theme } => {
            let items = board_in(link, board, None, None).await?.items;
            keys.extend(&items);
            let task_ids = ids_of(link, board, &items, &cards).await?;
            let theme_id = match theme {
                Some(name) => Some(find_theme(&plan, &name)?.theme.as_ref().map(|t| t.id.clone()).unwrap_or_default()),
                None => None,
            };
            let p = request::Payload::RulingAdd(pb::RulingAdd {
                workspace_id: ws,
                decision,
                why,
                reversal,
                task_ids,
                theme_id,
                actor: actor.into(),
            });
            let made = send(link, board, "ruling.add", p).await?;
            Ok(if json { ruling_json(&plan, &made, &keys).to_string() } else { format!("Recorded R-{}: {}", made.number, made.decision) })
        }
        RulingCmd::Set { id, state, note } => {
            let found = find_ruling(&plan, &id)?;
            let state = match state {
                RulingStateArg::Confirmed => pb::BoardRulingState::Confirmed,
                RulingStateArg::Reversed => pb::BoardRulingState::Reversed,
            };
            let p = request::Payload::RulingSet(pb::RulingSet {
                ruling_id: found.id.clone(),
                state: state as i32,
                note,
                actor: actor.into(),
                sha: None,
            });
            let set = send(link, board, "ruling.set", p).await?;
            Ok(if json {
                ruling_json(&plan, &set, &keys).to_string()
            } else {
                format!("R-{} is {}.", set.number, state_word(set.state))
            })
        }
    }
}

fn state_word(state: i32) -> &'static str {
    match pb::BoardRulingState::try_from(state) {
        Ok(pb::BoardRulingState::Standing) => "standing",
        Ok(pb::BoardRulingState::Confirmed) => "confirmed",
        Ok(pb::BoardRulingState::Reversed) => "reversed",
        _ => "unknown",
    }
}

fn is_standing(r: &pb::BoardRuling) -> bool {
    r.state == pb::BoardRulingState::Standing as i32
}

fn theme_name(plan: &pb::Plan, r: &pb::BoardRuling) -> Option<String> {
    let id = r.theme_id.as_ref()?;
    plan.themes.iter().filter_map(|v| v.theme.as_ref()).find(|t| t.id == *id).map(|t| t.name.clone())
}

/// A ruling as `--json` prints it: the shape `crates/client/src/plan_json.rs`
/// gives the phones, held to `test/fixtures/plan.json` by both.
pub(super) fn ruling_json(plan: &pb::Plan, r: &pb::BoardRuling, keys: &Keys) -> Value {
    json!({
        "id": id_text(&r.id),
        "short": format!("R-{}", r.number),
        "number": r.number,
        "decision": r.decision,
        "why": r.why,
        "reversal": r.reversal,
        "cards": r.task_ids.iter().enumerate().map(|(i, id)| json!({ "task": id_text(id), "key": key_of(r, i, keys) })).collect::<Vec<_>>(),
        "theme_id": r.theme_id.as_deref().map(id_text),
        "theme": theme_name(plan, r).unwrap_or_default(),
        "state": state_word(r.state),
        "reversed_sha": r.reversed_sha,
        "note": r.note,
        "actor": r.actor,
        "created_at": r.created_at,
        "settled_by": r.settled_by,
        "settled_at": r.settled_at,
    })
}

/// The key of a ruling's `i`th card: its own, else what the command read.
fn key_of(r: &pb::BoardRuling, i: usize, keys: &Keys) -> String {
    r.task_keys.get(i).cloned().unwrap_or_else(|| keys.of(&r.task_ids[i]))
}

/// "ov-1, ov-2 · Visual language · by manager 2 h ago": what a ruling touches,
/// and who made it when.
fn touches(plan: &pb::Plan, r: &pb::BoardRuling, keys: &Keys, now: i64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !r.task_ids.is_empty() {
        parts.push((0..r.task_ids.len()).map(|i| key_of(r, i, keys)).collect::<Vec<_>>().join(", "));
    }
    if let Some(theme) = theme_name(plan, r) {
        parts.push(theme);
    }
    parts.push(format!("by {} {}", r.actor, ago(now - r.created_at)));
    parts.join(" · ")
}

/// `plan ruling list`: the open ones first, with why and what reversing costs;
/// then the past decisions, a line each. `only` is the filter, said when it
/// leaves nothing.
fn list_text(plan: &pb::Plan, shown: &[&pb::BoardRuling], keys: &Keys, now: i64, only: Option<RulingListState>) -> String {
    if plan.rulings.is_empty() {
        return "No rulings yet. Record one with `farcooler plan ruling add`.".into();
    }
    if shown.is_empty() {
        let word = match only {
            Some(RulingListState::Kept) => "kept",
            Some(RulingListState::Reversed) => "reversed",
            _ => "open",
        };
        return format!("No {word} rulings.");
    }
    let mut out = Vec::new();
    let open: Vec<&&pb::BoardRuling> = shown.iter().filter(|r| is_standing(r)).collect();
    if !open.is_empty() {
        out.push("Decided for you".to_string());
        for r in open {
            out.push(format!("  R-{:<4} {}", r.number, r.decision));
            out.push(format!("         Why: {}", r.why));
            out.push(format!("         Reversing: {}", r.reversal));
            out.push(format!("         {}", touches(plan, r, keys, now)));
        }
    }
    let past: Vec<&&pb::BoardRuling> = shown.iter().filter(|r| !is_standing(r)).collect();
    if !past.is_empty() {
        out.push("Past decisions".to_string());
        for r in past {
            let word = match (r.state == pb::BoardRulingState::Reversed as i32, r.reversed_sha.as_deref()) {
                (true, Some(sha)) => format!("Reversed in {sha}"),
                (true, None) => "Reversed".to_string(),
                (false, _) => "Kept".to_string(),
            };
            out.push(format!("  R-{:<4} {word} · {}", r.number, r.decision));
            if !r.note.is_empty() {
                out.push(format!("         Note: {}", r.note));
            }
        }
    }
    out.join("\n")
}

/// The overview's lines for rulings: the standing ones, a line each, so the
/// orchestrator sees what the owner's Decided For You shows.
pub(super) fn overview_lines(plan: &pb::Plan) -> Vec<String> {
    let standing: Vec<&pb::BoardRuling> = plan.rulings.iter().filter(|r| is_standing(r)).collect();
    if standing.is_empty() {
        return Vec::new();
    }
    let mut out = vec!["Decided for you".to_string()];
    out.extend(standing.iter().map(|r| format!("  R-{:<4} {}", r.number, r.decision)));
    out
}

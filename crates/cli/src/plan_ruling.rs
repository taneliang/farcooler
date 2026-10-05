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
//! farcooler plan ruling list
//! ```
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
    /// Confirm or reverse a ruling, as the owner said.
    Set {
        /// The ruling's short id: R-12, or 12.
        id: String,
        #[arg(long, value_enum)]
        state: RulingStateArg,
        /// The owner's words, or why it moved.
        #[arg(long)]
        note: Option<String>,
    },
    /// Every ruling on this board: the standing ones first, newest first.
    List,
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
        "ruling_state" => "A ruling can't make that move. A standing ruling is confirmed or reversed, a confirmed one can still be reversed, and a reversed one is final.",
        _ => return None,
    })
}

fn needs_rulings<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_RULINGS) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

async fn send<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload) -> Result<pb::BoardRuling, Failed> {
    let mut r = with(req_for(method, board.repository), p);
    r.required_capabilities.push(capability::BOARD_RULINGS.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, "The runner couldn't record that ruling. Try again."))?;
    match expect_value(answer.value)? {
        result::Value::BoardRuling(r) => Ok(r),
        _ => Err(unreadable()),
    }
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
    let plan = get_plan(link, &ws, true).await?;
    let mut keys = Keys::of_plan(&plan);
    match cmd {
        RulingCmd::List => Ok(if json {
            json!({ "rulings": plan.rulings.iter().map(|r| ruling_json(&plan, r, &keys)).collect::<Vec<_>>() }).to_string()
        } else {
            list_text(&plan, &keys, now)
        }),
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
            let found = number_of(&id)
                .and_then(|n| plan.rulings.iter().find(|r| r.number == n))
                .ok_or_else(|| -> Failed { format!("There's no ruling {id} on this board. `plan ruling list` shows them.").into() })?;
            let state = match state {
                RulingStateArg::Confirmed => pb::BoardRulingState::Confirmed,
                RulingStateArg::Reversed => pb::BoardRulingState::Reversed,
            };
            let p = request::Payload::RulingSet(pb::RulingSet {
                ruling_id: found.id.clone(),
                state: state as i32,
                note,
                actor: actor.into(),
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

/// `plan ruling list`: standing first, with why and what reversing costs;
/// then the settled ones, a line each.
fn list_text(plan: &pb::Plan, keys: &Keys, now: i64) -> String {
    if plan.rulings.is_empty() {
        return "No rulings yet. Record one with `farcooler plan ruling add`.".into();
    }
    let mut out = Vec::new();
    let standing: Vec<&pb::BoardRuling> = plan.rulings.iter().filter(|r| is_standing(r)).collect();
    if !standing.is_empty() {
        out.push("Decided for you".to_string());
        for r in standing {
            out.push(format!("  R-{:<4} {}", r.number, r.decision));
            out.push(format!("         Why: {}", r.why));
            out.push(format!("         Reversing: {}", r.reversal));
            out.push(format!("         {}", touches(plan, r, keys, now)));
        }
    }
    let settled: Vec<&pb::BoardRuling> = plan.rulings.iter().filter(|r| !is_standing(r)).collect();
    if !settled.is_empty() {
        out.push("Settled".to_string());
        for r in settled {
            let word = if r.state == pb::BoardRulingState::Reversed as i32 { "Reversed" } else { "Confirmed" };
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

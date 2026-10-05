//! `farcooler plan train`: trains (ov-309).
//!
//! A train is a batch of lanes landing together: a name, the base it was cut
//! from, the SHA it pushed, and where it stands. The orchestrator starts one
//! when it picks lanes into an integration branch, and records each move; the
//! runner reads the pushed SHA's CI through `gh` and moves a pushed train to
//! green or red on its own. This replaces the hand-kept trains page.
//!
//! ```text
//! farcooler plan train start integ-14 --lane mac-ux --lane phones [--base origin/main]
//! farcooler plan train set integ-14 [--state gating|...] [--sha 1a1b3275] [--add-lane L] [--remove-lane L]
//! farcooler plan train list
//! ```
//!
//! A child of `plan.rs`, whose naming and refusal helpers it shares, and whose
//! tests (`plan_tests.rs`) hold it. Behind `board_trains` as well as
//! `board_plan`: a runner with the plan and no trains is told so before
//! anything is sent.

use clap::{Subcommand, ValueEnum};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use serde_json::{Value, json};

use super::{Failed, Keys, ago, expect_value, find_lane, get_plan, id_text, refused_here, unreadable};
use crate::ci_words::{read_for, status_key, summary, train_state_key, train_state_word};
use crate::short_bytes;
use crate::tasks::{Board, DispatchLink, Refused};
use crate::{req_for, with};

/// What a runner with the plan and no trains is told.
const NEEDS_UPDATE: &str = "This runner needs an update to keep trains.";

#[derive(Debug, Clone, Subcommand)]
pub(super) enum TrainCmd {
    /// Start a train, integrating, with these lanes on it.
    Start {
        /// Its name, one word: integ-14.
        name: String,
        /// A lane to put on it, by name. Repeat for more.
        #[arg(long = "lane", value_name = "LANE")]
        lanes: Vec<String>,
        /// What it's cut from: origin/main, or a SHA.
        #[arg(long, default_value = "")]
        base: String,
    },
    /// Move a train, record what it pushed, or put lanes on or off it.
    ///
    /// A new --sha without --state moves it to pushed. From there the runner
    /// moves it to green or red as its CI says.
    Set {
        /// The train's name.
        name: String,
        #[arg(long, value_enum)]
        state: Option<TrainStateArg>,
        /// The SHA it pushed.
        #[arg(long)]
        sha: Option<String>,
        /// What it's cut from now.
        #[arg(long)]
        base: Option<String>,
        /// A lane to put on it. Repeat for more.
        #[arg(long = "add-lane", value_name = "LANE")]
        add: Vec<String>,
        /// A lane to take off it. Repeat for more.
        #[arg(long = "remove-lane", value_name = "LANE")]
        remove: Vec<String>,
    },
    /// Every train on this board: the ones not landed first, with their CI.
    List,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum TrainStateArg {
    Integrating,
    Gating,
    Pushed,
    Green,
    Red,
    Landed,
    Dropped,
}

/// This module's sentences for the words the runner refuses a train with.
/// Asked before `plan.rs`'s, whose `sha` and `name` mean a lane's.
pub(super) fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "name" => "A train's name is one word with no spaces, like integ-14.",
        "name_taken" => "That train name is already used on this board. Trains keep their names, so pick the next one.",
        "base" => "That base is too long. Name a ref or a SHA.",
        "sha" => "Give the SHA it pushed with --sha: 7 to 40 hex digits. A train is pushed, green or red only once it has one.",
        "train_settled" => "That train has landed or been dropped, so it takes no more moves or SHAs.",
        _ => return None,
    })
}

fn needs_trains<L: DispatchLink>(link: &L) -> Result<(), Failed> {
    if link.capabilities().iter().any(|c| c == capability::BOARD_TRAINS) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

async fn send<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload) -> Result<pb::BoardTrain, Failed> {
    let mut r = with(req_for(method, board.repository), p);
    r.required_capabilities.push(capability::BOARD_TRAINS.to_string());
    let answer = link.call(r).await.map_err(|e| match &e {
        ClientError::Daemon { code, what, .. } if said_here(what).is_some() => {
            Box::new(Refused::naming(said_here(what).unwrap_or_default().to_string(), *code, what.clone())) as Failed
        }
        _ => refused_here(e, "The runner couldn't record that train. Try again."),
    })?;
    match expect_value(answer.value)? {
        result::Value::BoardTrain(t) => Ok(t),
        _ => Err(unreadable()),
    }
}

fn lane_ids(plan: &pb::Plan, names: &[String]) -> Result<Vec<bytes::Bytes>, Failed> {
    names.iter().map(|n| find_lane(plan, n).map(|l| l.id.clone())).collect()
}

fn state_of(arg: TrainStateArg) -> pb::BoardTrainState {
    match arg {
        TrainStateArg::Integrating => pb::BoardTrainState::Integrating,
        TrainStateArg::Gating => pb::BoardTrainState::Gating,
        TrainStateArg::Pushed => pb::BoardTrainState::Pushed,
        TrainStateArg::Green => pb::BoardTrainState::Green,
        TrainStateArg::Red => pb::BoardTrainState::Red,
        TrainStateArg::Landed => pb::BoardTrainState::Landed,
        TrainStateArg::Dropped => pb::BoardTrainState::Dropped,
    }
}

/// One `plan train` command on `board`, answering what to print.
pub(super) async fn train<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    cmd: TrainCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_trains(link)?;
    let plan = get_plan(link, &ws, true).await?;
    let keys = Keys::of_plan(&plan);
    let said = |plan: &pb::Plan, t: &pb::BoardTrain| -> String {
        if json { train_json(plan, t, &keys).to_string() } else { train_text(plan, t, now).join("\n") }
    };
    match cmd {
        TrainCmd::List => Ok(if json {
            json!({ "trains": plan.trains.iter().map(|t| train_json(&plan, t, &keys)).collect::<Vec<_>>() }).to_string()
        } else if plan.trains.is_empty() {
            "No trains yet. Start one with `farcooler plan train start`.".into()
        } else {
            plan.trains.iter().flat_map(|t| train_text(&plan, t, now)).collect::<Vec<_>>().join("\n")
        }),
        TrainCmd::Start { name, lanes, base } => {
            let p = request::Payload::TrainStart(pb::TrainStart {
                workspace_id: ws,
                name,
                base,
                lane_ids: lane_ids(&plan, &lanes)?,
                actor: actor.into(),
            });
            let made = send(link, board, "train.start", p).await?;
            Ok(said(&plan, &made))
        }
        TrainCmd::Set { name, state, sha, base, add, remove } => {
            let found = find_train(&plan, &name)?;
            let p = request::Payload::TrainSet(pb::TrainSet {
                train_id: found.id.clone(),
                state: state.map_or(pb::BoardTrainState::Unspecified, state_of) as i32,
                base,
                sha,
                add_lane_ids: lane_ids(&plan, &add)?,
                remove_lane_ids: lane_ids(&plan, &remove)?,
                actor: actor.into(),
            });
            let set = send(link, board, "train.set", p).await?;
            Ok(said(&plan, &set))
        }
    }
}

/// A train by name, ignoring case.
fn find_train<'a>(plan: &'a pb::Plan, name: &str) -> Result<&'a pb::BoardTrain, Failed> {
    let wanted = name.trim().to_lowercase();
    plan.trains
        .iter()
        .find(|t| t.name.to_lowercase() == wanted)
        .ok_or_else(|| format!("No train here is called {name:?}. `plan train list` shows them.").into())
}

/// A lane's name by id, or its short id for one the plan no longer lists.
fn lane_name(plan: &pb::Plan, id: &[u8]) -> String {
    plan.lanes.iter().find(|l| l.id == id).map_or_else(|| short_bytes(id), |l| l.name.clone())
}

/// A train's CI read, once the runner has one.
pub(super) fn ci_of<'a>(plan: &'a pb::Plan, t: &pb::BoardTrain) -> Option<&'a pb::BoardCiRead> {
    if t.ci_subject.is_empty() { None } else { read_for(&plan.ci, &t.ci_subject) }
}

/// "integ-14 · Red · 1a1b3275 · CI Failed · 2 of 15 jobs failed": the train's
/// line, as the Now section heads its group with it.
pub(super) fn train_line(plan: &pb::Plan, t: &pb::BoardTrain) -> String {
    let mut parts = vec![t.name.clone(), train_state_word(t.state).to_string()];
    if let Some(sha) = &t.pushed_sha {
        parts.push(sha.chars().take(8).collect());
    }
    if let Some(read) = ci_of(plan, t) {
        // "CI unknown" already says CI (review train-1005c L2).
        let said = summary(read);
        parts.push(if said.starts_with("CI ") { said } else { format!("CI {said}") });
        parts.extend(crate::ci_words::stale(read, plan.now_ms));
    } else if t.pushed_sha.is_some() && !is_settled(t) {
        parts.push("CI not read yet".into());
    }
    parts.join(" · ")
}

fn is_settled(t: &pb::BoardTrain) -> bool {
    t.state == pb::BoardTrainState::Landed as i32 || t.state == pb::BoardTrainState::Dropped as i32
}

/// `plan train list`'s lines for one train: its line, its base and lanes, and
/// any job that didn't pass.
fn train_text(plan: &pb::Plan, t: &pb::BoardTrain, now: i64) -> Vec<String> {
    let mut out = vec![train_line(plan, t)];
    let mut facts = Vec::new();
    if !t.base.is_empty() {
        facts.push(format!("cut from {}", t.base));
    }
    facts.push(format!("{} for {}", train_state_word(t.state).to_lowercase(), super::age(now - t.state_since)));
    out.push(format!("  {}", facts.join(" · ")));
    if !t.lane_ids.is_empty() {
        let lanes: Vec<String> = t.lane_ids.iter().map(|id| lane_name(plan, id)).collect();
        out.push(format!("  Lanes: {}", lanes.join(", ")));
    }
    if let Some(read) = ci_of(plan, t) {
        for job in read.jobs.iter().filter(|j| matches!(j.state.as_str(), "failed" | "canceled")) {
            let word = if job.state == "failed" { "Failed" } else { "Canceled" };
            out.push(format!("  {word}: {}", job.name));
        }
        if !read.url.is_empty() {
            out.push(format!("  {} · read {}", read.url, ago(now - read.fetched_at)));
        }
    }
    out
}

/// A train as `--json` prints it: the shape `crates/client/src/plan_json.rs`
/// gives the phones, held to `test/fixtures/plan.json` by both.
pub(super) fn train_json(plan: &pb::Plan, t: &pb::BoardTrain, _keys: &Keys) -> Value {
    json!({
        "id": id_text(&t.id),
        "short": short_bytes(&t.id),
        "name": t.name,
        "base": t.base,
        "pushed_sha": t.pushed_sha,
        "state": train_state_key(t.state),
        "state_since": t.state_since,
        "actor": t.actor,
        "created_at": t.created_at,
        "landed_at": t.landed_at,
        "lanes": t.lane_ids.iter().map(|id| json!({ "lane": id_text(id), "name": lane_name(plan, id) })).collect::<Vec<_>>(),
        "ci_subject": t.ci_subject,
    })
}

/// A CI read as `--json` prints it, the same shape as the client's.
pub(super) fn ci_json(r: &pb::BoardCiRead) -> Value {
    json!({
        "subject": r.subject,
        "sha": r.sha,
        "status": status_key(r.status),
        "url": r.url,
        "jobs": r.jobs.iter().map(|j| json!({ "name": j.name, "state": j.state, "url": j.url })).collect::<Vec<_>>(),
        "fetched_at": r.fetched_at,
        "changed_at": r.changed_at,
        "asked_at": r.asked_at,
    })
}

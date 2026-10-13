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
//!     [--title T] [--agent ID [--card KEY]] [--force]
//! farcooler plan train set integ-14 [--state gating|...] [--sha 1a1b3275] [--add-lane L] [--remove-lane L]
//!     [--title T] [--agent ID [--ended]] [--card KEY] [--force]
//! farcooler plan train list
//! ```
//!
//! Only a lane that is ready to land (state `landing`) boards a train. A lane
//! still in review boards with `--force`, which moves it to landing first; a
//! queued, building or fixing lane never does (ov-503). The runner enforces the
//! same rule, so an older CLI can't board one either.
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

use super::{Failed, Keys, ago, card, expect_value, find_lane, get_plan, id_text, refused_here, send as send_plan, spend_words, state_word, unreadable};
use crate::ci_words::{read_for, status_key, summary, train_phase_key, train_phase_word, train_state_key, train_state_word};
use crate::short_bytes;
use crate::tasks::{Board, DispatchLink, Refused};
use crate::{req_for, with};

/// What a runner with the plan and no trains is told.
const NEEDS_UPDATE: &str = "This runner needs an update to keep trains.";

#[derive(Debug, Clone, Subcommand)]
pub(super) enum TrainCmd {
    /// Start a train, integrating, with these lanes on it.
    ///
    /// Only lanes that are ready to land (state landing) board a train. A lane
    /// that is queued, building or fixing is refused; one still in review
    /// boards with --force.
    Start {
        /// Its name, one word: integ-14.
        name: String,
        /// A lane to put on it, by name. Repeat for more.
        #[arg(long = "lane", value_name = "LANE")]
        lanes: Vec<String>,
        /// What it's cut from: origin/main, or a SHA.
        #[arg(long, default_value = "")]
        base: String,
        /// What it carries, for people. Without one it reads "Train 14".
        #[arg(long)]
        title: Option<String>,
        /// Board a lane that is still in review by moving it to landing first.
        #[arg(long, requires = "lanes")]
        force: bool,
        #[command(flatten)]
        integrator: Integrator,
    },
    /// Move a train, record what it pushed, or put lanes on or off it.
    ///
    /// A new --sha without --state moves it to pushed. From there the runner
    /// moves it to green or red as its CI says. Lanes you add follow the same
    /// rule as at start: ready to land, or in review with --force. Landing a
    /// train lands every open lane on it.
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
        /// Its title for people. An empty one takes it away.
        #[arg(long)]
        title: Option<String>,
        /// Board an added lane that is still in review by moving it to landing first.
        #[arg(long, requires = "add")]
        force: bool,
        #[command(flatten)]
        integrator: Integrator,
        /// The integrating agent has finished.
        #[arg(long, requires = "agent")]
        ended: bool,
    },
    /// Take a lane off its train, to the next one or back to waiting.
    ///
    /// For a train the runner flagged as held: some lanes are ready, and one
    /// that isn't has kept it waiting. Only a lane that is ready to land can
    /// board another train, so a lane that isn't goes back to waiting, and
    /// boards a train when it is ready. The runner never does this itself:
    /// the lane has to rebase.
    Move {
        /// The lane, by name.
        lane: String,
        /// Where it goes: `next` (the next train started after its own, if one
        /// is still integrating), or a train's name. Only a lane that is ready
        /// to land boards a train; any other lane goes back to waiting.
        #[arg(long, default_value = "next")]
        to: String,
    },
    /// Every train on this board: the ones not landed first, with their CI.
    List,
}

/// The agent integrating a train, and the card it works for it (ov-461). The
/// agent is the train's own, never a lane named like it.
#[derive(Debug, Clone, clap::Args)]
pub(super) struct Integrator {
    /// The agent integrating it: its id, from its launch result.
    #[arg(long)]
    agent: Option<String>,
    /// `claude` or `codex`, for that agent.
    #[arg(long, requires = "agent")]
    harness: Option<String>,
    /// That agent's model.
    #[arg(long, requires = "agent")]
    model: Option<String>,
    /// The card the agent works for the train: "ov-453".
    #[arg(long, value_name = "KEY")]
    card: Option<String>,
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
        "train_settled" => "That train has landed or been dropped, so it takes no more moves, SHAs or lanes.",
        "lane_not_ready" => "A train takes only lanes that are ready to land. That lane isn't, so it waits for the next train.",
        "lane_in_review" => "That lane is still in review. Move it to landing when its review is clean, or add --force to board it now.",
        "train_unknown" => "No train on this board has that name. Start it first with `plan train start`.",
        "train_moving" => "That train has already gone to gating or CI, so a lane can't leave it. Wait for it to land, or drop the train.",
        "train_changed" => "That train moved while it was being changed. Read it again.",
        "lane_closed" => "That lane has already landed or been dropped, so it can't board a train.",
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

/// The lanes in `names` that board a train: those ready to land, and those in
/// review when `force`. Anything else is refused before a word is sent, naming
/// the lane and where it is. A lane already on `train` is left as it is. The
/// ones returned are those `force` has to move to landing first.
fn ready_to_board<'a>(plan: &'a pb::Plan, names: &[String], train: Option<&str>, force: bool) -> Result<Vec<&'a pb::Lane>, Failed> {
    let mut to_move = Vec::new();
    for name in names {
        let lane = find_lane(plan, name)?;
        if train.is_some_and(|t| lane.train.as_deref().is_some_and(|on| on.eq_ignore_ascii_case(t))) {
            continue;
        }
        let state = pb::LaneState::try_from(lane.state).unwrap_or(pb::LaneState::Queued);
        let said = match state {
            pb::LaneState::Landing => continue,
            pb::LaneState::Review if force => {
                to_move.push(lane);
                continue;
            }
            pb::LaneState::Review => format!(
                "{} is in review. Move it to landing when its review is clean, or add --force to board it now.",
                lane.name
            ),
            pb::LaneState::Landed | pb::LaneState::Dropped => {
                format!("{} has already {}, so it can't board a train.", lane.name, state_word(lane.state).to_lowercase())
            }
            _ => format!(
                "{} is {}. A train takes only lanes that are ready to land, so it waits for the next one.",
                lane.name,
                state_word(lane.state).to_lowercase()
            ),
        };
        return Err(said.into());
    }
    Ok(to_move)
}

/// Move each forced lane from review to landing, one lane write each, so the
/// timeline says how it got there.
async fn move_to_landing<L: DispatchLink>(link: &mut L, board: &Board, lanes: &[&pb::Lane], actor: &str) -> Result<(), Failed> {
    for lane in lanes {
        let p = request::Payload::LaneUpdate(pb::LaneUpdate {
            lane_id: lane.id.clone(),
            state: Some(pb::LaneState::Landing as i32),
            reason: Some("Boarded a train with --force.".into()),
            actor: actor.into(),
            ..Default::default()
        });
        send_plan(link, board, "lane.update", p).await?;
    }
    Ok(())
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
        if json { train_json(plan, t, &keys).to_string() } else { train_text(plan, t, &keys, now).join("\n") }
    };
    match cmd {
        TrainCmd::List => Ok(if json {
            json!({ "trains": plan.trains.iter().map(|t| train_json(&plan, t, &keys)).collect::<Vec<_>>() }).to_string()
        } else if plan.trains.is_empty() {
            "No trains yet. Start one with `farcooler plan train start`.".into()
        } else {
            plan.trains.iter().flat_map(|t| train_text(&plan, t, &keys, now)).collect::<Vec<_>>().join("\n")
        }),
        TrainCmd::Move { lane, to } => {
            let moved = plan_move(&plan, &lane, &to)?;
            let p = request::Payload::TrainSet(pb::TrainSet {
                train_id: moved.on.id.clone(),
                add_lane_ids: if moved.boards { vec![moved.lane.id.clone()] } else { Vec::new() },
                remove_lane_ids: if moved.boards { Vec::new() } else { vec![moved.lane.id.clone()] },
                actor: actor.into(),
                ..Default::default()
            });
            send(link, board, "train.set", p).await?;
            let from = train_called(moved.from);
            let (where_to, said) = match (moved.boards, &moved.dest) {
                (true, Some(dest)) => (Some(train_called(dest)), format!("Moved {} from {from} to {}.", moved.lane.name, train_called(dest))),
                _ if moved.lane.state == pb::LaneState::Landing as i32 => {
                    (None, format!("Took {} off {from}. It's ready to land and boards the next train that starts.", moved.lane.name))
                }
                _ => (None, format!("Took {} off {from}. It waits until it's ready to land, then boards a train.", moved.lane.name)),
            };
            Ok(if json {
                json!({ "lane": moved.lane.name, "from": from, "to": where_to }).to_string()
            } else {
                said
            })
        }
        TrainCmd::Start { name, lanes, base, title, force, integrator } => {
            let boarding = ready_to_board(&plan, &lanes, None, force)?;
            // Before any forced lane is written: a name in use would leave them in landing.
            if plan.trains.iter().any(|t| t.name.eq_ignore_ascii_case(name.trim())) {
                return Err(said_here("name_taken").unwrap_or_default().to_string().into());
            }
            let (agent, card_id) = integrator.wire(link, board, false).await?;
            move_to_landing(link, board, &boarding, actor).await?;
            let p = request::Payload::TrainStart(pb::TrainStart {
                workspace_id: ws,
                name,
                base,
                lane_ids: lane_ids(&plan, &lanes)?,
                actor: actor.into(),
                title: title.unwrap_or_default(),
                agent,
                card_id,
            });
            let made = send(link, board, "train.start", p).await?;
            Ok(said(&plan, &made))
        }
        TrainCmd::Set { name, state, sha, base, add, remove, title, force, integrator, ended } => {
            let found = find_train(&plan, &name)?;
            let boarding = ready_to_board(&plan, &add, Some(&found.name), force)?;
            if is_settled(found) && !add.is_empty() {
                return Err(said_here("train_settled").unwrap_or_default().to_string().into());
            }
            let (agent, card_id) = integrator.wire(link, board, ended).await?;
            move_to_landing(link, board, &boarding, actor).await?;
            let p = request::Payload::TrainSet(pb::TrainSet {
                train_id: found.id.clone(),
                state: state.map_or(pb::BoardTrainState::Unspecified, state_of) as i32,
                base,
                sha,
                add_lane_ids: lane_ids(&plan, &add)?,
                remove_lane_ids: lane_ids(&plan, &remove)?,
                actor: actor.into(),
                title,
                agent,
                card_id,
            });
            let set = send(link, board, "train.set", p).await?;
            Ok(said(&plan, &set))
        }
    }
}

/// Where `plan train move` sends a lane, decided from the plan.
struct Moving<'a> {
    lane: &'a pb::Lane,
    from: &'a pb::BoardTrain,
    dest: Option<&'a pb::BoardTrain>,
    /// The lane boards `dest`; else it comes off `from` and waits.
    boards: bool,
    /// The train the write goes to: `dest` when it boards, else `from`.
    on: &'a pb::BoardTrain,
}

/// "Train 86" for a train called `train-86` or `integ-86`, else its title or name.
fn train_called(t: &pb::BoardTrain) -> String {
    if t.title.is_empty() { t.name.clone() } else { t.title.clone() }
}

/// Where a lane goes. `next` is the train still integrating that was started
/// next after its own. Only a lane that is ready to land boards another train
/// (the runner holds the same rule), so any other lane waits instead, and a
/// train named outright is refused for it before anything is sent.
fn plan_move<'a>(plan: &'a pb::Plan, lane: &str, to: &str) -> Result<Moving<'a>, Failed> {
    let lane = find_lane(plan, lane)?;
    let from = match lane.train.as_deref() {
        Some(name) => find_train(plan, name)?,
        None => return Err(format!("{} isn't on a train.", lane.name).into()),
    };
    if is_settled(from) {
        return Err(said_here("train_settled").unwrap_or_default().to_string().into());
    }
    if from.pushed_sha.is_some() || from.state != pb::BoardTrainState::Integrating as i32 {
        return Err(format!("{} has already gone to gating or CI, so a lane can't come off it now.", train_called(from)).into());
    }
    let integrating = |t: &&pb::BoardTrain| t.state == pb::BoardTrainState::Integrating as i32 && t.id != from.id;
    let dest = if to.trim().eq_ignore_ascii_case("next") {
        plan.trains.iter().filter(integrating).filter(|t| t.created_at >= from.created_at).min_by_key(|t| (t.created_at, t.name.clone()))
    } else {
        let named = find_train(plan, to)?;
        if !integrating(&named) {
            return Err(format!("{} isn't integrating, so a lane can't board it.", train_called(named)).into());
        }
        Some(named)
    };
    let ready = lane.state == pb::LaneState::Landing as i32;
    if !ready && !to.trim().eq_ignore_ascii_case("next") {
        return Err(format!(
            "{} is {}. A train takes only lanes that are ready to land, so move it to {} when it is.",
            lane.name,
            state_word(lane.state).to_lowercase(),
            train_called(dest.unwrap_or(from))
        )
        .into());
    }
    let boards = ready && dest.is_some();
    Ok(Moving { lane, from, dest, boards, on: if boards { dest.unwrap_or(from) } else { from } })
}

impl Integrator {
    /// The wire's agent record and card id, the card named by key on `board`.
    async fn wire<L: DispatchLink>(
        self,
        link: &mut L,
        board: &Board,
        ended: bool,
    ) -> Result<(Option<pb::TrainAgentRecord>, Option<bytes::Bytes>), Failed> {
        let card_id = match &self.card {
            Some(key) => {
                let items = crate::tasks::board_in(link, board, None, None).await?.items;
                Some(card(link, board, &items, key).await?.id)
            }
            None => None,
        };
        let agent = self.agent.map(|id| pb::TrainAgentRecord {
            harness: self.harness.unwrap_or_else(|| "claude".into()),
            agent_id: id,
            model: self.model.filter(|m| !m.is_empty()),
            ended,
        });
        Ok((agent, card_id))
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

/// A train as a person reads it, its name second: "Train 14 (integ-14)".
pub(super) fn train_label(t: &pb::BoardTrain) -> String {
    if t.title.is_empty() || t.title.eq_ignore_ascii_case(&t.name) { t.name.clone() } else { format!("{} ({})", t.title, t.name) }
}

/// "working · 470k tokens": the integrating agent's state and spend (ov-461).
fn agent_words(a: &pb::TrainAgent) -> String {
    let state = if a.ended_at.is_some() { "finished" } else { "working" };
    let spend = a.spend.unwrap_or_default();
    if spend.runs > 0 { format!("{state} · {}", spend_words(&spend)) } else { state.to_string() }
}

/// "Train 14 (integ-14) · Red · 1a1b3275 · CI Failed · 2 of 15 jobs failed":
/// the train's line, as the Now section heads its group with it.
pub(super) fn train_line(plan: &pb::Plan, t: &pb::BoardTrain) -> String {
    let word = said_phase(t).unwrap_or_else(|| train_state_word(t.state));
    let mut parts = vec![train_label(t), word.to_string()];

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
    if let Some(agent) = &t.agent {
        parts.push(format!("agent {}", agent_words(agent)));
    }
    parts.join(" · ")
}

/// The phase's word where it says what the stored state can't: a train
/// waiting on lanes, stalled, gating locally, or green and not on main. For
/// the rest ("CI failed" beside the CI line that says so) the state's word reads
/// the same.
fn said_phase(t: &pb::BoardTrain) -> Option<&'static str> {
    use pb::BoardTrainPhase as P;
    match P::try_from(t.phase) {
        Ok(P::WaitingForLanes | P::Stalled | P::GatingLocally | P::ReadyToLand) => train_phase_word(t.phase),
        _ => None,
    }
}

fn is_settled(t: &pb::BoardTrain) -> bool {
    t.state == pb::BoardTrainState::Landed as i32 || t.state == pb::BoardTrainState::Dropped as i32
}

/// `plan train list`'s lines for one train: its line, its base and lanes, and
/// any job that didn't pass.
fn train_text(plan: &pb::Plan, t: &pb::BoardTrain, keys: &Keys, now: i64) -> Vec<String> {
    let mut out = vec![train_line(plan, t)];
    let mut facts = Vec::new();
    if !t.base.is_empty() {
        facts.push(format!("cut from {}", t.base));
    }
    let (word, since) = match said_phase(t) {
        Some(word) => (word, t.phase_since),
        None => (train_state_word(t.state), t.state_since),
    };
    facts.push(format!("{} for {}", word.to_lowercase(), super::age(now - since)));
    out.push(format!("  {}", facts.join(" · ")));
    if !t.held_by.is_empty() {
        let held: Vec<String> = t.held_by.iter().map(|id| lane_name(plan, id)).collect();
        out.push(format!(
            "  Holding for {}. `{} plan train move {} --actor manager` takes the lane off, to the next train or back to waiting.",
            held.join(", "),
            crate::runner_install::cli_name(),
            held[0]
        ));
    }
    if !t.summary.is_empty() {
        out.push(format!("  Carries: {}", t.summary));
    }
    if let Some(a) = &t.agent {
        let model = if a.model.is_empty() { String::new() } else { format!(" {}", a.model) };
        out.push(format!("  Agent: {}{model} {} · {}", a.harness, a.agent_id, agent_words(a)));
    }
    if let Some(card) = &t.card_id {
        out.push(format!("  Card: {}", keys.of(card)));
    }
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
pub(super) fn train_json(plan: &pb::Plan, t: &pb::BoardTrain, keys: &Keys) -> Value {
    json!({
        "id": id_text(&t.id),
        "short": short_bytes(&t.id),
        "name": t.name,
        "base": t.base,
        "pushed_sha": t.pushed_sha,
        "state": train_state_key(t.state),
        "state_since": t.state_since,
        "phase": train_phase_key(t.phase),
        "phase_since": (t.phase != 0).then_some(t.phase_since),
        "held_by": t.held_by.iter().map(|id| json!({ "lane": id_text(id), "name": lane_name(plan, id) })).collect::<Vec<_>>(),
        "actor": t.actor,
        "created_at": t.created_at,
        "landed_at": t.landed_at,
        "lanes": t.lane_ids.iter().map(|id| json!({ "lane": id_text(id), "name": lane_name(plan, id) })).collect::<Vec<_>>(),
        "ci_subject": t.ci_subject,
        "title": t.title,
        "summary": t.summary,
        "card": t.card_id.as_ref().map(|id| json!({ "task": id_text(id), "key": keys.of(id) })),
        "agent": t.agent.as_ref().map(|a| json!({
            "harness": a.harness, "agent_id": a.agent_id, "model": a.model,
            "started_at": a.started_at, "ended_at": a.ended_at,
            "spend": super::spend_json(&a.spend.unwrap_or_default()),
        })),
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

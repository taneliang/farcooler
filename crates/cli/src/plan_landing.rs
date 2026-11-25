//! `farcooler landing`: landings (ov-309, ov-872), read.
//!
//! The workflow engine lands now (ov-960): its landing groups draw here as
//! landings, and the landings from before it are kept as history, each read
//! as where it ended. Historical landings are numbered per board. Engine
//! landing groups are identified by the UUIDs shown in the list.
//!
//! ```text
//! farcooler landing list
//! farcooler landing show 12
//! ```
//!
//! `landing show` prints the landing's details and the timeline for stored
//! landings. The board's workflow lands tickets (`farcooler workflow get <board>`).
//!
//! A child of `plan.rs`, whose board and refusal helpers it shares.

use clap::{Args, Subcommand};
use farcooler_protocol::v1 as pb;
use serde_json::json;

use super::overview_text::{agent_name, key_of};
use super::{
    Common, Failed, age, ago, events_json, events_of, get_plan, id_text, needs_the_line, on_a_board,
    spend_words, unreadable,
};
use crate::ci_words::{landing_phase_word, landing_state_word, read_for, stale, summary};
use crate::tasks::{Board, DispatchLink};

/// `farcooler landing`.
#[derive(Debug, Clone, Args)]
pub(crate) struct LandingArgs {
    #[command(flatten)]
    pub(super) common: Common,
    #[command(subcommand)]
    pub(super) cmd: LandingCmd,
}

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum LandingCmd {
    /// Every landing on this board: the ones not landed first, with their CI.
    List,
    /// One landing: its tickets, lander, CI and timeline.
    Show {
        /// The historical landing number or the engine group UUID from landing list.
        selector: String,
    },
}

/// Landed or dropped.
pub(super) fn is_settled(l: &pb::Landing) -> bool {
    l.state == pb::LandingState::Landed as i32 || l.state == pb::LandingState::Dropped as i32
}

/// "Landing 12", "Mac polish (Landing 12)", or an engine group's title.
pub(super) fn landing_label(l: &pb::Landing) -> String {
    if l.number == 0 {
        return if l.title.is_empty() { "Landing".into() } else { l.title.clone() };
    }
    let number = format!("Landing {}", l.number);
    if l.title.is_empty() || l.title.eq_ignore_ascii_case(&number) { number } else { format!("{} ({number})", l.title) }
}

/// Match an engine group UUID or a historical number such as "12", "#12",
/// "Landing 12" or "landing-12".
fn find_landing<'a>(plan: &'a pb::Plan, given: &str) -> Result<&'a pb::Landing, Failed> {
    if let Ok(id) = uuid::Uuid::parse_str(given.trim()) {
        return plan.landings.iter()
            .find(|l| l.number == 0 && l.id.as_ref() == id.as_bytes())
            .ok_or_else(|| format!("No engine landing here has ID {id}. `farcooler landing list` shows them.").into());
    }
    let text = given.trim().to_lowercase();
    let text = text.strip_prefix("landing").unwrap_or(&text).trim_start_matches(['#', '-', ' ']);
    let Ok(number) = text.parse::<u32>() else {
        return Err(format!("Use a historical landing number or an engine group UUID from `farcooler landing list`. Got {given:?}.").into());
    };
    let mut matches = plan.landings.iter().filter(|l| l.number == number);
    let found = matches.next().ok_or_else(|| {
        Failed::from(format!("No landing here is number {number}. `farcooler landing list` shows them."))
    })?;
    // Older callers could use 0 when there was only one engine group.
    if number == 0 && matches.next().is_some() {
        return Err("More than one engine landing. Use a group UUID from `farcooler landing list`.".into());
    }
    Ok(found)
}

/// A landing's CI read, once the runner has one.
fn ci_of<'a>(plan: &'a pb::Plan, l: &pb::Landing) -> Option<&'a pb::BoardCiRead> {
    if l.ci_subject.is_empty() { None } else { read_for(&plan.ci, &l.ci_subject) }
}

/// The phase's word where it says what the stored state can't: a landing
/// waiting on another, on main, or on its tickets, stalled, gating locally,
/// or green and not landed. For the rest ("CI failed" beside the CI line that
/// says so) the state's word reads the same.
fn said_phase(l: &pb::Landing) -> Option<&'static str> {
    use pb::LandingPhase as P;
    match P::try_from(l.phase) {
        Ok(
            P::WaitingForPreviousLanding
            | P::WaitingForMain
            | P::WaitingForTickets
            | P::Stalled
            | P::GatingLocally
            | P::ReadyToLand,
        ) => landing_phase_word(l.phase),
        _ => None,
    }
}

fn route_words(route: i32) -> Option<&'static str> {
    match pb::LandingRoute::try_from(route) {
        Ok(pb::LandingRoute::PullRequest) => Some("by pull request"),
        Ok(pb::LandingRoute::Ask) => Some("asks you first"),
        _ => None,
    }
}

fn keys_on(plan: &pb::Plan, l: &pb::Landing) -> String {
    l.task_ids.iter().map(|id| key_of(plan, id)).collect::<Vec<_>>().join(" ")
}

/// "Landing 9 · Red · c85bf83d · CI Failed · 1 of 3 jobs failed · ov-1 ·
/// lander Lander sonnet (a1b2c3d4)": the landing's line, as the overview
/// lists it.
pub(super) fn landing_line(plan: &pb::Plan, l: &pb::Landing) -> String {
    let word = said_phase(l).unwrap_or_else(|| landing_state_word(l.state));
    let mut parts = vec![landing_label(l), word.to_string()];
    parts.extend(route_words(l.route).map(String::from));
    if let Some(sha) = &l.pushed_sha {
        parts.push(sha.chars().take(8).collect());
    }
    if let Some(read) = ci_of(plan, l) {
        // "CI unknown" already says CI.
        let said = summary(read);
        parts.push(if said.starts_with("CI ") { said } else { format!("CI {said}") });
        parts.extend(stale(read, plan.now_ms));
    } else if l.pushed_sha.is_some() && !is_settled(l) {
        parts.push("CI not read yet".into());
    }
    let keys = keys_on(plan, l);
    if !keys.is_empty() {
        parts.push(keys);
    }
    if let Some(lander) = &l.lander_id {
        parts.push(format!("lander {}", agent_name(plan, lander)));
        let spend = l.spend.unwrap_or_default();
        if spend.runs > 0 {
            parts.push(spend_words(&spend));
        }
    }
    parts.join(" · ")
}

/// `landing list` and `landing show`'s lines for one landing: its line, its
/// base, what it carries, and any job that didn't pass.
fn landing_text(plan: &pb::Plan, l: &pb::Landing, now: i64) -> Vec<String> {
    let mut line = landing_line(plan, l);
    if l.number == 0 {
        line.push_str(&format!(" · {}", id_text(&l.id)));
    }
    let mut out = vec![line];
    let mut facts = Vec::new();
    if !l.base.is_empty() {
        facts.push(format!("cut from {}", l.base));
    }
    let (word, since) = match said_phase(l) {
        Some(word) => (word, l.phase_since),
        None => (landing_state_word(l.state), l.state_since),
    };
    facts.push(format!("{} for {}", word.to_lowercase(), age(now - since)));
    out.push(format!("  {}", facts.join(" · ")));
    if let Some(reason) = &l.reason { out.push(format!("  {reason}")); }
    if !l.summary.is_empty() {
        out.push(format!("  Carries: {}", l.summary));
    }
    if let Some(read) = ci_of(plan, l) {
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

/// One `landing` command on `board`, answering what to print.
pub(super) async fn run_on<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: LandingCmd,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_the_line(&link.capabilities())?;
    let (board, ws) = on_a_board(link, board).await?;
    let board = &board;
    let mut plan = get_plan(link, &ws, true).await?;
    let payload = pb::request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: ws, include_closed: true });
    let mut request = super::with(super::req_for("landing.history", board.repository), payload);
    request.required_capabilities.push(farcooler_protocol::capability::BOARD_LINE.to_string());
    match link.call(request).await {
        Ok(answer) => match super::expect_value(answer.value)? {
            pb::result::Value::Plan(history) => {
                plan.landings.extend(history.landings);
                plan.cards.extend(history.cards);
            }
            _ => return Err(unreadable()),
        },
        Err(super::ClientError::Daemon { code, .. }) if code == pb::ErrorCode::CapabilityUnsupported as i32 => (),
        Err(e) => return Err(super::refused_here(e, "The runner couldn't read landing history.")),
    }
    match cmd {
        LandingCmd::List => Ok(if json {
            let all: Vec<_> = plan.landings.iter().map(|l| farcooler_client::plan_json::landing_json(&plan, l)).collect();
            json!({ "landings": all }).to_string()
        } else if plan.landings.is_empty() {
            "No landings yet. The board's workflow lands its tickets: `farcooler workflow get <board>` shows its runs.".into()
        } else {
            plan.landings.iter().flat_map(|l| landing_text(&plan, l, now)).collect::<Vec<_>>().join("\n")
        }),
        LandingCmd::Show { selector } => {
            let found = find_landing(&plan, &selector)?;
            // Engine groups have workflow history. The line timeline only accepts stored landing IDs.
            let events = if found.number == 0 {
                Vec::new()
            } else {
                events_of(link, board, pb::plan_events_request::Subject::LandingId(found.id.clone())).await?
            };
            Ok(if json {
                let mut out = farcooler_client::plan_json::landing_json(&plan, found);
                out["events"] = events_json(&events);
                out.to_string()
            } else {
                let mut out = landing_text(&plan, found, now);
                if !events.is_empty() {
                    out.push(String::new());
                    out.push("Timeline".into());
                    out.extend(events.iter().map(|e| format!("  {}  {}", ago(now - e.at), e.body)));
                }
                out.join("\n")
            })
        }
    }
}


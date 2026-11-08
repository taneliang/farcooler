//! `farcooler landing`: landings (ov-309, ov-872).
//!
//! A landing puts finished tickets into the product along one route: push
//! (fold onto the base, gate, push, watch CI), pull request (one per ticket,
//! which a person merges) or ask (it waits for you). A landing of one ticket
//! is the usual case; several ready at once may share one gate. Landings are
//! named by number, counting up per board: Landing 12.
//!
//! ```text
//! farcooler landing list
//! farcooler landing show 12
//! farcooler landing start KEY... [--land] [--title T] [--lander AGENT] [--force]
//! farcooler landing start --record-only [--ticket KEY]... [--route push|pull_request|ask] [--base origin/main]
//!     [--title T] [--lander AGENT] [--force]
//! farcooler landing set 12 [--state gating|...] [--sha 1a1b3275] [--base B] [--title T]
//!     [--add KEY] [--remove KEY] [--lander AGENT]
//! farcooler landing move KEY [--to next|12]
//! ```
//!
//! `landing start` hands the landing to the runner's landing job (ov-872): the
//! charter's `## Landing` section routes each ticket, and the push route is
//! folded, gated, pushed and watched to landed, each step on the landing's
//! timeline, which `landing show` prints. `--record-only` only records it, for
//! a landing someone lands by hand.
//!
//! The board keeps one collecting landing: a ticket that passes review joins
//! it on its own, and one that leaves landing comes off a landing whose SHA
//! isn't out yet. `landing start` adopts the collecting landing rather than
//! making a second. Only a ticket ready to land goes on a landing; one still
//! in review goes on with `--force`, which moves it to landing first. The
//! runner holds the same rule. The runner reads a pushed SHA's CI through
//! `gh` and moves the landing to green or red on its own.
//!
//! A child of `plan.rs`, whose board and refusal helpers it shares.

use clap::{Args, Subcommand, ValueEnum};
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::json;

use super::agent::find_agent;
use super::overview_text::{agent_name, key_of};
use super::ticket::stage_phrase;
use super::{
    Common, Failed, age, ago, board_in, card, events_json, events_of, find_line, get_plan, needs_the_line,
    on_a_board, send, send_saying, spend_words, unreadable,
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
        /// Its number: 12.
        number: String,
    },
    /// Land these tickets: the runner starts a landing with them and runs it.
    ///
    /// The charter's ## Landing section chooses each ticket's route by the
    /// files it changes. Push tickets are folded onto the base, gated with
    /// the charter's gate, pushed, and watched until CI passes; a batch whose
    /// gate fails splits in two. A ticket the charter asks about waits for
    /// you. `landing show` follows the steps.
    ///
    /// Only tickets that are ready to land go on a landing. One still in
    /// review goes on with --force; any other is refused.
    Start {
        /// A ticket to land, by key.
        #[arg(value_name = "KEY")]
        keys: Vec<String>,
        /// A ticket to put on it, by key. Repeat for more.
        #[arg(long = "ticket", value_name = "KEY")]
        tickets: Vec<String>,
        /// Only record the landing; land it yourself, and give it its SHA
        /// with `landing set`.
        #[arg(long)]
        record_only: bool,
        /// You say land them: tickets the charter asked about are pushed.
        /// Only a person says this, never an agent.
        #[arg(long, conflicts_with = "record_only")]
        land: bool,
        /// How a recorded landing lands: push (the default), pull_request or
        /// ask. With --record-only; otherwise the charter chooses.
        #[arg(long, value_enum, requires = "record_only")]
        route: Option<RouteArg>,
        /// What it's cut from: origin/main, or a SHA.
        #[arg(long, default_value = "")]
        base: String,
        /// What it carries, for people. Without one it reads "Landing 12".
        #[arg(long)]
        title: Option<String>,
        /// The agent that lands it, by its short id.
        #[arg(long, value_name = "AGENT")]
        lander: Option<String>,
        /// Put a ticket that's still in review on the landing by moving it to landing first.
        #[arg(long)]
        force: bool,
    },
    /// Move a landing, record what it pushed, name its lander, or put tickets on or off it.
    ///
    /// A new --sha without --state moves it to pushed. From there the runner
    /// moves it to green or red as its CI says. Landing it lands every ticket
    /// on it.
    Set {
        /// Its number: 12.
        number: String,
        /// Open, gating, pushed, green, red, landed or dropped.
        #[arg(long, value_enum)]
        state: Option<StateArg>,
        /// The SHA it pushed.
        #[arg(long)]
        sha: Option<String>,
        /// What it's cut from now.
        #[arg(long)]
        base: Option<String>,
        /// Its title for people.
        #[arg(long)]
        title: Option<String>,
        /// A ticket to put on it, by key. Repeat for more.
        #[arg(long = "add", value_name = "KEY")]
        add: Vec<String>,
        /// A ticket to take off it, by key. Repeat for more.
        #[arg(long = "remove", value_name = "KEY")]
        remove: Vec<String>,
        /// The agent that lands it, by its short id.
        #[arg(long, value_name = "AGENT")]
        lander: Option<String>,
    },
    /// Take a ticket off its landing so it rides the next one.
    ///
    /// A ticket that leaves landing comes off its landing on its own, so this
    /// is for one you want elsewhere. Only an open landing gives a ticket up.
    Move {
        /// The ticket, by key.
        key: String,
        /// Where it goes: `next` (the next open landing after its own), or a
        /// landing's number. Only a ticket ready to land joins one; any other
        /// waits until it is.
        #[arg(long, default_value = "next")]
        to: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum RouteArg {
    Push,
    #[value(name = "pull_request", alias = "pull-request")]
    PullRequest,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum StateArg {
    Open,
    Gating,
    Pushed,
    Green,
    Red,
    Landed,
    Dropped,
}

impl StateArg {
    fn wire(self) -> pb::LandingState {
        match self {
            StateArg::Open => pb::LandingState::Open,
            StateArg::Gating => pb::LandingState::Gating,
            StateArg::Pushed => pb::LandingState::Pushed,
            StateArg::Green => pb::LandingState::Green,
            StateArg::Red => pb::LandingState::Red,
            StateArg::Landed => pb::LandingState::Landed,
            StateArg::Dropped => pb::LandingState::Dropped,
        }
    }
}

/// This module's sentences for the words the runner refuses a landing with,
/// said before `plan.rs`'s.
pub(super) fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "base" => "That base is too long. Name a ref or a SHA.",
        "sha" => "Give the SHA it pushed with --sha: 7 to 40 hex digits. A landing is pushed, green or red only once it has one.",
        "no_sha" => "A landing lands only once it has pushed a commit. Give it with --sha.",
        "lander" => "A landing's lander is a lander or the orchestrator still at work on this board.",
        "not_ready" => "A landing takes only tickets that are ready to land. That one isn't yet, so it waits for the next landing.",
        "ticket_in_review" => "That ticket is still in review. Move it to landing when its review is clean, or add --force to put it on a landing now.",
        "ticket_closed" => "That ticket has already landed or been dropped, so it can't go on a landing.",
        "landing_moving" => "That landing has already gone to gating or CI, so a ticket can't come off it. Wait for it to land, or drop it.",
        "landing_settled" => "That landing has landed or been dropped, so it takes no more moves, SHAs or tickets.",
        "engine_lands" => "This board lands through the workflow engine now, so landings from before the move are kept as history and take no more moves, SHAs or tickets. Land tickets through a workflow: `farcooler workflow get <board>` shows its runs.",
        "landing_changed" => "That landing moved while it was being changed. Read it again.",
        "route" => "Use push, pull_request or ask.",
        "no_landing_rules" => "The charter has no ## Landing section, so the runner doesn't know how this board lands. Add one (`- Route: push`, `- Gate: <command>`), or land it yourself with --record-only.",
        "charter_landing" => "The charter's ## Landing section doesn't read. `farcooler workspace charter` shows it.",
        "landing_job_running" => "The runner is already landing on this board. Wait for it, with `farcooler landing list`.",
        "land_is_yours" => "Only a person says to land what the charter asks about. Ask the owner.",
        "waits_for_you" => "The charter asks before landing that ticket. It lands when a person says land it (`landing start <key> --land`).",
        "no_tickets" => "Name a ticket to land.",
        "title" => "A title is one short line.",
        _ => return None,
    })
}

async fn send_landing<L: DispatchLink>(link: &mut L, board: &Board, method: &str, p: request::Payload) -> Result<pb::Landing, Failed> {
    match send_saying(link, board, method, p, said_here).await? {
        result::Value::Landing(l) => Ok(l),
        _ => Err(unreadable()),
    }
}

/// Landed or dropped: it takes no more writes.
pub(super) fn is_settled(l: &pb::Landing) -> bool {
    l.state == pb::LandingState::Landed as i32 || l.state == pb::LandingState::Dropped as i32
}

/// "Landing 12", or "Mac polish (Landing 12)" for one with a title of its own.
pub(super) fn landing_label(l: &pb::Landing) -> String {
    let number = format!("Landing {}", l.number);
    if l.title.is_empty() || l.title.eq_ignore_ascii_case(&number) { number } else { format!("{} ({number})", l.title) }
}

/// A landing by its number: "12", "#12", "Landing 12" or "landing-12".
fn find_landing<'a>(plan: &'a pb::Plan, given: &str) -> Result<&'a pb::Landing, Failed> {
    let text = given.trim().to_lowercase();
    let text = text.strip_prefix("landing").unwrap_or(&text).trim_start_matches(['#', '-', ' ']);
    let Ok(number) = text.parse::<u32>() else {
        return Err(format!("Name a landing by its number, like 12. {given:?} isn't one.").into());
    };
    plan.landings
        .iter()
        .find(|l| l.number == number)
        .ok_or_else(|| format!("No landing here is number {number}. `farcooler landing list` shows them.").into())
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
    let mut out = vec![landing_line(plan, l)];
    let mut facts = Vec::new();
    if !l.base.is_empty() {
        facts.push(format!("cut from {}", l.base));
    }
    let (word, since) = match said_phase(l) {
        Some(word) => (word, l.phase_since),
        None => (landing_state_word(l.state), l.state_since),
    };
    facts.push(format!("{} for {}", word.to_lowercase(), age(now - since)));
    if l.collecting {
        facts.push("tickets ready to land join it by themselves".into());
    }
    out.push(format!("  {}", facts.join(" · ")));
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

/// The tickets in `tasks` that `on` takes now, or `None` for a new landing:
/// those ready to land, those waiting on you for a landing that asks, and
/// those in review when `force`. Anything else is refused before a word is
/// sent, naming the ticket and where it is. The ones returned are those
/// `force` has to move to landing first.
fn ready_to_land(plan: &pb::Plan, tasks: &[pb::Task], on: Option<&pb::Landing>, ask: bool, force: bool) -> Result<Vec<bytes::Bytes>, Failed> {
    let mut to_move = Vec::new();
    for task in tasks {
        if on.is_some_and(|l| l.task_ids.contains(&task.id)) {
            continue;
        }
        let key = &task.key;
        let Some(line) = plan.lines.iter().find(|l| l.task_id == task.id) else {
            return Err(format!("{key} isn't on the line. Put it there with `farcooler plan ticket set {key}`.").into());
        };
        let said = match pb::TicketStage::try_from(line.stage) {
            Ok(pb::TicketStage::Landing) => continue,
            Ok(pb::TicketStage::NeedsYou) if ask => continue,
            Ok(pb::TicketStage::Reviewing) if force => {
                to_move.push(task.id.clone());
                continue;
            }
            Ok(pb::TicketStage::Reviewing) => format!(
                "{key} is in review. Move it to landing when its review is clean, or add --force to put it on a landing now."
            ),
            Ok(pb::TicketStage::Landed | pb::TicketStage::Dropped) => {
                format!("{key} is {} already, so it can't go on a landing.", stage_phrase(line.stage))
            }
            _ => format!(
                "{key} is {}. A landing takes only tickets that are ready to land, so it waits for the next one.",
                stage_phrase(line.stage)
            ),
        };
        return Err(said.into());
    }
    Ok(to_move)
}

/// Move each forced ticket from review to landing, one write each, so its
/// timeline says how it got there.
async fn move_to_landing<L: DispatchLink>(link: &mut L, board: &Board, tasks: &[bytes::Bytes], actor: &str) -> Result<(), Failed> {
    for task in tasks {
        let p = request::Payload::LineSet(pb::TicketLineSet {
            task_id: task.clone(),
            stage: Some(pb::TicketStage::Landing as i32),
            reason: Some("Put on a landing with --force.".into()),
            actor: actor.into(),
            ..Default::default()
        });
        send(link, board, "line.set", p).await?;
    }
    Ok(())
}

/// The cards `keys` name on this board, read once.
async fn tasks_of<L: DispatchLink>(link: &mut L, board: &Board, keys: &[String]) -> Result<Vec<pb::Task>, Failed> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let items = board_in(link, board, None, None).await?.items;
    let mut out = Vec::new();
    for key in keys {
        out.push(card(link, board, &items, key).await?);
    }
    Ok(out)
}

/// One `landing` command on `board`, answering what to print.
pub(super) async fn run_on<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: LandingCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_the_line(&link.capabilities())?;
    let (board, ws) = on_a_board(link, board).await?;
    let board = &board;
    let plan = get_plan(link, &ws, true).await?;
    let said = |plan: &pb::Plan, l: &pb::Landing| -> String {
        if json { farcooler_client::plan_json::landing_json(plan, l).to_string() } else { landing_text(plan, l, now).join("\n") }
    };
    match cmd {
        LandingCmd::List => Ok(if json {
            let all: Vec<_> = plan.landings.iter().map(|l| farcooler_client::plan_json::landing_json(&plan, l)).collect();
            json!({ "landings": all }).to_string()
        } else if plan.landings.is_empty() {
            "No landings yet. A ticket that passes review joins the next one by itself, or start one with `farcooler landing start`.".into()
        } else {
            plan.landings.iter().flat_map(|l| landing_text(&plan, l, now)).collect::<Vec<_>>().join("\n")
        }),
        LandingCmd::Show { number } => {
            let found = find_landing(&plan, &number)?;
            let events = events_of(link, board, pb::plan_events_request::Subject::LandingId(found.id.clone())).await?;
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
        LandingCmd::Start { keys, tickets, record_only, land, route, base, title, lander, force } => {
            let tickets: Vec<String> = keys.into_iter().chain(tickets).collect();
            if (!record_only || force) && tickets.is_empty() {
                return Err("Name a ticket to land, like `farcooler landing start ov-12`.".into());
            }
            let tasks = tasks_of(link, board, &tickets).await?;
            let boarding = ready_to_land(&plan, &tasks, None, route == Some(RouteArg::Ask) || land, force)?;
            let lander_id = lander.map(|a| find_agent(&plan, &a)).transpose()?;
            move_to_landing(link, board, &boarding, actor).await?;
            let p = request::Payload::LandingStart(pb::LandingStart {
                workspace_id: ws.clone(),
                route: route.map_or(pb::LandingRoute::Unspecified, |r| match r {
                    RouteArg::Push => pb::LandingRoute::Push,
                    RouteArg::PullRequest => pb::LandingRoute::PullRequest,
                    RouteArg::Ask => pb::LandingRoute::Ask,
                }) as i32,
                base,
                task_ids: tasks.iter().map(|t| t.id.clone()).collect(),
                title: title.unwrap_or_default(),
                lander_id,
                actor: actor.into(),
                run: !record_only,
                land,
            });
            let made = match send_landing(link, board, "landing.start", p).await {
                Ok(made) => made,
                Err(e) if e.to_string() == said_here("charter_landing").unwrap_or_default() => {
                    // Name the line that didn't read, from the charter itself.
                    let charter = crate::workspaces::charter::read(link, crate::uuid_of(&ws)).await.ok();
                    let named = charter.and_then(|c| crate::workspaces::charter::landing_section_problem(&c.text));
                    return Err(named.map_or(e, Into::into));
                }
                Err(e) => return Err(e),
            };
            let shown = said(&plan_with(&plan, &made, &tasks), &made);
            if record_only || json {
                return Ok(shown);
            }
            Ok(format!("{shown}\nThe runner is landing it. `farcooler landing show {}` follows its steps.", made.number))
        }
        LandingCmd::Set { number, state, sha, base, title, add, remove, lander } => {
            let found = find_landing(&plan, &number)?;
            if is_settled(found) {
                return Err(said_here("landing_settled").unwrap_or_default().into());
            }
            let adding = tasks_of(link, board, &add).await?;
            let ask = found.route == pb::LandingRoute::Ask as i32;
            ready_to_land(&plan, &adding, Some(found), ask, false)?;
            let removing = tasks_of(link, board, &remove).await?;
            let lander_id = lander.map(|a| find_agent(&plan, &a)).transpose()?;
            let p = request::Payload::LandingSet(pb::LandingSet {
                landing_id: found.id.clone(),
                state: state.map_or(pb::LandingState::Unspecified, StateArg::wire) as i32,
                base,
                sha,
                add_task_ids: adding.iter().map(|t| t.id.clone()).collect(),
                remove_task_ids: removing.iter().map(|t| t.id.clone()).collect(),
                title,
                lander_id,
                actor: actor.into(),
            });
            let set = send_landing(link, board, "landing.set", p).await?;
            let named: Vec<pb::Task> = adding.into_iter().chain(removing).collect();
            Ok(said(&plan_with(&plan, &set, &named), &set))
        }
        LandingCmd::Move { key, to } => {
            let moving = plan_move(&plan, &key, &to)?;
            let (on, add, remove) = match moving.dest.filter(|_| moving.boards) {
                Some(dest) => (dest, vec![moving.task.clone()], Vec::new()),
                None => (moving.from, Vec::new(), vec![moving.task.clone()]),
            };
            let p = request::Payload::LandingSet(pb::LandingSet {
                landing_id: on.id.clone(),
                add_task_ids: add,
                remove_task_ids: remove,
                actor: actor.into(),
                ..Default::default()
            });
            send_landing(link, board, "landing.set", p).await?;
            let from = landing_label(moving.from);
            let key = &moving.key;
            let (to_number, line) = match (moving.boards, moving.dest) {
                (true, Some(dest)) => (Some(dest.number), format!("Moved {key} from {from} to {}.", landing_label(dest))),
                _ if moving.ready => (
                    None,
                    format!(
                        "Took {key} off {from}. It stays off until you put it on a landing with `farcooler landing start --ticket {key}`, or it goes back through review."
                    ),
                ),
                _ => (None, format!("Took {key} off {from}. It joins a landing when it's ready to land.")),
            };
            Ok(if json { json!({ "ticket": key, "from": moving.from.number, "to": to_number }).to_string() } else { line })
        }
    }
}

/// `plan` with `landing` in it as the runner answered, and the cards `tasks`
/// named, so the answer names every ticket by its key.
fn plan_with(plan: &pb::Plan, landing: &pb::Landing, tasks: &[pb::Task]) -> pb::Plan {
    let mut out = super::cards_only(plan, tasks);
    out.agents = plan.agents.clone();
    out.ci = plan.ci.clone();
    out.lines = plan.lines.clone();
    out.landings = vec![landing.clone()];
    out
}

/// Where `landing move` sends a ticket, decided from the plan.
struct Moving<'a> {
    key: String,
    task: bytes::Bytes,
    from: &'a pb::Landing,
    dest: Option<&'a pb::Landing>,
    /// Ready to land: in the landing stage.
    ready: bool,
    /// It joins `dest`; else it comes off `from`.
    boards: bool,
}

/// Where a ticket goes. `next` is the open landing numbered next after its
/// own. Only a ticket that is ready to land joins another landing (the
/// runner holds the same rule), so any other comes off and waits, and a
/// landing named outright is refused for it before anything is sent.
fn plan_move<'a>(plan: &'a pb::Plan, key: &str, to: &str) -> Result<Moving<'a>, Failed> {
    let (card, line) = find_line(plan, key)?;
    let Some(from) = plan.landings.iter().find(|l| !is_settled(l) && l.task_ids.contains(&line.task_id)) else {
        return Err(format!("{} isn't on a landing.", card.key).into());
    };
    if from.state != pb::LandingState::Open as i32 || from.pushed_sha.is_some() {
        return Err(format!("{} has already gone to gating or CI, so a ticket can't come off it now.", landing_label(from)).into());
    }
    let open = |l: &&pb::Landing| l.state == pb::LandingState::Open as i32 && l.id != from.id;
    let next = to.trim().eq_ignore_ascii_case("next");
    let dest = if next {
        plan.landings.iter().filter(open).filter(|l| l.number > from.number).min_by_key(|l| l.number)
    } else {
        let named = find_landing(plan, to)?;
        if !open(&named) {
            return Err(format!("{} isn't open, so a ticket can't join it.", landing_label(named)).into());
        }
        Some(named)
    };
    let ready = line.stage == pb::TicketStage::Landing as i32;
    if !ready && !next {
        return Err(format!(
            "{} is {}. A landing takes only tickets that are ready to land, so move it to {} when it is.",
            card.key,
            stage_phrase(line.stage),
            landing_label(dest.unwrap_or(from))
        )
        .into());
    }
    Ok(Moving { key: card.key.clone(), task: line.task_id.clone(), from, dest, ready, boards: ready && dest.is_some() })
}

//! `farcooler plan`, `farcooler agent` and `farcooler landing`: themes, the
//! line, agents and landings (ov-268, ov-872).
//!
//! **Experimental, and beside the board.** A theme is why a group of cards
//! exists. THE LINE holds one row per ticket being worked: its stage
//! (waiting, building, in review, needs you, landing, landed or dropped), its
//! own branch, the ticket it's stacked on, its pull request and how it landed.
//! The plan is the board's ordered list of waiting tickets. An agent is
//! recorded when it starts: the orchestrator, a builder, a reviewer or a
//! lander, working any number of tickets. A landing puts finished tickets into
//! the product. The orchestrator writes all of it, and the `task` verbs are
//! unchanged: nothing here edits a card, and no card carries any of it.
//!
//! ```text
//! farcooler plan                              the overview an app draws
//! farcooler plan set KEY...                   waiting tickets, first is next up
//! farcooler plan theme  list|show|create|set|move|cards
//! farcooler plan ticket set|show|pr-body      one ticket on the line
//! farcooler plan ruling add|set|keep|reverse|list   decided for you (ov-304)
//! farcooler agent  list|dispatch|record|work|end    agents at work
//! farcooler landing list|show|start|set|move        landings and their CI
//! ```
//!
//! `plan` is nested because `farcooler theme` already lists the terminal's
//! color schemes. Every command takes `--repo`, `--workspace` and `--actor`
//! with the meanings `task` gives them (`--runner` and `--json` are the
//! top-level flags), and names cards the way `task` does. A ticket is named
//! by its card's key, a theme by its name or short id, an agent by its short
//! id, and a landing by its number.
//!
//! One read draws the overview, so a heartbeat reads `plan --json` instead of
//! `task list` plus a `task show` per card. `plan --json` is exactly
//! `farcooler_client::plan_json::plan_json`, the shape the apps decode.

use std::collections::HashMap;

use clap::{Args, Subcommand, ValueEnum};
use farcooler_core::usage_words::{NOT_REPORTED, dollars, tokens};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use serde_json::Value;

use crate::tasks::{Board, DispatchLink, Refused, actor_for, board_for, board_in, find_task, refused};
use crate::workspaces::{WORKSPACE_ENV, workspaces_on};
use crate::{Fallible, connect_to, expect_value, req_for, short_bytes, uuid_of, with};

#[path = "plan_ruling.rs"]
mod ruling;
#[path = "plan_cost.rs"]
mod cost;
#[path = "plan_stage.rs"]
mod stage;
#[path = "plan_overview.rs"]
mod overview_text;
#[cfg(test)]
use overview_text::checks;
use overview_text::overview;
#[path = "plan_pr_body.rs"]
mod pr_body;
#[path = "plan_move.rs"]
mod theme_move;
#[path = "plan_ticket.rs"]
mod ticket;
#[path = "plan_agent.rs"]
pub(crate) mod agent;
#[path = "plan_landing.rs"]
mod landing;

pub(crate) use agent::AgentArgs;
pub(crate) use landing::LandingArgs;

/// What a runner without the layer is told.
const NEEDS_UPDATE: &str = "This runner needs an update to keep a plan.";

/// What a runner with themes and no line is told.
const NEEDS_LINE: &str = "This runner needs an update to keep tickets, agents and landings.";

/// What the overview says on a board with nothing planned.
const NOTHING_PLANNED: &str = "Nothing is planned yet. Make a theme with `farcooler plan theme create`, or put a ticket on the line with `farcooler plan ticket set`.";

/// `farcooler plan`, with no subcommand the overview.
#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    cmd: Option<PlanCmd>,
}

/// The flags every `plan`, `agent` and `landing` command takes.
#[derive(Debug, Clone, Args)]
pub(crate) struct Common {
    /// Which repository's board, when two have the workspace's name.
    #[arg(long, global = true)]
    repo: Option<String>,
    /// The board, by name or task prefix. Read from FARCOOLER_WORKSPACE in a pane.
    #[arg(long, global = true)]
    workspace: Option<String>,
    /// Who this write is from. Read from FARCOOLER_ACTOR when not given.
    #[arg(long, global = true)]
    actor: Option<String>,
}

#[derive(Debug, Clone, Subcommand)]
enum PlanCmd {
    /// Replace the plan: these waiting tickets, in this order. The first is next up.
    ///
    /// A ticket not named leaves the plan. Only a waiting ticket on the line
    /// can be in it. With no tickets, the plan is emptied.
    Set {
        /// Card keys, first is next up.
        tickets: Vec<String>,
    },
    /// Themes: why a group of cards exists.
    #[command(subcommand)]
    Theme(ThemeCmd),
    /// One ticket on the line: its stage, branch, stack, pull request and timeline.
    #[command(subcommand)]
    Ticket(ticket::TicketCmd),
    /// Rulings: calls made for the owner, which stand until they say otherwise.
    #[command(subcommand)]
    Ruling(ruling::RulingCmd),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThemeStateArg {
    Active,
    Paused,
    Done,
    Dropped,
}

#[derive(Debug, Clone, Subcommand)]
enum ThemeCmd {
    /// List the themes on this board.
    List {
        /// Only the theme this card is in.
        #[arg(long)]
        card: Option<String>,
    },
    /// One theme: its outcome, where it stands, what's next, its cards and its tickets.
    Show {
        /// The theme's name, or its short id.
        name: String,
    },
    /// Make a theme. A card is in at most one theme; one already in another moves here.
    Create {
        /// A short name for a row: "Visual language".
        name: String,
        /// One sentence: the world when it's done.
        #[arg(long)]
        outcome: String,
        /// A card to put in it. Repeat for more.
        #[arg(long = "card", value_name = "KEY")]
        cards: Vec<String>,
    },
    /// Change a theme. `--story` replaces the story and keeps the old one.
    Set {
        /// The theme's name, or its short id.
        name: String,
        /// Where it stands, in two to five sentences, for the owner and not as a log.
        #[arg(long)]
        story: Option<String>,
        /// What happens next, in one line.
        #[arg(long)]
        next: Option<String>,
        /// What needs the owner, in one line. The ask itself still goes through `task ask`.
        #[arg(long, conflicts_with = "no_ask")]
        ask: Option<String>,
        /// Nothing needs the owner now.
        #[arg(long)]
        no_ask: bool,
        /// One sentence: the world when it's done.
        #[arg(long)]
        outcome: Option<String>,
        /// Active, paused, done or dropped.
        #[arg(long, value_enum)]
        state: Option<ThemeStateArg>,
        /// A new name.
        #[arg(long)]
        rename: Option<String>,
        /// A token budget: 5000000, 800k or 5M. The plan flags the theme when it goes over.
        #[arg(long, value_parser = cost::parse_budget, conflicts_with = "no_budget")]
        budget: Option<u64>,
        /// Take the budget away.
        #[arg(long)]
        no_budget: bool,
    },
    /// Put a theme at a place in the ranking, 1 being first. The ranking is
    /// the whole team's: anyone may move a theme.
    Move {
        /// The theme's name, or its short id.
        name: String,
        /// Its new place: 1 for first.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
        to: u32,
    },
    /// Add cards to a theme and take others out.
    Cards {
        /// The theme's name, or its short id.
        name: String,
        /// A card to add. Repeat for more.
        #[arg(long = "add", value_name = "KEY")]
        add: Vec<String>,
        /// A card to take out. Repeat for more.
        #[arg(long = "remove", value_name = "KEY")]
        remove: Vec<String>,
    },
}

/// The runner, the board and who is writing, for one `plan`, `agent` or
/// `landing` command.
async fn open(runner: Option<&str>, common: &Common) -> Result<(crate::Link, Board, String), Failed> {
    let mut link = connect_to(runner).await?;
    needs_the_layer(&link.capabilities())?;
    let actor = actor_for(common.actor.as_deref())?.to_string();
    let board =
        board_for(&mut link, common.repo.as_deref(), common.workspace.as_deref(), std::env::var(WORKSPACE_ENV).ok()).await?;
    Ok((link, board, actor))
}

/// `farcooler plan ...`, on a connected runner.
pub async fn plan(runner: Option<&str>, args: PlanArgs, json: bool) -> Fallible {
    let (mut link, board, actor) = open(runner, &args.common).await?;
    println!("{}", run_on(&mut link, &board, args.cmd, &actor, json, now_ms()).await?);
    Ok(())
}

/// `farcooler agent ...`, on a connected runner.
pub async fn agents(runner: Option<&str>, args: AgentArgs, json: bool) -> Fallible {
    let (mut link, board, actor) = open(runner, &args.common).await?;
    println!("{}", agent::run_on(&mut link, &board, args.cmd, &actor, json).await?);
    Ok(())
}

/// `farcooler landing ...`, on a connected runner.
pub async fn landings(runner: Option<&str>, args: LandingArgs, json: bool) -> Fallible {
    let (mut link, board, _) = open(runner, &args.common).await?;
    println!("{}", landing::run_on(&mut link, &board, args.cmd, json, now_ms()).await?);
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Refused before anything is sent: a runner without the layer would answer
/// `CAPABILITY_UNSUPPORTED`, but a refusal here costs no round trip and says
/// what to do.
fn needs_the_layer(capabilities: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if capabilities.iter().any(|c| c == capability::BOARD_PLAN) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

/// The line, agents and landings (`board_line`), refused the same way.
fn needs_the_line(capabilities: &[String]) -> Result<(), Failed> {
    needs_the_layer(capabilities)?;
    if capabilities.iter().any(|c| c == capability::BOARD_LINE) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_LINE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

type Failed = Box<dyn std::error::Error>;

/// One `plan` command on `board`, answering what to print.
async fn run_on<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: Option<PlanCmd>,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_the_layer(&link.capabilities())?;
    // Themes and rulings were kept before the line was; everything else reads it.
    if !matches!(cmd, Some(PlanCmd::Theme(_) | PlanCmd::Ruling(_))) {
        needs_the_line(&link.capabilities())?;
    }
    let (board, ws) = on_a_board(link, board).await?;
    let board = &board;
    match cmd {
        None => {
            let plan = get_plan(link, &ws, false).await?;
            Ok(if json { plan_json(&plan).to_string() } else { overview(&plan, now) })
        }
        Some(PlanCmd::Set { tickets }) => {
            let plan = get_plan(link, &ws, true).await?;
            let ids = ranked(&plan, &tickets)?;
            let set = request::Payload::LineOrder(pb::LineOrder { workspace_id: ws, task_ids: ids, actor: actor.into() });
            let result::Value::Plan(plan) = send(link, board, "line.order", set).await? else { return Err(unreadable()) };
            Ok(if json { plan_json(&plan).to_string() } else { overview(&plan, now) })
        }
        Some(PlanCmd::Theme(cmd)) => theme(link, board, ws, cmd, actor, json, now).await,
        Some(PlanCmd::Ticket(cmd)) => ticket::ticket(link, board, ws, cmd, actor, json, now).await,
        Some(PlanCmd::Ruling(cmd)) => ruling::ruling(link, board, ws, cmd, actor, json, now).await,
    }
}

/// `board`, or when it names no workspace the repository's Main: a plan is one
/// board's, and `task create` files on Main in the same case. With the
/// workspace's id.
async fn on_a_board<L: DispatchLink>(link: &mut L, board: &Board) -> Result<(Board, bytes::Bytes), Failed> {
    let board = if board.workspace.is_some() || !board.has_workspaces {
        board.clone()
    } else {
        let main = workspaces_on(link, Some(board.repository)).await?.into_iter().find(|w| w.is_main);
        Board { workspace: main, ..board.clone() }
    };
    let Some(workspace) = &board.workspace else { return Err("Name a board with --workspace.".into()) };
    let ws = workspace.id.clone();
    Ok((board, ws))
}

fn unreadable() -> Failed {
    crate::daemon_link::UNREADABLE.into()
}

/// The capability a layer method needs: its own for `agent.stop`, the line's
/// for tickets, agents and landings, the plan's for the rest.
fn capability_for(method: &str) -> &'static str {
    if method == "agent.stop" {
        capability::AGENT_STOP
    } else if ["line.", "agent.", "landing."].iter().any(|p| method.starts_with(p)) {
        capability::BOARD_LINE
    } else {
        capability::BOARD_PLAN
    }
}

/// A request for a layer method, with the capability it needs named so an
/// older runner refuses rather than dropping a field.
async fn send<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    method: &str,
    payload: request::Payload,
) -> Result<result::Value, Failed> {
    send_saying(link, board, method, payload, |_| None).await
}

/// `send`, with a child module's sentences for the runner's words said
/// before this module's.
async fn send_saying<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    method: &str,
    payload: request::Payload,
    own: fn(&str) -> Option<&'static str>,
) -> Result<result::Value, Failed> {
    let mut r = with(req_for(method, board.repository), payload);
    r.required_capabilities.push(capability_for(method).to_string());
    let answer =
        link.call(r).await.map_err(|e| refused_saying(e, own, "The runner couldn't record that. Try again."))?;
    expect_value(answer.value)
}

async fn get_plan<L: DispatchLink>(link: &mut L, workspace: &bytes::Bytes, all: bool) -> Result<pb::Plan, Failed> {
    let mut r = with(
        req_for("plan.get", crate::uuid_of(workspace)),
        request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: workspace.clone(), include_closed: all }),
    );
    r.required_capabilities.push(capability::BOARD_PLAN.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, "The runner couldn't read the plan."))?;
    match expect_value(answer.value)? {
        result::Value::Plan(plan) => Ok(plan),
        _ => Err(unreadable()),
    }
}

async fn events_of<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    subject: pb::plan_events_request::Subject,
) -> Result<Vec<pb::PlanEvent>, Failed> {
    let p = request::Payload::PlanEvents(pb::PlanEventsRequest { subject: Some(subject), since_ms: 0 });
    match send(link, board, "plan.events", p).await? {
        result::Value::PlanEventList(list) => Ok(list.events),
        _ => Err(unreadable()),
    }
}

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

/// This module's sentences for the words the runner refuses with. `agent` and
/// `landing` say theirs first, where a word means something of their own.
fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "name" => "A theme's name is a short phrase, and it can't be empty.",
        "name_taken" => "That name is already in use on this board.",
        "place" => "That place isn't in the ranking.",
        "other_board" => "Those tickets aren't all on this board.",
        "stage" => "A ticket can't make that move. It goes waiting, building, in review, landing, landed, and back to building for a fix. Any open ticket can be dropped or wait on you.",
        "ticket_closed" => "That ticket has landed or been dropped, so it takes no more changes.",
        "on_the_line" => "That ticket is already on the line.",
        "not_on_the_line" => "That ticket isn't on the line. Put it there with `farcooler plan ticket set <key>`.",
        "not_waiting" => "Only a waiting ticket can be in the plan.",
        "duplicate" => "A ticket can only be in the plan once.",
        "stack_parent" => "A ticket can only be stacked on another open ticket on the line, never on itself.",
        "stack_cycle" => "That would stack the ticket on one stacked on it.",
        "parent_not_landed" => "Its parent ticket hasn't landed yet, so it lands after that one does.",
        "same_branch" => "It shares a branch with other tickets, so it lands with them through a landing.",
        "not_landed" => "Give a ticket the commit it landed as only once it has landed.",
        "ticket_pr" => "That isn't the ticket's own pull request.",
        "role" => "That work doesn't fit the agent: a builder builds and fixes, a reviewer reviews, and a lander lands.",
        "agent_ended" => "That agent has already finished.",
        "verdict" => "That isn't a way work ends here.",
        "harness" => "A harness is one word, like claude or codex.",
        "agent_id" => "Name the agent with its short id from `farcooler agent list`.",
        "title" => "A title is one short line, up to 80 characters.",
        "state" => "That isn't a state this takes.",
        "actor" => "Use user, manager or agent:<terminal id> for --actor.",
        "outcome" | "next" | "owner_ask" | "reason" => "That's too long for one line. Shorten it.",
        "story" => "That story is too long. Shorten it to a few sentences.",
        "sha" | "path" | "branch" | "model" => "That's too long.",
        "subject" => "Name a theme, a ticket or a landing.",
        _ => return None,
    })
}

/// A refusal from the runner: this module's sentence for a word it knows,
/// else the one `refused` makes.
fn refused_here(err: ClientError, invalid: &str) -> Failed {
    refused_saying(err, |_| None, invalid)
}

/// `refused_here`, with `own` asked first.
fn refused_saying(err: ClientError, own: fn(&str) -> Option<&'static str>, invalid: &str) -> Failed {
    if let ClientError::Daemon { code, what, .. } = &err
        && let Some(said) = own(what).or_else(|| said_here(what)).or_else(|| ruling::said_here(what))
    {
        return Box::new(Refused::naming(said.to_string(), *code, what.clone()));
    }
    refused(err, invalid)
}

// ---------------------------------------------------------------------------
// naming things
// ---------------------------------------------------------------------------

/// Card keys by task id, from whatever the command has read.
struct Keys(HashMap<Vec<u8>, String>);

impl Keys {
    fn of_plan(plan: &pb::Plan) -> Keys {
        Keys(plan.cards.iter().map(|c| (c.task_id.to_vec(), c.key.clone())).collect())
    }

    fn extend(&mut self, tasks: &[pb::Task]) {
        self.0.extend(tasks.iter().map(|t| (t.id.to_vec(), t.key.clone())));
    }

    /// The card's key, or its short id when this command hasn't read it.
    fn of(&self, task: &[u8]) -> String {
        self.0.get(task).cloned().unwrap_or_else(|| short_bytes(task))
    }
}

/// The card a key names on this board. A key that is on another board, or
/// nowhere, is refused with the sentence that says which.
async fn card<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    items: &[pb::Task],
    key: &str,
) -> Result<pb::Task, Failed> {
    let wanted = key.trim().to_lowercase();
    if let Some(found) =
        items.iter().find(|t| t.key.to_lowercase() == wanted || short_bytes(&t.id) == wanted)
    {
        return Ok(found.clone());
    }
    match find_task(link, Some(&board.repository.to_string()), key).await {
        Ok(elsewhere) => Err(format!("{} is on another board.", elsewhere.key).into()),
        Err(_) => Err(format!("No card here is called {key:?}.").into()),
    }
}

async fn ids_of<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    items: &[pb::Task],
    given: &[String],
) -> Result<Vec<bytes::Bytes>, Failed> {
    let mut out = Vec::new();
    for key in given {
        out.push(card(link, board, items, key).await?.id);
    }
    Ok(out)
}

/// The card the plan names by `key`, by its key or its short id.
fn plan_card<'a>(plan: &'a pb::Plan, key: &str) -> Option<&'a pb::PlanCard> {
    let wanted = key.trim().to_lowercase();
    plan.cards.iter().find(|c| c.key.to_lowercase() == wanted || short_bytes(&c.task_id) == wanted)
}

/// A ticket on the line by its card's key, with the card.
fn find_line<'a>(plan: &'a pb::Plan, key: &str) -> Result<(&'a pb::PlanCard, &'a pb::TicketLine), Failed> {
    let off = || -> Failed {
        format!("{} isn't on the line. Put it there with `farcooler plan ticket set {}`.", key.trim(), key.trim()).into()
    };
    let card = plan_card(plan, key).ok_or_else(off)?;
    let line = plan.lines.iter().find(|l| l.task_id == card.task_id).ok_or_else(off)?;
    Ok((card, line))
}

/// A theme by its name, then by its short id, then by the start of its name
/// when only one theme starts that way.
fn find_theme<'a>(plan: &'a pb::Plan, name: &str) -> Result<&'a pb::BoardThemeView, Failed> {
    let wanted = name.trim().to_lowercase();
    let named = |v: &&pb::BoardThemeView| v.theme.as_ref().map(|t| t.name.to_lowercase());
    if let Some(v) = plan.themes.iter().find(|v| named(v).is_some_and(|n| n == wanted)) {
        return Ok(v);
    }
    if let Some(v) = plan.themes.iter().find(|v| v.theme.as_ref().is_some_and(|t| short_bytes(&t.id) == wanted)) {
        return Ok(v);
    }
    let starting: Vec<&pb::BoardThemeView> =
        plan.themes.iter().filter(|v| !wanted.is_empty() && named(v).is_some_and(|n| n.starts_with(&wanted))).collect();
    match starting.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("No theme here is called {name:?}.").into()),
        many => Err(format!("{name:?} starts {} themes. Say more of the name.", many.len()).into()),
    }
}

/// The tickets `plan set` names, in order: each one waiting on the line and
/// named once.
fn ranked(plan: &pb::Plan, keys: &[String]) -> Result<Vec<bytes::Bytes>, Failed> {
    let mut ids: Vec<bytes::Bytes> = Vec::new();
    for key in keys {
        let (card, line) = find_line(plan, key)?;
        if line.stage != pb::TicketStage::Waiting as i32 {
            return Err(format!(
                "Only a waiting ticket can be in the plan: {} is {}.",
                card.key,
                ticket::stage_phrase(line.stage)
            )
            .into());
        }
        if ids.contains(&card.task_id) {
            return Err(format!("{} is named twice. A ticket can only be in the plan once.", card.key).into());
        }
        ids.push(card.task_id.clone());
    }
    Ok(ids)
}

// ---------------------------------------------------------------------------
// themes
// ---------------------------------------------------------------------------

async fn theme<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    cmd: ThemeCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    let plan = get_plan(link, &ws, true).await?;
    match cmd {
        ThemeCmd::List { card: wanted } => {
            let mut views: Vec<&pb::BoardThemeView> = plan.themes.iter().collect();
            if let Some(key) = wanted {
                let items = board_in(link, board, None, None).await?.items;
                let task = card(link, board, &items, &key).await?;
                views.retain(|v| v.task_ids.contains(&task.id));
            }
            Ok(if json {
                Value::Array(views.iter().map(|v| theme_json(&plan, v, &[])).collect()).to_string()
            } else if views.is_empty() {
                "No themes here.".to_string()
            } else {
                views.iter().map(|v| theme_row(v)).collect::<Vec<_>>().join("\n")
            })
        }
        ThemeCmd::Show { name } => {
            let view = find_theme(&plan, &name)?;
            let id = view.theme.as_ref().map(|t| t.id.clone()).unwrap_or_default();
            let events = events_of(link, board, pb::plan_events_request::Subject::ThemeId(id)).await?;
            Ok(if json {
                let mut out = theme_json(&plan, view, &[]);
                out["events"] = events_json(&events);
                out.to_string()
            } else {
                theme_text(view, &plan, &events, now)
            })
        }
        ThemeCmd::Create { name, outcome, cards } => {
            let items = board_in(link, board, None, None).await?.items;
            let task_ids = ids_of(link, board, &items, &cards).await?;
            let p = request::Payload::BoardThemeCreate(pb::BoardThemeCreate {
                workspace_id: ws,
                name,
                outcome,
                task_ids,
                actor: actor.into(),
            });
            let result::Value::BoardThemeView(view) = send(link, board, "board_theme.create", p).await? else {
                return Err(unreadable());
            };
            Ok(if json {
                theme_json(&plan, &view, &items).to_string()
            } else {
                format!("Made theme {:?} with {}.", theme_name(&view), count(view.task_ids.len(), "card"))
            })
        }
        ThemeCmd::Set { name, story, next, ask, no_ask, outcome, state, rename, budget, no_budget } => {
            let budget = cost::asked(budget, no_budget);
            if story.is_none()
                && next.is_none()
                && ask.is_none()
                && !no_ask
                && outcome.is_none()
                && state.is_none()
                && rename.is_none()
                && budget.is_none()
            {
                return Err("Say what to change: --story, --next, --ask, --no-ask, --outcome, --state, --rename or --budget.".into());
            }
            if budget.is_some() {
                cost::needs_cost(link)?;
            }
            let id = find_theme(&plan, &name)?.theme.as_ref().map(|t| t.id.clone()).unwrap_or_default();
            let p = request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate {
                theme_id: id,
                name: rename,
                outcome,
                story,
                next,
                owner_ask: if no_ask { Some(String::new()) } else { ask },
                state: state.map(|s| theme_state(s) as i32),
                ordinal: None,
                actor: actor.into(),
                budget_tokens: budget,
            });
            let result::Value::BoardThemeView(view) = send(link, board, "board_theme.update", p).await? else {
                return Err(unreadable());
            };
            Ok(if json { theme_json(&plan, &view, &[]).to_string() } else { format!("Updated theme {:?}.", theme_name(&view)) })
        }
        ThemeCmd::Move { name, to } => theme_move::move_theme(link, board, &plan, &name, to, actor, json).await,
        ThemeCmd::Cards { name, add, remove } => {
            if add.is_empty() && remove.is_empty() {
                return Err("Name a card with --add or --remove.".into());
            }
            let id = find_theme(&plan, &name)?.theme.as_ref().map(|t| t.id.clone()).unwrap_or_default();
            let items = board_in(link, board, None, None).await?.items;
            let p = request::Payload::BoardThemeCards(pb::BoardThemeCards {
                theme_id: id,
                add: ids_of(link, board, &items, &add).await?,
                remove: ids_of(link, board, &items, &remove).await?,
                actor: actor.into(),
            });
            let result::Value::BoardThemeView(view) = send(link, board, "board_theme.cards", p).await? else {
                return Err(unreadable());
            };
            Ok(if json {
                theme_json(&plan, &view, &items).to_string()
            } else {
                format!("Theme {:?} has {}.", theme_name(&view), count(view.task_ids.len(), "card"))
            })
        }
    }
}

fn theme_state(arg: ThemeStateArg) -> pb::BoardThemeState {
    match arg {
        ThemeStateArg::Active => pb::BoardThemeState::Active,
        ThemeStateArg::Paused => pb::BoardThemeState::Paused,
        ThemeStateArg::Done => pb::BoardThemeState::Done,
        ThemeStateArg::Dropped => pb::BoardThemeState::Dropped,
    }
}

fn theme_name(view: &pb::BoardThemeView) -> String {
    view.theme.as_ref().map(|t| t.name.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// words
// ---------------------------------------------------------------------------

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// A ticket's stage, as a person reads it.
fn stage_word(stage: i32) -> &'static str {
    match pb::TicketStage::try_from(stage) {
        Ok(pb::TicketStage::Waiting) => "Waiting",
        Ok(pb::TicketStage::Building) => "Building",
        Ok(pb::TicketStage::Reviewing) => "In review",
        Ok(pb::TicketStage::NeedsYou) => "Needs you",
        Ok(pb::TicketStage::Landing) => "Landing",
        Ok(pb::TicketStage::Landed) => "Landed",
        Ok(pb::TicketStage::Dropped) => "Dropped",
        _ => "Unknown",
    }
}

/// Whether a ticket is still being worked: not landed or dropped.
fn is_live(stage: i32) -> bool {
    stage != pb::TicketStage::Landed as i32 && stage != pb::TicketStage::Dropped as i32
}

/// An agent's role, as a person reads it.
fn role_word(role: i32) -> &'static str {
    match pb::AgentRole::try_from(role) {
        Ok(pb::AgentRole::Orchestrator) => "Orchestrator",
        Ok(pb::AgentRole::Builder) => "Builder",
        Ok(pb::AgentRole::Reviewer) => "Reviewer",
        Ok(pb::AgentRole::Lander) => "Lander",
        _ => "Agent",
    }
}

fn status_word(status: i32) -> &'static str {
    match pb::TaskStatus::try_from(status) {
        Ok(pb::TaskStatus::Backlog) => "Backlog",
        Ok(pb::TaskStatus::Todo) => "To Do",
        Ok(pb::TaskStatus::NeedsDecision) => "Needs Decision",
        Ok(pb::TaskStatus::InProgress) => "In Progress",
        Ok(pb::TaskStatus::InReview) => "In Review",
        Ok(pb::TaskStatus::Done) => "Done",
        Ok(pb::TaskStatus::Cancelled) => "Cancelled",
        _ => "Unknown",
    }
}

fn is_open(status: i32) -> bool {
    status != pb::TaskStatus::Done as i32 && status != pb::TaskStatus::Cancelled as i32
}

/// "5m", "3h", "2d": how long, coarsely.
fn age(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "under a minute".to_string(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

/// "just now", or "5m ago".
fn ago(ms: i64) -> String {
    if ms.max(0) < 60_000 { "just now".to_string() } else { format!("{} ago", age(ms)) }
}

fn total(s: &pb::Spend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

fn spend_words(spend: &pb::Spend) -> String {
    if total(spend) == 0 {
        return NOT_REPORTED.to_string();
    }
    let mut said = format!("{} tokens", tokens(total(spend)));
    if let Some(micros) = spend.cost_micros {
        said.push_str(&format!(" · {} API-equivalent", dollars(micros)));
    }
    if spend.unmeasured_agents > 0 {
        said.push_str(&format!(" · {} not reported", count(spend.unmeasured_agents as usize, "agent")));
    }
    said.push_str(&shared_words(spend.shared_agents));
    said
}

/// " · 1 agent's spend split with other tickets": an agent that worked several
/// tickets counts once across them, evenly, and each ticket's figure says it
/// holds a part. Empty when no agent is shared.
fn shared_words(shared: u32) -> String {
    match shared {
        0 => String::new(),
        1 => " · 1 agent\u{2019}s spend split with other tickets".to_string(),
        n => format!(" · {n} agents\u{2019} spend split with other tickets"),
    }
}

/// "1 of 2 done": the cancelled cards left out, as the Mac's `PlanWords.total`
/// leaves them (ov-273).
fn done_of(view: &pb::BoardThemeView) -> String {
    let c = view.counts.unwrap_or_default();
    let total = c.backlog + c.todo + c.needs_decision + c.in_progress + c.in_review + c.done;
    format!("{} of {} done", c.done, total)
}

fn theme_row(view: &pb::BoardThemeView) -> String {
    let t = view.theme.clone().unwrap_or_default();
    let mut row = format!("{}  {} · {}", t.name, done_of(view), state_of_theme(t.state));
    if !t.next.is_empty() {
        row.push_str(&format!(" · Next: {}", t.next));
    }
    if !t.owner_ask.is_empty() {
        row.push_str(&format!(" · Needs you: {}", t.owner_ask));
    }
    if let Some(budget) = cost::budget_words(&view.spend.unwrap_or_default(), view.budget_tokens) {
        row.push_str(&format!(" · {budget}"));
    }
    row
}

fn state_of_theme(state: i32) -> &'static str {
    match pb::BoardThemeState::try_from(state) {
        Ok(pb::BoardThemeState::Active) => "active",
        Ok(pb::BoardThemeState::Paused) => "paused",
        Ok(pb::BoardThemeState::Done) => "done",
        Ok(pb::BoardThemeState::Dropped) => "dropped",
        _ => "unknown",
    }
}

fn theme_text(view: &pb::BoardThemeView, plan: &pb::Plan, events: &[pb::PlanEvent], now: i64) -> String {
    let keys = Keys::of_plan(plan);
    let t = view.theme.clone().unwrap_or_default();
    let mut out = vec![format!("{} · {}", t.name, state_of_theme(t.state))];
    if !t.outcome.is_empty() {
        out.push(t.outcome.clone());
    }
    out.push(String::new());
    if !t.story.is_empty() {
        let when = if t.story_at > 0 { format!(" (updated {})", ago(now - t.story_at)) } else { String::new() };
        out.push(format!("Where it stands{when}: {}", t.story));
    }
    if let Some(prev) = events.iter().rev().find(|e| e.kind == "story" && !e.body.is_empty()) {
        out.push(format!("Before that: {}", prev.body));
    }
    if !t.next.is_empty() {
        out.push(format!("Next: {}", t.next));
    }
    if !t.owner_ask.is_empty() {
        out.push(format!("Needs you: {}", t.owner_ask));
    }
    let lines: Vec<&pb::TicketLine> = plan.lines.iter().filter(|l| view.task_ids.contains(&l.task_id)).collect();
    if !lines.is_empty() {
        out.push(String::new());
        out.push("On the line".into());
        out.extend(lines.iter().map(|l| format!("  {}", overview_text::ticket_row(plan, l, now))));
    }
    let spend = view.spend.unwrap_or_default();
    if spend.runs > 0 || view.budget_tokens.is_some() {
        out.push(String::new());
        out.push(format!("Spend  {}", spend_words(&spend)));
        out.extend(cost::budget_words(&spend, view.budget_tokens).map(|b| format!("Budget  {b}")));
        out.extend(cost::trend_words(&view.trend_tokens).map(|t| format!("Last 7 days  {t}")));
    }
    out.push(String::new());
    out.push(format!("Cards · {}", done_of(view)));
    for id in &view.task_ids {
        match plan.cards.iter().find(|c| c.task_id == *id) {
            Some(c) => out.push(format!("  {}  {}  {}", c.key, c.title, status_word(c.status))),
            None => out.push(format!("  {}", keys.of(id))),
        }
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------
// json: the client's shapes, the apps' contract
// ---------------------------------------------------------------------------

fn id_text(id: &[u8]) -> String {
    uuid_of(id).to_string()
}

/// The whole plan as the apps decode it.
fn plan_json(plan: &pb::Plan) -> Value {
    farcooler_client::plan_json::plan_json(plan)
}

/// A plan of only `plan`'s cards and `items`, for drawing one record of it in
/// the client's shape with every card named by its key.
fn cards_only(plan: &pb::Plan, items: &[pb::Task]) -> pb::Plan {
    let mut cards = plan.cards.clone();
    for t in items.iter().filter(|t| !plan.cards.iter().any(|c| c.task_id == t.id)) {
        cards.push(pb::PlanCard { task_id: t.id.clone(), key: t.key.clone(), title: t.title.clone(), status: t.status });
    }
    pb::Plan { now_ms: plan.now_ms, cards, ..Default::default() }
}

/// A theme as `plan --json` carries it, its cards named from `plan` and `items`.
fn theme_json(plan: &pb::Plan, view: &pb::BoardThemeView, items: &[pb::Task]) -> Value {
    let one = pb::Plan { themes: vec![view.clone()], ..cards_only(plan, items) };
    plan_json(&one)["themes"][0].take()
}

/// A ticket on the line as `plan --json` carries it.
fn line_json(plan: &pb::Plan, line: &pb::TicketLine, items: &[pb::Task]) -> Value {
    let one = pb::Plan { lines: vec![line.clone()], ..cards_only(plan, items) };
    plan_json(&one)["lines"][0].take()
}

/// A timeline as `plan.events` carries it.
fn events_json(events: &[pb::PlanEvent]) -> Value {
    farcooler_client::plan_json::events_json(&pb::PlanEventList { events: events.to_vec() })["events"].take()
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_waits_tests.rs"]
mod waits_tests;

#[cfg(test)]
#[path = "plan_suggest_tests.rs"]
mod suggest_tests;

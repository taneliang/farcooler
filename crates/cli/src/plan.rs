//! `farcooler plan`: themes, lanes and the plan (ov-268).
//!
//! **Experimental, and beside the board.** A theme is why a group of cards
//! exists. A lane is one unit of execution: an agent, or a chain of them, in
//! one worktree on one branch. The plan is the board's ordered list of queued
//! lanes. The orchestrator writes all of it, and the `task` verbs are
//! unchanged: nothing here edits a card, and no card carries any of it.
//!
//! ```text
//! farcooler plan                              the overview an app draws
//! farcooler plan set LANE...                  queued lanes, first is next up
//! farcooler plan theme list|show|create|set|cards
//! farcooler plan lane  list|show|start|set|cards
//! farcooler plan ruling add|set|list           decided for you (ov-304)
//! farcooler plan train start|set|list          trains and their CI (ov-309)
//! ```
//!
//! Nested under `plan` because `farcooler theme` already lists the terminal's
//! color schemes. Every command takes `--repo`, `--workspace` and `--actor`
//! with the meanings `task` gives them (`--runner` and `--json` are the
//! top-level flags), and names cards the way `task` does. A lane is named by
//! its name, a theme by its name or short id.
//!
//! One read draws the overview, so a heartbeat reads `plan --json` instead of
//! `task list` plus a `task show` per card. Writes are one command per lane
//! event and one per theme checkpoint.

use std::collections::HashMap;

use clap::{Args, Subcommand, ValueEnum};
use farcooler_core::usage_words::{NOT_REPORTED, dollars, tokens};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use serde_json::{Value, json};

use crate::tasks::{Board, DispatchLink, Refused, actor_for, board_for, board_in, find_task, refused};
use crate::workspaces::{WORKSPACE_ENV, workspaces_on};
use crate::{Fallible, connect_to, expect_value, req_for, short_bytes, uuid_of, with};

#[path = "plan_ruling.rs"]
mod ruling;
#[path = "plan_train.rs"]
mod train;
#[path = "plan_cost.rs"]
mod cost;

/// What a runner without the layer is told.
const NEEDS_UPDATE: &str = "This runner needs an update to keep a plan.";

/// What the overview says on a board with no theme and no lane.
const NOTHING_PLANNED: &str = "Nothing is planned yet. Make a theme with `farcooler plan theme create`, or a lane with `farcooler plan lane start`.";

/// `farcooler plan`, with no subcommand the overview.
#[derive(Debug, Clone, Args)]
pub struct PlanArgs {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    cmd: Option<PlanCmd>,
}

/// The flags every `plan` command takes.
#[derive(Debug, Clone, Args)]
struct Common {
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
    /// Replace the plan: these queued lanes, in this order. The first is next up.
    ///
    /// A lane not named is out of the plan. Only a queued lane can be in it.
    /// With no lanes, the plan is emptied.
    Set {
        /// Lane names, first is next up.
        lanes: Vec<String>,
    },
    /// Themes: why a group of cards exists.
    #[command(subcommand)]
    Theme(ThemeCmd),
    /// Lanes: one agent, or a chain of them, in one worktree on one branch.
    #[command(subcommand)]
    Lane(LaneCmd),
    /// Rulings: calls made for the owner, which stand until they say otherwise.
    #[command(subcommand)]
    Ruling(ruling::RulingCmd),
    /// Trains: lanes landing together, with the CI the runner reads for them.
    #[command(subcommand)]
    Train(train::TrainCmd),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ThemeStateArg {
    Active,
    Paused,
    Done,
    Dropped,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LaneStateArg {
    Queued,
    Building,
    Review,
    Fixing,
    Landing,
    Landed,
    Dropped,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RoleArg {
    Build,
    Review,
    Fix,
}

#[derive(Debug, Clone, Subcommand)]
enum ThemeCmd {
    /// List the themes on this board.
    List {
        /// Only the theme this card is in.
        #[arg(long)]
        card: Option<String>,
    },
    /// One theme: its outcome, where it stands, what's next, its cards and its lanes.
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

#[derive(Debug, Clone, Subcommand)]
enum LaneCmd {
    /// List the lanes that haven't landed or been dropped.
    List {
        /// Include the ones that have.
        #[arg(long)]
        all: bool,
        /// Only lanes working this card.
        #[arg(long)]
        card: Option<String>,
    },
    /// One lane: its cards, agents, spend and timeline.
    Show {
        /// The lane's name.
        name: String,
    },
    /// Make a lane. With `--agent` it starts building; without, it's queued.
    ///
    /// A card is `KEY` for the whole card, or `KEY:SLICE` for one slice of it
    /// ("ov-113:s3-4 phones"). The agent is also recorded as a worker on the
    /// first card that's still open, so the runner reads its spend.
    Start {
        /// One word, no spaces: "mac-ux", "ov-113-phones".
        name: String,
        /// A card the lane works. Repeat for more.
        #[arg(long = "card", value_name = "KEY[:SLICE]")]
        cards: Vec<String>,
        /// One line: why it's in the plan.
        #[arg(long)]
        reason: Option<String>,
        /// The lane's worktree, as the orchestrator gave it.
        #[arg(long)]
        path: Option<String>,
        /// Its branch.
        #[arg(long)]
        branch: Option<String>,
        /// `claude` or `codex`. Claude when an agent is given and this isn't.
        #[arg(long)]
        harness: Option<String>,
        /// The build agent's model: "sonnet", "opus".
        #[arg(long)]
        model: Option<String>,
        /// The build agent's id, from its launch result.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Move a lane, reword it, put it on a train, or record an agent.
    ///
    /// `--agent ID --role review` records a reviewer and moves the lane to
    /// review in the same write (`--role fix` to fixing).
    Set {
        /// The lane's name.
        name: String,
        /// Queued, building, review, fixing, landing, landed or dropped.
        #[arg(long, value_enum)]
        state: Option<LaneStateArg>,
        /// One line: why it's where it is now.
        #[arg(long)]
        reason: Option<String>,
        /// The train it's landing in: "integ-9".
        #[arg(long, conflicts_with = "no_train")]
        train: Option<String>,
        /// Take it out of its train.
        #[arg(long)]
        no_train: bool,
        /// The commit it landed as.
        #[arg(long)]
        sha: Option<String>,
        /// An agent working the lane: its id.
        #[arg(long)]
        agent: Option<String>,
        /// What that agent does: build, review or fix.
        #[arg(long, value_enum, requires = "agent")]
        role: Option<RoleArg>,
        /// That agent's model.
        #[arg(long, requires = "agent")]
        model: Option<String>,
        /// `claude` or `codex`, for that agent.
        #[arg(long, requires = "agent")]
        harness: Option<String>,
        /// That agent has finished.
        #[arg(long, requires = "agent")]
        ended: bool,
        /// The lane's worktree.
        #[arg(long)]
        path: Option<String>,
        /// Its branch.
        #[arg(long)]
        branch: Option<String>,
        /// A token budget: 5000000, 800k or 5M. The plan flags the lane when it goes over.
        #[arg(long, value_parser = cost::parse_budget, conflicts_with = "no_budget")]
        budget: Option<u64>,
        /// Take the budget away.
        #[arg(long)]
        no_budget: bool,
    },
    /// Add cards to a lane and take others off it.
    Cards {
        /// The lane's name.
        name: String,
        /// A card to add, `KEY` or `KEY:SLICE`. Repeat for more.
        #[arg(long = "add", value_name = "KEY[:SLICE]")]
        add: Vec<String>,
        /// A card to take off, `KEY` or `KEY:SLICE`. Repeat for more.
        #[arg(long = "remove", value_name = "KEY[:SLICE]")]
        remove: Vec<String>,
    },
}

/// `farcooler plan ...`, on a connected runner.
pub async fn plan(runner: Option<&str>, args: PlanArgs, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    needs_the_layer(&link.capabilities())?;
    let actor = actor_for(args.common.actor.as_deref())?.to_string();
    let board = board_for(
        &mut link,
        args.common.repo.as_deref(),
        args.common.workspace.as_deref(),
        std::env::var(WORKSPACE_ENV).ok(),
    )
    .await?;
    println!("{}", run_on(&mut link, &board, args.cmd, &actor, json, now_ms()).await?);
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

type Failed = Box<dyn std::error::Error>;

/// One command on `board`, answering what to print.
async fn run_on<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    cmd: Option<PlanCmd>,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    needs_the_layer(&link.capabilities())?;
    let board = &on_a_board(link, board).await?;
    let Some(workspace) = &board.workspace else { return Err("Name a board with --workspace.".into()) };
    let ws = workspace.id.clone();
    match cmd {
        None => {
            let plan = get_plan(link, &ws, false).await?;
            Ok(if json { plan_json(&plan, &Keys::of_plan(&plan)).to_string() } else { overview(&plan, now) })
        }
        Some(PlanCmd::Set { lanes }) => {
            let plan = get_plan(link, &ws, true).await?;
            let ids = ranked(&plan, &lanes)?;
            let set = request::Payload::PlanSet(pb::PlanSet { workspace_id: ws, lane_ids: ids, actor: actor.into() });
            let result::Value::Plan(plan) = send(link, board, "plan.set", set).await? else { return Err(unreadable()) };
            Ok(if json { plan_json(&plan, &Keys::of_plan(&plan)).to_string() } else { overview(&plan, now) })
        }
        Some(PlanCmd::Theme(cmd)) => theme(link, board, ws, cmd, actor, json, now).await,
        Some(PlanCmd::Lane(cmd)) => lane(link, board, ws, cmd, actor, json, now).await,
        Some(PlanCmd::Ruling(cmd)) => ruling::ruling(link, board, ws, cmd, actor, json, now).await,
        Some(PlanCmd::Train(cmd)) => train::train(link, board, ws, cmd, actor, json, now).await,
    }
}

/// `board`, or when it names no workspace the repository's Main: a plan is one
/// board's, and `task create` files on Main in the same case.
async fn on_a_board<L: DispatchLink>(link: &mut L, board: &Board) -> Result<Board, Failed> {
    if board.workspace.is_some() || !board.has_workspaces {
        return Ok(board.clone());
    }
    let main = workspaces_on(link, Some(board.repository)).await?.into_iter().find(|w| w.is_main);
    Ok(Board { workspace: main, ..board.clone() })
}

fn unreadable() -> Failed {
    crate::daemon_link::UNREADABLE.into()
}

/// A request for a layer method, with the capability it needs named so an
/// older runner refuses rather than dropping a field.
async fn send<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    method: &str,
    payload: request::Payload,
) -> Result<result::Value, Failed> {
    let mut r = with(req_for(method, board.repository), payload);
    r.required_capabilities.push(capability::BOARD_PLAN.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, "The runner couldn't record that. Try again."))?;
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

/// This module's sentences for the words the runner refuses with.
fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "name" => "A lane's name is one word with no spaces, and a theme's is a short phrase. Neither can be empty.",
        "name_taken" => "That name is already in use on this board.",
        "other_board" => "Those cards aren't all on this board.",
        "lane_state" => "A lane can't make that move. It goes queued, building, review, landing, landed, with fixing between review and either landing or landed. It can be dropped until it lands.",
        "lane_closed" => "That lane has landed or been dropped, so it takes no more changes.",
        "lane_twice" => "A lane can only be in the plan once.",
        "plan_state" => "Only a queued lane can be in the plan.",
        "harness" => "Use claude or codex.",
        "agent_id" => "Name the agent with --agent, the id from its launch result.",
        "role" => "Use build, review or fix.",
        "state" => "That isn't a state this takes.",
        "actor" => "Use user, manager or agent:<terminal id> for --actor.",
        "outcome" | "next" | "owner_ask" | "reason" => "That's too long for one line. Shorten it.",
        "story" => "That story is too long. Shorten it to a few sentences.",
        "train" | "sha" | "path" | "branch" | "model" | "slice" => "That's too long.",
        "subject" => "Name a theme or a lane.",
        _ => return None,
    })
}

/// A refusal from the runner: this module's sentence for a word it knows,
/// else the one `refused` makes.
fn refused_here(err: ClientError, invalid: &str) -> Failed {
    if let ClientError::Daemon { code, what, .. } = &err
        && let Some(said) = said_here(what).or_else(|| ruling::said_here(what))
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

/// A card as `--card` says it: `KEY`, or `KEY:SLICE`.
fn split_card(text: &str) -> (&str, String) {
    match text.split_once(':') {
        Some((key, slice)) => (key.trim(), slice.trim().to_string()),
        None => (text.trim(), String::new()),
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

async fn cards_of<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    items: &[pb::Task],
    given: &[String],
) -> Result<Vec<pb::LaneCard>, Failed> {
    let mut out = Vec::new();
    for text in given {
        let (key, slice) = split_card(text);
        let task = card(link, board, items, key).await?;
        out.push(pb::LaneCard { task_id: task.id, slice });
    }
    Ok(out)
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

fn find_lane<'a>(plan: &'a pb::Plan, name: &str) -> Result<&'a pb::Lane, Failed> {
    let wanted = name.trim().to_lowercase();
    let named: Vec<&pb::Lane> = plan.lanes.iter().filter(|l| l.name.to_lowercase() == wanted).collect();
    // A name is free again once its lane has landed, so more than one can
    // match: the live one is the one meant.
    match named.iter().find(|l| is_live(l.state)).or(named.first()) {
        Some(lane) => Ok(lane),
        None => match plan.lanes.iter().find(|l| short_bytes(&l.id) == wanted) {
            Some(lane) => Ok(lane),
            None => Err(format!("No lane here is called {name:?}.").into()),
        },
    }
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

/// The lanes `plan set` names, in order, each one queued and named once.
fn ranked(plan: &pb::Plan, names: &[String]) -> Result<Vec<bytes::Bytes>, Failed> {
    let mut ids: Vec<bytes::Bytes> = Vec::new();
    for name in names {
        let lane = find_lane(plan, name)?;
        if lane.state != pb::LaneState::Queued as i32 {
            return Err(format!(
                "Only a queued lane can be in the plan: {} is {}.",
                lane.name,
                state_word(lane.state).to_lowercase()
            )
            .into());
        }
        if ids.contains(&lane.id) {
            return Err(format!("{} is named twice. A lane can only be in the plan once.", lane.name).into());
        }
        ids.push(lane.id.clone());
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
    let mut keys = Keys::of_plan(&plan);
    match cmd {
        ThemeCmd::List { card: wanted } => {
            let mut views: Vec<&pb::BoardThemeView> = plan.themes.iter().collect();
            if let Some(key) = wanted {
                let items = board_in(link, board, None, None).await?.items;
                let task = card(link, board, &items, &key).await?;
                views.retain(|v| v.task_ids.contains(&task.id));
            }
            Ok(if json {
                Value::Array(views.iter().map(|v| theme_json(v, &keys)).collect()).to_string()
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
                let mut out = theme_json(view, &keys);
                out["events"] = events.iter().map(event_json).collect();
                out.to_string()
            } else {
                theme_text(view, &plan, &keys, &events, now)
            })
        }
        ThemeCmd::Create { name, outcome, cards } => {
            let items = board_in(link, board, None, None).await?.items;
            keys.extend(&items);
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
                theme_json(&view, &keys).to_string()
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
            Ok(if json { theme_json(&view, &keys).to_string() } else { format!("Updated theme {:?}.", theme_name(&view)) })
        }
        ThemeCmd::Cards { name, add, remove } => {
            if add.is_empty() && remove.is_empty() {
                return Err("Name a card with --add or --remove.".into());
            }
            let id = find_theme(&plan, &name)?.theme.as_ref().map(|t| t.id.clone()).unwrap_or_default();
            let items = board_in(link, board, None, None).await?.items;
            keys.extend(&items);
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
                theme_json(&view, &keys).to_string()
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
// lanes
// ---------------------------------------------------------------------------

async fn lane<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: bytes::Bytes,
    cmd: LaneCmd,
    actor: &str,
    json: bool,
    now: i64,
) -> Result<String, Failed> {
    let plan = get_plan(link, &ws, true).await?;
    let mut keys = Keys::of_plan(&plan);
    match cmd {
        LaneCmd::List { all, card: wanted } => {
            let mut lanes: Vec<&pb::Lane> = plan.lanes.iter().filter(|l| all || is_live(l.state)).collect();
            if let Some(key) = wanted {
                let items = board_in(link, board, None, None).await?.items;
                let task = card(link, board, &items, &key).await?;
                lanes.retain(|l| l.cards.iter().any(|c| c.task_id == task.id));
            }
            Ok(if json {
                Value::Array(lanes.iter().map(|l| lane_json(l, &keys)).collect()).to_string()
            } else if lanes.is_empty() {
                "No lanes here.".to_string()
            } else {
                lanes.iter().map(|l| lane_row(l, &keys, now)).collect::<Vec<_>>().join("\n")
            })
        }
        LaneCmd::Show { name } => {
            let found = find_lane(&plan, &name)?;
            let events = events_of(link, board, pb::plan_events_request::Subject::LaneId(found.id.clone())).await?;
            Ok(if json {
                let mut out = lane_json(found, &keys);
                out["events"] = events.iter().map(event_json).collect();
                out.to_string()
            } else {
                lane_text(found, &keys, &events, now)
            })
        }
        LaneCmd::Start { name, cards, reason, path, branch, harness, model, agent } => {
            let items = board_in(link, board, None, None).await?.items;
            keys.extend(&items);
            let cards = cards_of(link, board, &items, &cards).await?;
            let harness = harness.or_else(|| agent.is_some().then(|| "claude".to_string())).unwrap_or_default();
            let record = agent.map(|id| pb::LaneAgentRecord {
                harness: harness.clone(),
                agent_id: id,
                role: pb::LaneAgentRole::Build as i32,
                model: model.clone().filter(|m| !m.is_empty()),
                ended: false,
            });
            let p = request::Payload::LaneCreate(pb::LaneCreate {
                workspace_id: ws,
                name,
                reason: reason.unwrap_or_default(),
                cards,
                worktree_path: path.unwrap_or_default(),
                branch: branch.unwrap_or_default(),
                harness,
                model: model.unwrap_or_default(),
                worktree_id: None,
                agent: record,
                actor: actor.into(),
            });
            let result::Value::Lane(made) = send(link, board, "lane.create", p).await? else {
                return Err(unreadable());
            };
            Ok(if json {
                lane_json(&made, &keys).to_string()
            } else {
                format!("Started lane {} ({}, {}).", made.name, state_word(made.state).to_lowercase(), count(made.cards.len(), "card"))
            })
        }
        LaneCmd::Set { name, state, reason, train, no_train, sha, agent, role, model, harness, ended, path, branch, budget, no_budget } => {
            let budget = cost::asked(budget, no_budget);
            let found = find_lane(&plan, &name)?;
            let asked_state = state.map(lane_state);
            // A reviewer or a fix round means the lane is in review or fixing,
            // so the move rides in the same write.
            let implied = role.and_then(|r| match r {
                RoleArg::Review => Some(pb::LaneState::Review),
                RoleArg::Fix => Some(pb::LaneState::Fixing),
                RoleArg::Build => None,
            });
            let record = agent.map(|id| pb::LaneAgentRecord {
                harness: harness.unwrap_or_else(|| "claude".into()),
                agent_id: id,
                role: match role.unwrap_or(RoleArg::Build) {
                    RoleArg::Build => pb::LaneAgentRole::Build,
                    RoleArg::Review => pb::LaneAgentRole::Review,
                    RoleArg::Fix => pb::LaneAgentRole::Fix,
                } as i32,
                model: model.filter(|m| !m.is_empty()),
                ended,
            });
            let state = asked_state.or(implied).filter(|s| *s as i32 != found.state || asked_state.is_some());
            if state.is_none()
                && reason.is_none()
                && train.is_none()
                && !no_train
                && sha.is_none()
                && record.is_none()
                && path.is_none()
                && branch.is_none()
                && budget.is_none()
            {
                return Err("Say what to change: --state, --reason, --train, --no-train, --sha, --agent, --path, --branch or --budget.".into());
            }
            if budget.is_some() {
                cost::needs_cost(link)?;
            }
            if let Some(to) = state {
                refuse_a_move(found, to)?;
            }
            let p = request::Payload::LaneUpdate(pb::LaneUpdate {
                lane_id: found.id.clone(),
                state: state.map(|s| s as i32),
                reason,
                train: if no_train { Some(String::new()) } else { train },
                landed_sha: sha,
                worktree_path: path,
                branch,
                worktree_id: None,
                agent: record,
                actor: actor.into(),
                budget_tokens: budget,
            });
            let result::Value::Lane(moved) = send(link, board, "lane.update", p).await? else {
                return Err(unreadable());
            };
            Ok(if json {
                lane_json(&moved, &keys).to_string()
            } else {
                format!("Lane {} is {}.", moved.name, state_word(moved.state).to_lowercase())
            })
        }
        LaneCmd::Cards { name, add, remove } => {
            if add.is_empty() && remove.is_empty() {
                return Err("Name a card with --add or --remove.".into());
            }
            let found = find_lane(&plan, &name)?;
            let items = board_in(link, board, None, None).await?.items;
            keys.extend(&items);
            let p = request::Payload::LaneCards(pb::LaneCards {
                lane_id: found.id.clone(),
                add: cards_of(link, board, &items, &add).await?,
                remove: cards_of(link, board, &items, &remove).await?,
                actor: actor.into(),
            });
            let result::Value::Lane(changed) = send(link, board, "lane.cards", p).await? else {
                return Err(unreadable());
            };
            Ok(if json {
                lane_json(&changed, &keys).to_string()
            } else {
                format!("Lane {} has {}.", changed.name, count(changed.cards.len(), "card"))
            })
        }
    }
}

/// A move the state machine refuses, said before sending with where the lane
/// is and where it can go. The machine is the store's (`LaneState::can_move_to`),
/// so this can't disagree with it.
fn refuse_a_move(lane: &pb::Lane, to: pb::LaneState) -> Result<(), Failed> {
    use farcooler_store::plan::LaneState;
    let word = |s: i32| LaneState::parse(lane_state_word(s));
    let (Some(from), Some(target)) = (word(lane.state), word(to as i32)) else { return Ok(()) };
    if from.can_move_to(target) {
        return Ok(());
    }
    let go: Vec<&str> = from.moves().into_iter().map(|s| s.as_str()).collect();
    Err(match go.as_slice() {
        [] => format!("{} is {}, so it takes no more changes.", lane.name, from.as_str()),
        [only] => format!("{} is {}. It can only go to {only}.", lane.name, state_word(lane.state).to_lowercase()),
        [init @ .., last] => format!(
            "{} is {}. It can go to {} or {last}.",
            lane.name,
            state_word(lane.state).to_lowercase(),
            init.join(", ")
        ),
    }
    .into())
}

fn lane_state(arg: LaneStateArg) -> pb::LaneState {
    match arg {
        LaneStateArg::Queued => pb::LaneState::Queued,
        LaneStateArg::Building => pb::LaneState::Building,
        LaneStateArg::Review => pb::LaneState::Review,
        LaneStateArg::Fixing => pb::LaneState::Fixing,
        LaneStateArg::Landing => pb::LaneState::Landing,
        LaneStateArg::Landed => pb::LaneState::Landed,
        LaneStateArg::Dropped => pb::LaneState::Dropped,
    }
}

fn is_live(state: i32) -> bool {
    state != pb::LaneState::Landed as i32 && state != pb::LaneState::Dropped as i32
}

// ---------------------------------------------------------------------------
// words
// ---------------------------------------------------------------------------

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// A lane's state, as a person reads it.
fn state_word(state: i32) -> &'static str {
    match pb::LaneState::try_from(state) {
        Ok(pb::LaneState::Queued) => "Queued",
        Ok(pb::LaneState::Building) => "Building",
        Ok(pb::LaneState::Review) => "In review",
        Ok(pb::LaneState::Fixing) => "Fixing",
        Ok(pb::LaneState::Landing) => "Landing",
        Ok(pb::LaneState::Landed) => "Landed",
        Ok(pb::LaneState::Dropped) => "Dropped",
        _ => "Unknown",
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

fn spend_words(spend: &pb::LaneSpend) -> String {
    let total = spend.input_tokens + spend.output_tokens + spend.cache_read_tokens + spend.cache_write_tokens;
    if total == 0 {
        return NOT_REPORTED.to_string();
    }
    let mut said = format!("{} tokens", tokens(total));
    if let Some(micros) = spend.cost_micros {
        said.push_str(&format!(" · {} API-equivalent", dollars(micros)));
    }
    if spend.unmeasured_agents > 0 {
        said.push_str(&format!(" · {} not reported", count(spend.unmeasured_agents as usize, "agent")));
    }
    said.push_str(&shared_words(spend.shared_agents));
    said
}

/// " · 1 agent's spend split with other lanes": an agent on several lanes
/// counts once across them, evenly, and the lane's figure says it holds a
/// part. Empty when no agent is shared.
fn shared_words(shared: u32) -> String {
    match shared {
        0 => String::new(),
        1 => " · 1 agent\u{2019}s spend split with other lanes".to_string(),
        n => format!(" · {n} agents\u{2019} spend split with other lanes"),
    }
}

/// A card's part of a lane's spend. A lane's agents are recorded on the lane,
/// not on a card, so a card's number is the lane's split evenly and says so;
/// `None` for a lane with fewer than two cards, where the lane's spend is the
/// card's.
fn share_words(spend: &pb::LaneSpend, cards: usize) -> Option<String> {
    let total = spend.input_tokens + spend.output_tokens + spend.cache_read_tokens + spend.cache_write_tokens;
    if cards < 2 || total == 0 {
        return None;
    }
    let mut said = format!("About {} tokens", tokens(total / cards as u64));
    if let Some(micros) = spend.cost_micros {
        said.push_str(&format!(" · {} API-equivalent", dollars(micros / cards as i64)));
    }
    Some(format!("{said} a card, the lane\u{2019}s spend split evenly across {cards} cards"))
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

fn lane_row(l: &pb::Lane, keys: &Keys, now: i64) -> String {
    let mut row = format!("{:<16} {}", l.name, lane_status(l, now));
    if !l.cards.is_empty() {
        let listed: Vec<String> = l.cards.iter().map(|c| keys.of(&c.task_id)).collect();
        row.push_str(&format!(" · {}", listed.join(" ")));
    }
    row
}

/// "In review · in integ-9 · 5 cards · 470k tokens".
fn lane_status(l: &pb::Lane, now: i64) -> String {
    let mut parts = vec![state_word(l.state).to_string()];
    if l.state == pb::LaneState::Fixing as i32 && l.fix_rounds > 0 {
        parts[0] = format!("Fixing · round {}", l.fix_rounds);
    }
    if let Some(rank) = l.plan_rank {
        parts.push(if rank == 1 { "next up".to_string() } else { format!("{} in the plan", ordinal(rank)) });
    }
    if let Some(train) = &l.train {
        parts.push(format!("in {train}"));
    }
    parts.push(count(l.cards.len(), "card"));
    let spend = l.spend.unwrap_or_default();
    if spend.runs > 0 {
        parts.push(spend_words(&spend));
    }
    parts.extend(cost::budget_words(&spend, l.budget_tokens));
    if l.stale {
        parts.push(format!("stuck for {}", age(now - l.state_since)));
    }
    parts.join(" · ")
}

fn ordinal(n: u32) -> String {
    match n {
        1 => "1st".into(),
        2 => "2nd".into(),
        3 => "3rd".into(),
        n => format!("{n}th"),
    }
}

/// What the Mac's overview shows, in text, so an orchestrator reads the same
/// picture the owner sees.
fn overview(plan: &pb::Plan, now: i64) -> String {
    if plan.themes.is_empty() && plan.lanes.is_empty() && plan.rulings.is_empty() && plan.trains.is_empty() {
        return NOTHING_PLANNED.to_string();
    }
    let keys = Keys::of_plan(plan);
    let mut out: Vec<String> = Vec::new();
    let lane_of = |id: &bytes::Bytes| plan.lanes.iter().find(|l| l.id == *id);

    let next: Vec<&pb::Lane> = plan.order.iter().filter_map(lane_of).collect();
    if !next.is_empty() {
        out.push("Next up".into());
        for (i, l) in next.iter().enumerate() {
            let cards: Vec<String> = l.cards.iter().map(|c| keys.of(&c.task_id)).collect();
            out.push(format!("  {}  {:<16} {}", i + 1, l.name, cards.join(" ")));
            if !l.reason.is_empty() {
                out.push(format!("     {}", l.reason));
            }
        }
    }
    let now_lanes: Vec<&pb::Lane> = plan
        .lanes
        .iter()
        .filter(|l| is_live(l.state) && l.state != pb::LaneState::Queued as i32)
        .collect();
    // Trains not yet landed head row groups of their lanes (ov-309); the
    // lanes on none follow.
    let trains: Vec<&pb::BoardTrain> = plan
        .trains
        .iter()
        .filter(|t| t.state != pb::BoardTrainState::Landed as i32 && t.state != pb::BoardTrainState::Dropped as i32)
        .collect();
    if !now_lanes.is_empty() || !trains.is_empty() {
        out.push("Now".into());
        for t in &trains {
            out.push(format!("  {}", train::train_line(plan, t)));
            let on: Vec<&pb::Lane> = plan.lanes.iter().filter(|l| t.lane_ids.contains(&l.id)).collect();
            out.extend(on.iter().map(|l| format!("    {:<16} {}", l.name, lane_status(l, now))));
        }
        let grouped = |l: &pb::Lane| trains.iter().any(|t| t.lane_ids.contains(&l.id));
        out.extend(now_lanes.iter().filter(|l| !grouped(l)).map(|l| format!("  {:<16} {}", l.name, lane_status(l, now))));
    }
    let unplanned: Vec<&pb::Lane> = plan
        .lanes
        .iter()
        .filter(|l| l.state == pb::LaneState::Queued as i32 && l.plan_rank.is_none())
        .collect();
    if !unplanned.is_empty() {
        out.push("Queued, not in the plan".into());
        out.extend(unplanned.iter().map(|l| format!("  {:<16} {}", l.name, l.reason)));
    }
    if !plan.themes.is_empty() {
        out.push("Themes".into());
        for view in &plan.themes {
            out.push(format!("  {}", theme_row(view)));
        }
    }
    out.extend(ruling::overview_lines(plan));
    out.extend(cost::overview_lines(plan));
    let day = 24 * 60 * 60 * 1000;
    let landed: Vec<&str> = plan
        .lanes
        .iter()
        .filter(|l| l.state == pb::LaneState::Landed as i32 && now - l.state_since < day)
        .map(|l| l.name.as_str())
        .collect();
    if !landed.is_empty() {
        out.push("Landed today".into());
        out.push(format!("  {}", landed.join(", ")));
    }
    let checks = checks(plan, &keys);
    if !checks.is_empty() {
        out.push("Worth a look".into());
        out.extend(checks.into_iter().map(|c| format!("  {c}")));
    }
    out.join("\n")
}

/// What the reconciliation reports flag by hand, derived: a card whose lanes
/// have all landed while it isn't done, and one in progress with no lane. Only
/// for cards a theme or lane names; the layer knows no others.
fn checks(plan: &pb::Plan, keys: &Keys) -> Vec<String> {
    let mut out = Vec::new();
    for c in &plan.cards {
        if !is_open(c.status) {
            continue;
        }
        let cover = plan.coverage.iter().find(|v| v.task_id == c.task_id);
        let (live, landed) = cover.map_or((0, 0), |v| (v.live, v.landed));
        if live == 0 && landed > 0 {
            out.push(format!("{}  All its lanes have landed, and it's still {}.", keys.of(&c.task_id), status_word(c.status)));
        } else if live == 0
            && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        {
            out.push(format!("{}  {}, and no lane is working it.", keys.of(&c.task_id), status_word(c.status)));
        }
    }
    out
}

fn theme_text(
    view: &pb::BoardThemeView,
    plan: &pb::Plan,
    keys: &Keys,
    events: &[pb::PlanEvent],
    now: i64,
) -> String {
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
    let lanes: Vec<&pb::Lane> = plan
        .lanes
        .iter()
        .filter(|l| l.cards.iter().any(|c| view.task_ids.contains(&c.task_id)))
        .collect();
    if !lanes.is_empty() {
        out.push(String::new());
        out.push("Lanes".into());
        out.extend(lanes.iter().map(|l| format!("  {:<16} {}", l.name, lane_status(l, now))));
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
        let card = plan.cards.iter().find(|c| c.task_id == *id);
        match card {
            Some(c) => out.push(format!("  {}  {}  {}", c.key, c.title, status_word(c.status))),
            None => out.push(format!("  {}", keys.of(id))),
        }
    }
    out.join("\n")
}

fn lane_text(l: &pb::Lane, keys: &Keys, events: &[pb::PlanEvent], now: i64) -> String {
    let mut out = vec![format!("{} · {}", l.name, lane_status(l, now))];
    if !l.reason.is_empty() {
        out.push(l.reason.clone());
    }
    let place: Vec<&str> = [l.worktree_path.as_str(), l.branch.as_str(), l.model.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if !place.is_empty() {
        out.push(place.join(" · "));
    }
    if let Some(sha) = &l.landed_sha {
        out.push(format!("Landed as {sha}"));
    }
    out.push(String::new());
    out.push("Cards".into());
    for c in &l.cards {
        let slice = if c.slice.is_empty() { "whole card".to_string() } else { c.slice.clone() };
        out.push(format!("  {}  {}", keys.of(&c.task_id), slice));
    }
    let spend = l.spend.unwrap_or_default();
    out.push(format!("Spend  {}", spend_words(&spend)));
    if let Some(share) = share_words(&spend, l.cards.len()) {
        out.push(format!("Share  {share}"));
    }
    if !l.agents.is_empty() {
        let agents: Vec<String> = l
            .agents
            .iter()
            .map(|a| {
                let role = match pb::LaneAgentRole::try_from(a.role) {
                    Ok(pb::LaneAgentRole::Review) => "Reviewer",
                    Ok(pb::LaneAgentRole::Fix) => "Fixer",
                    _ => "Builder",
                };
                let model = if a.model.is_empty() { String::new() } else { format!(" {}", a.model) };
                let span = a.ended_at.unwrap_or(now) - a.started_at;
                format!("{role}{model}, {}", age(span))
            })
            .collect();
        out.push(format!("Agents  {}", agents.join(" · ")));
    }
    if !events.is_empty() {
        out.push(String::new());
        out.push("Timeline".into());
        out.extend(events.iter().map(|e| format!("  {}  {}", ago(now - e.at), e.body)));
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------
// json
// ---------------------------------------------------------------------------

fn id_text(id: &[u8]) -> String {
    uuid_of(id).to_string()
}

/// A spend as the JSON carries it: a lane's, or a theme's share (ov-306).
fn spend_json(s: &pb::LaneSpend) -> Value {
    json!({
        "input_tokens": s.input_tokens, "output_tokens": s.output_tokens,
        "cache_read_tokens": s.cache_read_tokens, "cache_write_tokens": s.cache_write_tokens,
        "cost_micros": s.cost_micros, "runs": s.runs, "unmeasured_agents": s.unmeasured_agents,
        "shared_agents": s.shared_agents,
    })
}

/// The week's tokens and the harness and model comparison as the JSON carries
/// them (ov-307). `null` from a runner without `board_cost`.
fn cost_json(c: &pb::PlanCost) -> Value {
    json!({
        "week_tokens": c.week_tokens,
        "compare": c.compare.iter().map(|p| json!({
            "harness": p.harness, "model": p.model, "cards": p.cards, "tokens": p.tokens,
            "cost_micros": p.cost_micros,
        })).collect::<Vec<_>>(),
        "compare_held_back": c.compare_held_back,
    })
}

/// Status counts as the JSON carries them: a theme's, or the board's (ov-306).
fn counts_json(c: &pb::PlanStatusCounts) -> Value {
    json!({
        "backlog": c.backlog, "todo": c.todo, "needs_decision": c.needs_decision,
        "in_progress": c.in_progress, "in_review": c.in_review, "done": c.done, "cancelled": c.cancelled,
    })
}

fn theme_json(view: &pb::BoardThemeView, keys: &Keys) -> Value {
    let t = view.theme.clone().unwrap_or_default();
    let c = view.counts.unwrap_or_default();
    json!({
        "id": id_text(&t.id),
        "short": short_bytes(&t.id),
        "name": t.name,
        "outcome": t.outcome,
        "story": t.story,
        "story_at": t.story_at,
        "next": t.next,
        "owner_ask": t.owner_ask,
        "state": state_of_theme(t.state),
        "ordinal": t.ordinal,
        "cards": view.task_ids.iter().map(|id| json!({ "task": id_text(id), "key": keys.of(id) })).collect::<Vec<_>>(),
        "counts": counts_json(&c),
        "spend": spend_json(&view.spend.unwrap_or_default()),
        "budget_tokens": view.budget_tokens,
        "trend_tokens": view.trend_tokens,
    })
}

fn lane_state_word(state: i32) -> &'static str {
    match pb::LaneState::try_from(state) {
        Ok(pb::LaneState::Queued) => "queued",
        Ok(pb::LaneState::Building) => "building",
        Ok(pb::LaneState::Review) => "review",
        Ok(pb::LaneState::Fixing) => "fixing",
        Ok(pb::LaneState::Landing) => "landing",
        Ok(pb::LaneState::Landed) => "landed",
        Ok(pb::LaneState::Dropped) => "dropped",
        _ => "unknown",
    }
}

fn lane_json(l: &pb::Lane, keys: &Keys) -> Value {
    let spend = l.spend.unwrap_or_default();
    json!({
        "id": id_text(&l.id),
        "short": short_bytes(&l.id),
        "name": l.name,
        "state": lane_state_word(l.state),
        "reason": l.reason,
        "plan_rank": l.plan_rank,
        "worktree_path": l.worktree_path,
        "branch": l.branch,
        "harness": l.harness,
        "model": l.model,
        "train": l.train,
        "landed_sha": l.landed_sha,
        "state_since": l.state_since,
        "stale": l.stale,
        "budget_tokens": l.budget_tokens,
        "fix_rounds": l.fix_rounds,
        "cards": l.cards.iter().map(|c| json!({
            "task": id_text(&c.task_id), "key": keys.of(&c.task_id), "slice": c.slice,
        })).collect::<Vec<_>>(),
        "agents": l.agents.iter().map(|a| json!({
            "harness": a.harness, "agent_id": a.agent_id,
            "role": match pb::LaneAgentRole::try_from(a.role) {
                Ok(pb::LaneAgentRole::Review) => "review",
                Ok(pb::LaneAgentRole::Fix) => "fix",
                _ => "build",
            },
            "model": a.model, "started_at": a.started_at, "ended_at": a.ended_at,
        })).collect::<Vec<_>>(),
        "spend": {
            "input_tokens": spend.input_tokens, "output_tokens": spend.output_tokens,
            "cache_read_tokens": spend.cache_read_tokens, "cache_write_tokens": spend.cache_write_tokens,
            "cost_micros": spend.cost_micros, "runs": spend.runs, "unmeasured_agents": spend.unmeasured_agents,
            "shared_agents": spend.shared_agents,
        },
    })
}

fn event_json(e: &pb::PlanEvent) -> Value {
    json!({
        "at": e.at, "actor": e.actor, "kind": e.kind, "body": e.body,
        "extra": serde_json::from_str::<Value>(&e.extra_json).unwrap_or(Value::Null),
    })
}

/// The whole plan, with the two flags the reconciliation reports derive by
/// hand: `landed_not_closed` and `no_lane`.
fn plan_json(plan: &pb::Plan, keys: &Keys) -> Value {
    let flagged = |wanted: fn(&pb::PlanCard, u32, u32) -> bool| -> Vec<Value> {
        plan.cards
            .iter()
            .filter(|c| {
                let (live, landed) = plan
                    .coverage
                    .iter()
                    .find(|v| v.task_id == c.task_id)
                    .map_or((0, 0), |v| (v.live, v.landed));
                is_open(c.status) && wanted(c, live, landed)
            })
            .map(|c| json!({ "task": id_text(&c.task_id), "key": c.key, "status": status_word(c.status) }))
            .collect()
    };
    json!({
        "now_ms": plan.now_ms,
        "themes": plan.themes.iter().map(|v| theme_json(v, keys)).collect::<Vec<_>>(),
        "lanes": plan.lanes.iter().map(|l| lane_json(l, keys)).collect::<Vec<_>>(),
        "order": plan.order.iter().map(|id| id_text(id)).collect::<Vec<_>>(),
        "cards": plan.cards.iter().map(|c| json!({
            "task": id_text(&c.task_id), "key": c.key, "title": c.title, "status": status_word(c.status),
        })).collect::<Vec<_>>(),
        "landed_not_closed": flagged(|_, live, landed| live == 0 && landed > 0),
        "no_lane": flagged(|c, live, landed| {
            live == 0 && landed == 0
                && (c.status == pb::TaskStatus::InProgress as i32 || c.status == pb::TaskStatus::InReview as i32)
        }),
        "rulings": plan.rulings.iter().map(|r| ruling::ruling_json(plan, r, keys)).collect::<Vec<_>>(),
        "trains": plan.trains.iter().map(|t| train::train_json(plan, t, keys)).collect::<Vec<_>>(),
        "ci": plan.ci.iter().map(train::ci_json).collect::<Vec<_>>(),
        "board_counts": counts_json(&plan.board_counts.unwrap_or_default()),
        "cost": plan.cost.as_ref().map(cost_json),
    })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

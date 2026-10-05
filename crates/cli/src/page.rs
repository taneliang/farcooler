//! `farcooler page`: orchestrator pages (ov-269, ov-283).
//!
//! **Experimental, and beside the board.** A page is a small JSON document of
//! typed blocks that the apps draw natively, published to a slot on a board.
//! The orchestrator is the one writer. Nothing here edits a card or the plan,
//! and neither carries a page.
//!
//! ```text
//! farcooler page list                       slots, titles, anchors, updated, revision
//! farcooler page show SLOT                  the page as text, the way the apps draw it
//! farcooler page set SLOT --file page.json  publish; --file - reads stdin
//! farcooler page check --file page.json     validate offline, no runner
//! farcooler page rm SLOT
//! farcooler page schema                     the blocks, with one example each
//! farcooler page stats                      how often each slot is published
//! ```
//!
//! Every command that talks to a runner takes `--repo`, `--workspace` and
//! `--actor` with the meanings `plan` gives them (`--runner` and `--json` are
//! the top-level flags). `check` and `schema` need no runner, so a document can
//! be fixed before it is published.
//!
//! Everyone who can read the board can read a page, so put nothing on one that
//! you wouldn't put in a task note.

use std::collections::HashMap;

use clap::{Args, Subcommand};
use farcooler_client::page_json::{page_json, pages_json};
use farcooler_core::page_doc::{self, Caps, Page, Target};
use farcooler_core::page_schema;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use serde_json::{Value, json};

use crate::page_text::{Live, render};
use crate::tasks::{Board, DispatchLink, Refused, actor_for, board_for, board_in, refused};
use crate::workspaces::{WORKSPACE_ENV, workspaces_on};
use crate::{Fallible, connect_to, expect_value, req_for, short_bytes, with};

/// What a runner without pages is told.
const NEEDS_UPDATE: &str = "This runner needs an update to show pages.";

/// What `list` says on a board with no page.
const NOTHING_PUBLISHED: &str = "No pages yet. Publish one with `farcooler page set SLOT --file page.json`.";

/// `farcooler page ...`.
#[derive(Debug, Clone, Args)]
pub struct PageArgs {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    cmd: PageCmd,
}

/// The flags every `page` command that talks to a runner takes.
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
enum PageCmd {
    /// List this board's pages: slot, title, theme, when it changed and its revision.
    List,
    /// One page as text, the way the apps draw it, with its references resolved.
    Show {
        /// The page's slot.
        slot: String,
    },
    /// Publish a page, replacing the slot whole. Identical bytes change nothing.
    ///
    /// Everyone who can read the board can read a page. Don't put anything on
    /// one that you wouldn't put in a task note. A slot can change 30 times an
    /// hour: pages are for checkpoints, not a live log.
    Set {
        /// A name for the page: lowercase letters, digits and hyphens, at most 40.
        slot: String,
        /// The page's JSON, from a file, or `-` for standard input.
        #[arg(long, value_name = "FILE")]
        file: String,
        /// Draw the page inside this theme's page, by name or short id.
        #[arg(long, value_name = "NAME", conflicts_with = "no_theme")]
        theme: Option<String>,
        /// Take the page off its theme.
        #[arg(long)]
        no_theme: bool,
        /// Refuse unless the page is at this revision (0 for a new slot).
        #[arg(long, value_name = "N")]
        if_revision: Option<u64>,
    },
    /// Check a page's JSON without a runner. Exits 1 with the path of what's wrong.
    ///
    /// It can't check that references resolve; `set` does.
    Check {
        /// The page's JSON, from a file, or `-` for standard input.
        #[arg(long, value_name = "FILE")]
        file: String,
    },
    /// Remove a page.
    Rm {
        /// The page's slot.
        slot: String,
    },
    /// The nine blocks with an example of each, or with --json, as JSON Schema.
    Schema,
    /// How often each slot has been published, and with which blocks.
    Stats {
        /// How far back: a number and d, h or m, like 14d.
        #[arg(long, default_value = "14d")]
        since: String,
    },
}

/// `farcooler page ...`.
pub async fn page(runner: Option<&str>, args: PageArgs, json: bool) -> Fallible {
    match &args.cmd {
        PageCmd::Check { file } => {
            println!("{}", check(&read_file(file)?)?);
            return Ok(());
        }
        PageCmd::Schema => {
            println!("{}", schema(json));
            return Ok(());
        }
        _ => {}
    }
    let mut link = connect_to(runner).await?;
    needs_pages(&link.capabilities())?;
    let actor = actor_for(args.common.actor.as_deref())?.to_string();
    let board = board_for(
        &mut link,
        args.common.repo.as_deref(),
        args.common.workspace.as_deref(),
        std::env::var(WORKSPACE_ENV).ok(),
    )
    .await?;
    println!("{}", run_on(&mut link, &board, args.cmd, &actor, json, now_ms(), &read_file).await?);
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

type Failed = Box<dyn std::error::Error>;

/// A file's text, or standard input for `-`.
fn read_file(file: &str) -> Result<String, Failed> {
    let read = if file == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(file)
    };
    read.map_err(|_| Refused::new(format!("Couldn't read {}.", if file == "-" { "standard input" } else { file }), None).into())
}

/// Refused before anything is sent: a refusal here costs no round trip and
/// says what to do.
fn needs_pages(capabilities: &[String]) -> Result<(), Failed> {
    if capabilities.iter().any(|c| c == capability::BOARD_PAGES) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

/// Check a document offline, saying how big it is, or where it's wrong.
fn check(text: &str) -> Result<String, Failed> {
    let page = page_doc::parse(text, &Caps::default()).map_err(|e| Refused::new(e.to_string(), None))?;
    let blocks = page.blocks.len();
    let refs = page.references().len();
    Ok(format!(
        "This page is valid: {blocks} block{}, {refs} reference{}, {} bytes. `page check` can't tell whether references resolve; `page set` does.",
        if blocks == 1 { "" } else { "s" },
        if refs == 1 { "" } else { "s" },
        page.to_json().len(),
    ))
}

fn schema(json: bool) -> String {
    if json {
        serde_json::to_string_pretty(&page_schema::json_schema()).unwrap_or_default()
    } else {
        page_schema::reference_text().trim_end().to_string()
    }
}

/// One command on `board`, answering what to print. `read` reads a `--file`.
async fn run_on<L: DispatchLink, R: Fn(&str) -> Result<String, Failed>>(
    link: &mut L,
    board: &Board,
    cmd: PageCmd,
    actor: &str,
    json: bool,
    now: i64,
    read: &R,
) -> Result<String, Failed> {
    match cmd {
        PageCmd::Check { file } => return check(&read(&file)?),
        PageCmd::Schema => return Ok(schema(json)),
        _ => {}
    }
    needs_pages(&link.capabilities())?;
    let board = &on_a_board(link, board).await?;
    let Some(workspace) = &board.workspace else { return Err("Name a board with --workspace.".into()) };
    let ws = workspace.id.clone();
    match cmd {
        PageCmd::Check { .. } | PageCmd::Schema => unreachable!("answered above"),
        PageCmd::List => {
            let list = list_pages(link, board, &ws, json).await?;
            if json {
                return Ok(pages_json(&list).to_string());
            }
            if list.pages.is_empty() {
                return Ok(NOTHING_PUBLISHED.to_string());
            }
            let plan = if list.pages.iter().any(|p| !p.anchor.is_empty()) { plan_of(link, &ws).await } else { None };
            Ok(table(&list.pages, plan.as_ref(), now))
        }
        PageCmd::Show { slot } => {
            let stored = get_page(link, board, &ws, &slot).await?;
            if json {
                return Ok(page_json(&stored).to_string());
            }
            let page = page_doc::parse(&stored.doc_json, &Caps::default())
                .map_err(|_| Refused::new(format!("The runner's copy of {slot} isn't a page this Far Cooler can read. Update Far Cooler."), None))?;
            let live = live_of(link, board, &ws, &page).await;
            let theme = match (stored.anchor.is_empty(), &live.plan) {
                (false, Some(plan)) => theme_named(plan, &stored.anchor),
                _ => None,
            };
            Ok(render(&stored, &page, theme.as_deref(), &live, now))
        }
        PageCmd::Set { slot, file, theme, no_theme, if_revision } => {
            let text = read(&file)?;
            if !page_doc::valid_slot(&slot) {
                return Err(Refused::new(
                    "A page's slot is lowercase letters, digits and hyphens, at most 40 characters.".into(),
                    None,
                )
                .into());
            }
            page_doc::parse(&text, &Caps::default()).map_err(|e| Refused::new(e.to_string(), None))?;
            let anchor_theme_id = match (theme, no_theme) {
                (Some(name), _) => Some(theme_id(link, &ws, &name).await?),
                (None, true) => Some(bytes::Bytes::new()),
                (None, false) => None,
            };
            let set = request::Payload::PageSet(pb::PageSet {
                workspace_id: ws,
                slot: slot.clone(),
                doc_json: text,
                anchor_theme_id,
                ordinal: None,
                if_revision,
                actor: actor.into(),
            });
            let result::Value::PageSetResult(done) = send(link, board, "page.set", set, &slot).await? else {
                return Err(unreadable());
            };
            let stored = done.page.unwrap_or_default();
            if json {
                return Ok(json!({ "slot": stored.slot, "changed": done.changed, "page": page_json(&stored) }).to_string());
            }
            Ok(if done.changed {
                format!("Published {}, revision {}.", stored.slot, stored.revision)
            } else {
                format!("{} is already up to date at revision {}. Nothing was published.", stored.slot, stored.revision)
            })
        }
        PageCmd::Rm { slot } => {
            let remove = request::Payload::PageRemove(pb::PageRemove {
                workspace_id: ws,
                slot: slot.clone(),
                actor: actor.into(),
            });
            let result::Value::BoardPage(gone) = send(link, board, "page.remove", remove, &slot).await? else {
                return Err(unreadable());
            };
            Ok(if json { json!({ "removed": gone.slot }).to_string() } else { format!("Removed {}.", gone.slot) })
        }
        PageCmd::Stats { since } => {
            let span = duration_ms(&since)?;
            let stats = request::Payload::PageStats(pb::PageStatsRequest { workspace_id: ws, since_ms: now - span });
            let result::Value::PageStatsList(list) = send(link, board, "page.stats", stats, "").await? else {
                return Err(unreadable());
            };
            if json {
                return Ok(stats_json(&list).to_string());
            }
            if list.slots.is_empty() {
                return Ok("Nothing was published in that time.".to_string());
            }
            Ok(list.slots.iter().map(|s| stats_row(s, now)).collect::<Vec<_>>().join("\n"))
        }
    }
}

/// `board`, or when it names no workspace the repository's Main: pages are one
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

// ---------------------------------------------------------------------------
// the wire
// ---------------------------------------------------------------------------

async fn send<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    method: &str,
    payload: request::Payload,
    slot: &str,
) -> Result<result::Value, Failed> {
    let mut r = with(req_for(method, board.repository), payload);
    r.required_capabilities.push(capability::BOARD_PAGES.to_string());
    let answer = link.call(r).await.map_err(|e| refused_here(e, slot))?;
    expect_value(answer.value)
}

async fn list_pages<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: &bytes::Bytes,
    with_docs: bool,
) -> Result<pb::BoardPageList, Failed> {
    let p = request::Payload::PageList(pb::PageListRequest { workspace_id: ws.clone(), with_docs });
    match send(link, board, "page.list", p, "").await? {
        result::Value::BoardPageList(list) => Ok(list),
        _ => Err(unreadable()),
    }
}

async fn get_page<L: DispatchLink>(
    link: &mut L,
    board: &Board,
    ws: &bytes::Bytes,
    slot: &str,
) -> Result<pb::BoardPage, Failed> {
    let p = request::Payload::PageGet(pb::PageGetRequest { workspace_id: ws.clone(), slot: slot.into() });
    match send(link, board, "page.get", p, slot).await? {
        result::Value::BoardPage(page) => Ok(page),
        _ => Err(unreadable()),
    }
}

/// The plan, when this runner has one: a page's lanes and themes are drawn
/// from it, and without it they're plain text.
async fn plan_of<L: DispatchLink>(link: &mut L, ws: &bytes::Bytes) -> Option<pb::Plan> {
    if !link.capabilities().iter().any(|c| c == capability::BOARD_PLAN) {
        return None;
    }
    let mut r = with(
        req_for("plan.get", crate::uuid_of(ws)),
        request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: ws.clone(), include_closed: true }),
    );
    r.required_capabilities.push(capability::BOARD_PLAN.to_string());
    match link.call(r).await.ok()?.value {
        Some(result::Value::Plan(plan)) => Some(plan),
        _ => None,
    }
}

/// What a page's references are drawn from, read only as far as it names them.
async fn live_of<L: DispatchLink>(link: &mut L, board: &Board, ws: &bytes::Bytes, page: &Page) -> Live {
    let refs = page.references();
    let names = |f: fn(&Target) -> bool| refs.iter().any(|r| f(&r.reference.target));
    let mut live = Live::default();
    if names(|t| matches!(t, Target::Task(_) | Target::Ask(_))) {
        if let Ok(list) = board_in(link, board, None, None).await {
            live.cards = list
                .items
                .iter()
                .map(|t| (t.key.to_ascii_lowercase(), (t.title.clone(), t.status)))
                .collect();
        }
    }
    if names(|t| matches!(t, Target::Lane(_) | Target::Theme(_) | Target::Ci(_) | Target::Cards(_))) {
        live.plan = plan_of(link, ws).await;
    }
    if names(|t| matches!(t, Target::Page(_))) {
        if let Ok(list) = list_pages(link, board, ws, false).await {
            live.pages = list.pages.iter().map(|p| (p.slot.clone(), p.title.clone())).collect();
        }
    }
    live
}

// ---------------------------------------------------------------------------
// themes
// ---------------------------------------------------------------------------

/// The id of the theme `name` names: its name, its short id, or a name it is
/// the start of when nothing else is.
async fn theme_id<L: DispatchLink>(link: &mut L, ws: &bytes::Bytes, name: &str) -> Result<bytes::Bytes, Failed> {
    let Some(plan) = plan_of(link, ws).await else {
        return Err(Refused::new("This runner has no plan, so a page can't be drawn in a theme. Publish it without --theme.".into(), None).into());
    };
    let wanted = name.trim().to_lowercase();
    let themes: Vec<&pb::BoardTheme> = plan
        .themes
        .iter()
        .filter_map(|v| v.theme.as_ref())
        .filter(|t| t.state != pb::BoardThemeState::Dropped as i32)
        .collect();
    if let Some(t) = themes.iter().find(|t| t.name.to_lowercase() == wanted || short_bytes(&t.id) == wanted) {
        return Ok(t.id.clone());
    }
    let starting: Vec<&&pb::BoardTheme> =
        themes.iter().filter(|t| !wanted.is_empty() && t.name.to_lowercase().starts_with(&wanted)).collect();
    match starting.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!("No theme here is called {name:?}.").into()),
        many => Err(format!("{name:?} starts {} themes. Say more of the name.", many.len()).into()),
    }
}

/// The name of the theme whose id (as text) a page is anchored to.
fn theme_named(plan: &pb::Plan, anchor: &str) -> Option<String> {
    plan.themes
        .iter()
        .filter_map(|v| v.theme.as_ref())
        .find(|t| crate::uuid_of(&t.id).to_string() == anchor)
        .map(|t| t.name.clone())
}

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

/// A refusal from the runner: its own sentence for a page it refused (that
/// sentence is the JSON path and the limit), this module's for the two it
/// words differently, else the one `refused` makes.
fn refused_here(err: ClientError, slot: &str) -> Failed {
    if let ClientError::Daemon { code, what, message, .. } = &err {
        if what == "page" {
            return Box::new(Refused::naming(message.clone(), *code, what.clone()));
        }
        if *code == pb::ErrorCode::ResourceConflict as i32 {
            return Box::new(Refused::naming(
                format!("This page changed since you read it. Read it again with `farcooler page show {slot}`, then publish."),
                *code,
                what.clone(),
            ));
        }
        if *code == pb::ErrorCode::NotFound as i32 && !slot.is_empty() {
            return Box::new(Refused::naming(format!("No page here is in the slot {slot}."), *code, what.clone()));
        }
    }
    refused(err, "The runner couldn't record that. Try again.")
}

// ---------------------------------------------------------------------------
// words
// ---------------------------------------------------------------------------

/// "5 min ago", "3 h ago": how long since `ms` before `now`, coarsely.
fn ago(now: i64, ms: i64) -> String {
    let minutes = (now - ms).max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1440),
    }
}

/// The list, as aligned columns under a header.
fn table(pages: &[pb::BoardPage], plan: Option<&pb::Plan>, now: i64) -> String {
    let rows: Vec<[String; 5]> = pages
        .iter()
        .map(|p| {
            let theme = if p.anchor.is_empty() {
                String::new()
            } else {
                plan.and_then(|plan| theme_named(plan, &p.anchor)).unwrap_or_else(|| "(not found)".to_string())
            };
            [p.slot.clone(), p.title.clone(), theme, ago(now, p.updated_at_ms), p.revision.to_string()]
        })
        .collect();
    let header = ["Slot", "Title", "Theme", "Updated", "Rev"].map(String::from);
    let widths: Vec<usize> =
        (0..5).map(|c| rows.iter().chain([&header]).map(|r| r[c].chars().count()).max().unwrap_or(0)).collect();
    std::iter::once(&header)
        .chain(rows.iter())
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(c, t)| format!("{t}{}", " ".repeat(widths[c] - t.chars().count())))
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// "14d", "12h", "90m" as milliseconds.
fn duration_ms(text: &str) -> Result<i64, Failed> {
    let say = || Refused::new("Use a number and d, h or m for --since, like 14d.".into(), None);
    let text = text.trim();
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let n: i64 = digits.parse().map_err(|_| say())?;
    let unit_ms = match unit {
        "d" => 86_400_000,
        "h" => 3_600_000,
        "m" => 60_000,
        _ => return Err(say().into()),
    };
    n.checked_mul(unit_ms).filter(|ms| *ms > 0).ok_or_else(|| say().into())
}

fn stats_row(s: &pb::PageSlotStats, now: i64) -> String {
    let mut shape: Vec<&pb::PageShapeCount> = s.shape.iter().collect();
    shape.sort_by(|a, b| b.count.cmp(&a.count).then(a.kind.cmp(&b.kind)));
    let mix: Vec<String> = shape.iter().map(|c| format!("{} {}", c.kind, c.count)).collect();
    let mut row = format!("{}  {} write{}", s.slot, s.sets, if s.sets == 1 { "" } else { "s" });
    if s.removes > 0 {
        row.push_str(&format!(", {} removal{}", s.removes, if s.removes == 1 { "" } else { "s" }));
    }
    row.push_str(&format!(" · last {}", ago(now, s.last_at_ms)));
    if !mix.is_empty() {
        row.push_str(&format!(" · {}", mix.join(", ")));
    }
    row
}

fn stats_json(list: &pb::PageStatsList) -> Value {
    let by_slot: Vec<Value> = list
        .slots
        .iter()
        .map(|s| {
            let shape: HashMap<&str, u32> = s.shape.iter().map(|c| (c.kind.as_str(), c.count)).collect();
            json!({
                "slot": s.slot, "sets": s.sets, "removes": s.removes,
                "first_at_ms": s.first_at_ms, "last_at_ms": s.last_at_ms, "shape": shape,
            })
        })
        .collect();
    json!({ "slots": by_slot })
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;

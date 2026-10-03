//! `farcooler task wait`, `task line` and `task worker`: when a task starts
//! (ov-212) and who works it (ov-213), plus the words `task list` and `task
//! show` print for both.
//!
//! A child of `tasks`, for the board's own helpers, and a file of its own so
//! that one stays inside its size budget.

use std::collections::HashMap;

use clap::Subcommand;
use farcooler_client::{task_starts_json, tasks_json};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use uuid::Uuid;

use super::{
    Board, DispatchLink, actor_for, board_for, find_task, refused, spoken_gap, stale_for_seconds, status_word,
    tasks_in, workspaces_on,
};
use crate::workspaces::WORKSPACE_ENV;
use crate::{Fallible, expect_value, id_bytes, req_for, truncate, uuid_of, with};

#[derive(Subcommand)]
pub enum StartCmd {
    /// Say when a task that hasn't started will start.
    ///
    /// Hold it until a time or an event, park it, or clear what was said. A
    /// todo task that's held or parked goes back to the backlog. A started
    /// task waiting to build goes in `task line --build` instead.
    Wait {
        /// A key like `ov-12`.
        #[arg(allow_negative_numbers = true)]
        key: String,
        /// Hold it until this local time: `2026-10-05 09:00`, or a date.
        #[arg(long, group = "hold")]
        until: Option<String>,
        /// Hold it until an event: release, recurrence or clear-board.
        #[arg(long, group = "hold")]
        after: Option<String>,
        /// Nobody plans to start it.
        #[arg(long, group = "hold")]
        park: bool,
        /// Clear what was said, a place in a line included.
        #[arg(long, group = "hold")]
        clear: bool,
        /// Which board the key is on, when two repositories use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from. Read from FARCOOLER_ACTOR when not given.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Print a board's line, or replace it.
    ///
    /// Given keys, the line becomes exactly those, in order: the first is
    /// next. A task in the line that isn't named loses its place. The agent
    /// line is for tasks not yet started; `--build` is the line for the
    /// build slot, for tasks in progress or in review.
    Line {
        /// Keys like `ov-12`, the first to start first.
        #[arg(allow_negative_numbers = true)]
        keys: Vec<String>,
        /// The build line rather than the agent line.
        #[arg(long)]
        build: bool,
        /// Empty the line.
        #[arg(long, conflicts_with = "keys")]
        clear: bool,
        /// Which workspace's board, by name, prefix or id. Defaults to the
        /// pane's own, then to the repository's Main.
        #[arg(long)]
        workspace: Option<String>,
        /// Which repository, when there are several.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from. Read from FARCOOLER_ACTOR when not given.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Record a subagent working a task, or that it ended.
    ///
    /// Run it from the session the subagent runs in: the session, folder and
    /// pane are read from there, so the runner can follow it. The runner sees
    /// a Claude subagent end. For codex, or one you stop using, run this again
    /// with `--done`. A backlog or todo task moves to in progress.
    Worker {
        /// A key like `ov-12`.
        #[arg(allow_negative_numbers = true)]
        key: String,
        /// The subagent's id from its launch result (Claude's `agentId`), or
        /// codex's agent path. With `--done` and no id, every subagent open
        /// on the task ends.
        #[arg(long, required_unless_present = "done")]
        subagent: Option<String>,
        /// claude or codex.
        #[arg(long, default_value = "claude")]
        harness: String,
        /// What it's doing, in a few words. Claude's own description is read
        /// when this isn't given.
        #[arg(long)]
        label: Option<String>,
        /// The model it runs, when it isn't read from the session.
        #[arg(long)]
        model: Option<String>,
        /// It ended: it finished, or with `--stopped`, was stopped.
        #[arg(long)]
        done: bool,
        /// With `--done`: it was stopped rather than finished.
        #[arg(long, requires = "done")]
        stopped: bool,
        /// Which board the key is on, when two repositories use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from. Read from FARCOOLER_ACTOR when not given.
        #[arg(long)]
        actor: Option<String>,
    },
}

/// What `task worker` reads from the session it runs in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SessionEnv {
    /// `CLAUDE_CODE_SESSION_ID`: the parent session, the same in its
    /// subagents.
    pub claude_session: Option<String>,
    pub cwd: Option<String>,
    /// `TMUX_PANE` and `TMUX`.
    pub tmux_pane: Option<String>,
    pub tmux: Option<String>,
}

impl SessionEnv {
    pub(super) fn from_process() -> SessionEnv {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        SessionEnv {
            claude_session: var("CLAUDE_CODE_SESSION_ID"),
            cwd: std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()),
            tmux_pane: var("TMUX_PANE"),
            tmux: var("TMUX"),
        }
    }
}

/// Refused before anything is sent: a runner without the capability would
/// drop what it doesn't know, and the write would look like it worked.
fn needs(link: &impl DispatchLink, which: &str) -> Result<(), String> {
    if link.capabilities().iter().any(|c| c == which) {
        return Ok(());
    }
    Err(match which {
        capability::TASK_WAITS => "this runner needs an update to record when a task starts",
        _ => "this runner needs an update to record subagents",
    }
    .to_string())
}

/// This module's refusal words, in this CLI's style (see `said_about`).
fn said_here(what: &str) -> Option<&'static str> {
    Some(match what {
        "line" => "name a line: the agent line, or --build",
        "line_status" => "the agent line takes tasks not yet started, and the build line tasks in progress or in review",
        "other_board" => "a line is one board's, and those tasks aren't all on it",
        "task_twice" => "a task can only be in a line once",
        "wait_status" => "that task has started. to hold it for the build slot, use `farcooler task line --build`",
        "until" => "that time has passed. name one still to come",
        "event" => "use release, recurrence or clear-board",
        "task_closed" => "that task is done or cancelled, so nothing works it now",
        "harness" => "use claude or codex",
        "agent_id" => "name the subagent with --subagent, the id from its launch result",
        "end_reason" => "a subagent ends finished, or with --stopped",
        _ => return None,
    })
}

/// A refusal from the runner: this module's sentence for a word it knows,
/// else `refused`'s.
fn refused_here(err: ClientError, invalid: &str) -> Box<dyn std::error::Error> {
    if let ClientError::Daemon { code, what, .. } = &err
        && let Some(said) = said_here(what)
    {
        return Box::new(super::Refused::naming(said.to_string(), *code, what.clone()));
    }
    refused(err, invalid)
}

/// The wait `task wait`'s flags name: `None` to clear. `now` is Unix
/// milliseconds, passed in so a test can stand at any time.
fn wait_asked(
    until: Option<&str>,
    after: Option<&str>,
    park: bool,
    clear: bool,
    now: i64,
) -> Result<pb::TaskSetWait, String> {
    let kind = |k: pb::TaskWaitKind| k as i32;
    match (until, after, park, clear) {
        (Some(text), ..) => {
            let at = crate::report::local_time(text.trim())
                .ok_or_else(|| format!("{text:?} isn't a time. write it as 2026-10-05 09:00, or a date"))?;
            if at <= now {
                return Err("that time has passed. name one still to come".into());
            }
            Ok(pb::TaskSetWait { kind: kind(pb::TaskWaitKind::Until), until: at, ..Default::default() })
        }
        (None, Some(word), ..) => {
            let event = match word.trim().replace('_', "-").as_str() {
                "release" => pb::TaskWaitEvent::Release,
                "recurrence" => pb::TaskWaitEvent::Recurrence,
                "clear-board" => pb::TaskWaitEvent::ClearBoard,
                _ => return Err(format!("{word:?} isn't an event. use release, recurrence or clear-board")),
            };
            Ok(pb::TaskSetWait { kind: kind(pb::TaskWaitKind::After), event: event as i32, ..Default::default() })
        }
        (None, None, true, _) => Ok(pb::TaskSetWait { kind: kind(pb::TaskWaitKind::Parked), ..Default::default() }),
        (None, None, false, true) => Ok(pb::TaskSetWait::default()),
        (None, None, false, false) => Err("say how it waits: --until, --after, --park or --clear".into()),
    }
}

/// `task worker`'s request, filled from the session it runs in. The second
/// half is what to say when the runner won't be able to follow it.
fn worker_request(
    task: &pb::Task,
    cmd: &WorkerArgs,
    env: &SessionEnv,
    actor: &str,
) -> Result<(pb::TaskWorkerSet, Option<String>), String> {
    let harness = cmd.harness.trim().to_lowercase();
    if !["claude", "codex"].contains(&harness.as_str()) {
        return Err(format!("{:?} isn't a harness this records. use claude or codex", cmd.harness));
    }
    let agent = cmd.subagent.as_deref().map(str::trim).unwrap_or_default();
    if agent.is_empty() && !cmd.done {
        return Err("name the subagent with --subagent, the id from its launch result".into());
    }
    let session = if harness == "claude" { env.claude_session.clone() } else { None };
    let unobserved = (!cmd.done && session.is_none()).then(|| {
        let reason = if harness == "claude" {
            "CLAUDE_CODE_SESSION_ID isn't set here, so the runner can't find its session"
        } else {
            "the runner can't see a codex subagent working"
        };
        format!(
            "recorded, unobserved: {reason}. when it's finished, run `farcooler task worker {} --subagent {} \
             --harness {harness} --done`",
            task.key,
            agent
        )
    });
    let request = pb::TaskWorkerSet {
        task_id: task.id.clone(),
        harness,
        agent_id: agent.to_string(),
        session_id: session,
        session_cwd: env.cwd.clone(),
        tmux_pane: env.tmux_pane.clone(),
        tmux_socket: env.tmux.clone(),
        label: cmd.label.clone(),
        model: cmd.model.clone(),
        end: cmd.done,
        end_reason: match (cmd.done, cmd.stopped) {
            (true, true) => "stopped".into(),
            (true, false) => "finished".into(),
            _ => String::new(),
        },
        actor: actor.to_string(),
    };
    Ok((request, unobserved))
}

/// `task worker`'s flags, apart from the board's.
pub(super) struct WorkerArgs {
    /// `None` with `done`: every open one.
    pub subagent: Option<String>,
    pub harness: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub done: bool,
    pub stopped: bool,
}

pub(super) async fn run<L: DispatchLink>(link: &mut L, cmd: StartCmd, json: bool, env: &SessionEnv) -> Fallible {
    match cmd {
        StartCmd::Wait { key, until, after, park, clear, repo, actor } => {
            needs(link, capability::TASK_WAITS)?;
            let actor = actor_for(actor.as_deref())?;
            let asked = wait_asked(until.as_deref(), after.as_deref(), park, clear, super::now_millis())?;
            let task = find_task(link, repo.as_deref(), &key).await?;
            let request = pb::TaskSetWait { task_id: task.id.clone(), actor: actor.to_string(), ..asked };
            let r = link
                .call(with(req_for("task.set_wait", uuid_of(&task.id)), request::Payload::TaskSetWait(request)))
                .await
                .map_err(|e| refused_here(e, "that wait couldn't be recorded"))?;
            let result::Value::Task(after) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                println!("{}", tasks_json::task_json(&after, super::now_millis()));
                return Ok(());
            }
            if after.status != task.status {
                println!("{}  moved to {}", after.key, status_word(after.status));
            }
            let said = starts_word(&after).unwrap_or_else(|| "nothing said about when it starts".into());
            println!("{}  {said}", after.key);
        }

        StartCmd::Line { keys, build, clear, workspace, repo, actor } => {
            needs(link, capability::TASK_WAITS)?;
            let line = if build { pb::TaskLine::Build } else { pb::TaskLine::Agent };
            let board = board_for(link, repo.as_deref(), workspace.as_deref(), std::env::var(WORKSPACE_ENV).ok()).await?;
            let mut tasks = Vec::with_capacity(keys.len());
            for key in &keys {
                tasks.push(find_task(link, repo.as_deref(), key).await?);
            }
            let workspace = match (&board.workspace, tasks.first()) {
                (Some(ws), _) => uuid_of(&ws.id),
                (None, Some(first)) => uuid_of(&first.workspace_id),
                (None, None) => main_of(link, &board).await?,
            };
            let in_line = if keys.is_empty() && !clear {
                line_of(tasks_in(link, &board, None, None).await?, workspace, line)
            } else {
                let actor = actor_for(actor.as_deref())?;
                let request = pb::TaskSetLine {
                    workspace_id: id_bytes(workspace),
                    line: line as i32,
                    task_ids: tasks.iter().map(|t| t.id.clone()).collect(),
                    actor: actor.to_string(),
                };
                let r = link
                    .call(with(req_for("task.set_line", workspace), request::Payload::TaskSetLine(request)))
                    .await
                    .map_err(|e| refused_here(e, "that line couldn't be set"))?;
                let result::Value::TaskList(l) = expect_value(r.value)? else {
                    return Err(crate::daemon_link::UNREADABLE.into());
                };
                l.items
            };
            print!("{}", render_line(&in_line, line, json));
        }

        StartCmd::Worker { key, subagent, harness, label, model, done, stopped, repo, actor } => {
            needs(link, capability::TASK_WORKERS)?;
            let actor = actor_for(actor.as_deref())?;
            let task = find_task(link, repo.as_deref(), &key).await?;
            let args = WorkerArgs { subagent, harness, label, model, done, stopped };
            let (request, unobserved) = worker_request(&task, &args, env, &actor.to_string())?;
            let r = link
                .call(with(req_for("task.worker", uuid_of(&task.id)), request::Payload::TaskWorkerSet(request)))
                .await
                .map_err(|e| match e {
                    ClientError::Daemon { code, .. } if done && farcooler_core::error::word_for(code) == "not-found" => {
                        match args.subagent.as_deref().map(str::trim) {
                            Some(id) => format!("no subagent {id} is recorded on {}", task.key),
                            None => format!("no subagent is open on {}", task.key),
                        }
                        .into()
                    }
                    e => refused_here(e, "that subagent couldn't be recorded"),
                })?;
            let result::Value::Task(after) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                let row = tasks_json::task_json(&after, super::now_millis());
                println!("{}", serde_json::json!({ "task": row, "observed": unobserved.is_none() }));
                return Ok(());
            }
            if after.status != task.status {
                println!("{}  moved to {}", after.key, status_word(after.status));
            }
            let what = if done { "subagent ended" } else { "subagent recorded" };
            println!("{}  {what}", after.key);
            if let Some(said) = unobserved {
                println!("{said}");
            }
        }
    }
    Ok(())
}

/// The repository's Main, for a line asked of no workspace.
async fn main_of<L: DispatchLink>(link: &mut L, board: &Board) -> Result<Uuid, Box<dyn std::error::Error>> {
    let all = workspaces_on(link, Some(board.repository)).await?;
    all.iter()
        .find(|w| w.is_main)
        .map(|w| uuid_of(&w.id))
        .ok_or_else(|| "this runner has no board to keep a line on. name one with --workspace".into())
}

/// A board's line, in order, from its rows.
fn line_of(tasks: Vec<pb::Task>, workspace: Uuid, line: pb::TaskLine) -> Vec<pb::Task> {
    let mut in_line: Vec<pb::Task> = tasks
        .into_iter()
        .filter(|t| uuid_of(&t.workspace_id) == workspace)
        .filter(|t| {
            t.wait.as_ref().is_some_and(|w| {
                w.kind == pb::TaskWaitKind::InLine as i32 && w.line == line as i32 && w.position > 0
            })
        })
        .collect();
    in_line.sort_by_key(|t| t.wait.as_ref().map(|w| w.position));
    in_line
}

fn render_line(tasks: &[pb::Task], line: pb::TaskLine, json: bool) -> String {
    let name = task_starts_json::line_word(line as i32);
    if json {
        let now = super::now_millis();
        let rows: Vec<_> = tasks.iter().map(|t| tasks_json::task_json(t, now)).collect();
        return format!("{}\n", serde_json::json!({ "line": name, "tasks": rows }));
    }
    if tasks.is_empty() {
        return format!("the {name} line is empty\n");
    }
    tasks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{:>2}  {:<8}  {:<14}  {}\n", i + 1, t.key, status_word(t.status), truncate(&t.title, 60)))
        .collect()
}

/// `3rd`, `11th`, `22nd`.
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// `ov-1`, `ov-1 and ov-2`, `ov-1, ov-2 and ov-3`.
fn and_list(keys: &[String]) -> String {
    match keys {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// When a task starts, in this CLI's words, or `None` when there's nothing
/// to say. Read in the order a person needs it: what it's blocked on, then
/// what was said, then, for todo, that it's ready.
///
/// A wait arrives only where it fits the status (the runner hides one that
/// doesn't), so this doesn't check the status again.
pub(super) fn starts_word(task: &pb::Task) -> Option<String> {
    if !task.waiting_on.is_empty() {
        return Some(format!("waiting on {}", and_list(&task.waiting_on)));
    }
    if let Some(wait) = &task.wait {
        let build = wait.line == pb::TaskLine::Build as i32;
        return Some(match pb::TaskWaitKind::try_from(wait.kind) {
            Ok(pb::TaskWaitKind::InLine) => match (build, wait.position) {
                (false, 1) => "next to start".into(),
                (true, 1) => "builds next".into(),
                (false, n) => format!("{} in line", ordinal(n)),
                (true, n) => format!("{} in line to build", ordinal(n)),
            },
            Ok(pb::TaskWaitKind::Until) => format!("starts {}", farcooler_core::local_time::moment(wait.until)),
            Ok(pb::TaskWaitKind::After) => match pb::TaskWaitEvent::try_from(wait.event) {
                Ok(pb::TaskWaitEvent::Release) => "after the next release".into(),
                Ok(pb::TaskWaitEvent::Recurrence) => "if it happens again".into(),
                Ok(pb::TaskWaitEvent::ClearBoard) => "when nothing else is waiting".into(),
                _ => "after an event".into(),
            },
            Ok(pb::TaskWaitKind::Parked) => "not planned".into(),
            _ => return None,
        });
    }
    (task.status == pb::TaskStatus::Todo as i32).then(|| "ready to start".into())
}

/// `task show`'s `starts` section, or nothing.
pub(super) fn starts_section(task: &pb::Task) -> String {
    let Some(word) = starts_word(task) else { return String::new() };
    let mut out = format!("starts\n  {word}\n");
    if let Some(wait) = task.wait.as_ref().filter(|w| !w.ahead.is_empty()) {
        out.push_str(&format!("  after {}\n", and_list(&wait.ahead)));
    }
    out
}

/// `task show`'s `workers` section: each subagent, how it is, and its label.
pub(super) fn workers_section(task: &pb::Task, now: i64) -> String {
    if task.workers.is_empty() {
        return String::new();
    }
    let mut out = String::from("workers\n");
    for w in &task.workers {
        let ago = |at: i64| match spoken_gap(stale_for_seconds(at, now)).as_str() {
            "just now" => "just now".to_string(),
            gap => format!("{gap} ago"),
        };
        let how = match pb::TaskWorkerState::try_from(w.state) {
            Ok(pb::TaskWorkerState::Running) => format!("working, started {}", ago(w.started_at)),
            Ok(pb::TaskWorkerState::Finished) => format!("finished {}", ago(w.ended_at)),
            Ok(pb::TaskWorkerState::Stopped) => format!("stopped {}", ago(w.ended_at)),
            _ => format!("started {}, unobserved", ago(w.started_at)),
        };
        let label = if w.label.is_empty() { String::new() } else { format!("  {}", w.label) };
        out.push_str(&format!("  {} subagent {}  {how}{label}\n", w.harness, w.agent_id));
    }
    out
}

/// A blocker's key for `task show`, and whether it's finished, from the
/// keys the board read named and the task's `waiting_on`. A blocker the read
/// didn't name is said to be not found, never finished. `waits_known` is
/// whether the runner sends `waiting_on` at all; from one that doesn't,
/// every block reads as unfinished, as it always did.
pub(super) fn block_line(
    block: &pb::TaskBlock,
    task: &pb::Task,
    keys: &HashMap<Uuid, String>,
    waits_known: bool,
) -> String {
    let reason = super::said(&block.reason);
    let Some(key) = keys.get(&uuid_of(&block.blocked_by)) else {
        // Not on the board read (a block across repositories): whether it
        // finished can't be told, so it isn't said.
        let short = crate::short_bytes(&block.blocked_by);
        return format!("  waits on {short}, a task not found on this board{reason}\n");
    };
    let finished = waits_known && !task.waiting_on.contains(key);
    if finished { format!("  waited on {key}, now finished{reason}\n") } else { format!("  waits on {key}{reason}\n") }
}

#[cfg(test)]
#[path = "task_starts_tests.rs"]
mod tests;

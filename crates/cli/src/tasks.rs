//! `farcooler task` — the board, for whoever is doing the work.
//!
//! **The split this surface exists to keep.** Current understanding is mutable
//! and lives on the task row; the record of how that understanding was reached
//! is append-only and lives in typed notes. `task set` revises the row. `task
//! note` appends to the record. There is no command here, and no flag on one,
//! that edits or deletes a note — and there must never be. `task_notes` carries
//! a `BEFORE UPDATE` trigger (`task_notes_forbid_update`) that refuses
//! unconditionally, so such a flag would compile, ship, and fail at runtime in
//! front of whoever typed it. Correcting the record is `task note --supersedes`,
//! which writes a NEW note pointing at the old one and leaves both readable
//! forever.
//!
//! **Reads are narrow on purpose.** A manager surveying thirty tasks on every
//! wake-up must not pull thirty histories: `task list` returns rows, and `task
//! show --fields intent,acceptance` prints the two sections it was asked for.
//! The loop gets slower as the board gets more useful, which is exactly
//! backwards, unless the cheap read is the one that is easy to type.
//!
//! **Every write names who made it, out loud.** proto3 cannot tell an omitted
//! `actor` from an empty one, and the daemon reads empty as `user` — so an
//! agent that forgets to name itself files its work under a person, and nothing
//! in the record ever says otherwise. This CLI is the layer that knows who is
//! calling, so it resolves an actor for every write and always sends a
//! non-empty word, `user` included. See `actor_from`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_transport::ClientError;
use uuid::Uuid;

use crate::{
    Fallible, Link, connect_to, expect_value, id_bytes, list_repositories, req_for,
    resolve_repository, short_bytes, truncate, uuid_of, with,
};

/// The ticket a dispatched pane is working.
///
/// **Nothing in this tree sets it yet.** Both of these are the contract with
/// whatever comes to dispatch panes; this side reads them, and until the other
/// side exports them a key is typed and every write files as `user`. Named as
/// constants so there is one word to grep for on the day that changes.
const TASK_ENV: &str = "FARCOOLER_TASK";

/// Who a dispatched pane is. `user`, `manager`, or `agent:<terminal id>` — the
/// same vocabulary `Actor` parses, because it IS what parses it. See
/// `TASK_ENV`: nothing sets this yet either.
const ACTOR_ENV: &str = "FARCOOLER_ACTOR";

/// The sections `show` prints, in the order it prints them.
///
/// `key` is not here: it is printed unconditionally, because a card that does
/// not say which task it is about is a card an agent can misfile.
const SECTIONS: [&str; 8] =
    ["title", "status", "intent", "acceptance", "constraints", "labels", "blocks", "history"];

#[derive(Subcommand)]
pub enum TaskCmd {
    /// The board: one row per task.
    ///
    /// The cheap read. Rows only — no intent, no acceptance, no history — so
    /// that surveying the whole board costs one call whatever is on it.
    List {
        /// Which repository's board. Defaults to the pane's own, then to the
        /// only one there is.
        #[arg(long)]
        repo: Option<String>,
        /// Only tasks sitting in this status.
        #[arg(long)]
        status: Option<String>,
        /// Only tasks that have sat where they are for longer than this:
        /// `45s`, `10m`, `2h`, `3d`.
        ///
        /// A different question rather than a filter on the same one — oldest
        /// first, and finished work left out — so it cannot be combined with
        /// `--status`, which the runner ignores in this mode.
        #[arg(long = "stale-for", conflicts_with = "status")]
        stale_for: Option<String>,
    },
    /// One task: what is understood, and how it came to be understood.
    ///
    /// Defaults to the whole card. Name `--fields` to pay for less.
    Show {
        /// A key like `fc-42`, or the last eight of a task's id. Defaults to
        /// the pane's own task.
        key: Option<String>,
        /// Which sections to print, comma separated: title, status, intent,
        /// acceptance, constraints, labels, blocks, history.
        #[arg(long)]
        fields: Option<String>,
        /// Narrow the history to one kind of note: decision, finding,
        /// question, answer, progress, comment, status_change, created.
        #[arg(long)]
        notes: Option<String>,
        /// Which board the key is on. Only needed when two repositories are
        /// registered here and both use it.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Put a new task on the board, with whatever is understood so far.
    Create {
        /// One line, for the board. At most 200 characters.
        #[arg(long)]
        title: String,
        /// What this task is for, in prose. Revisable at any time.
        #[arg(long)]
        intent: Option<String>,
        /// One checkable thing. Repeatable.
        #[arg(long = "accept")]
        accept: Vec<String>,
        /// One thing this task may not do. Repeatable.
        #[arg(long = "constraint")]
        constraint: Vec<String>,
        /// A word to file this task under. Repeatable.
        #[arg(long = "label")]
        label: Vec<String>,
        /// The workspace this task will use, if it already has one.
        #[arg(long)]
        workspace: Option<String>,
        /// Which repository's board. Defaults to the only one there is.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
        /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
        /// says — always sent, never left for the runner to assume.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Revise what is currently understood, and move the task.
    ///
    /// The mutable half. This writes no note: intent and acceptance are meant
    /// to be rewritten as understanding improves, and a log entry per wording
    /// change would bury the reasoning the record exists to keep. A status move
    /// IS recorded, by the runner, in the same transaction that moves it.
    Set {
        /// A key like `fc-42`, or the last eight of a task's id. Defaults to
        /// the pane's own task.
        key: Option<String>,
        /// backlog, todo, needs_decision, in_progress, in_review, done,
        /// cancelled.
        #[arg(long)]
        status: Option<String>,
        /// Rewrite the one line the board shows.
        #[arg(long)]
        title: Option<String>,
        /// Rewrite what this task is for. This is the mutable half: revise it
        /// freely as understanding improves.
        #[arg(long)]
        intent: Option<String>,
        /// Replace the acceptance list. Repeatable.
        #[arg(long = "accept")]
        accept: Vec<String>,
        /// Replace the constraints. Repeatable.
        #[arg(long = "constraint")]
        constraint: Vec<String>,
        /// Replace the labels. Repeatable.
        #[arg(long = "label")]
        label: Vec<String>,
        /// Tick acceptance line N, counting from 1. Repeatable.
        #[arg(long = "met")]
        met: Vec<usize>,
        /// Untick acceptance line N, counting from 1. Repeatable.
        #[arg(long = "unmet")]
        unmet: Vec<usize>,
        /// Move this task onto a workspace's lane.
        #[arg(long)]
        workspace: Option<String>,
        /// Which board the key is on. Only needed when two repositories are
        /// registered here and both use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
        /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
        /// says — always sent, never left for the runner to assume.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Append one entry to a task's record.
    ///
    /// Nothing here changes an entry already written, and nothing ever will.
    /// Correcting the record is `--supersedes`, which writes a new entry
    /// pointing at the old one; both stay readable.
    Note {
        /// A key like `fc-42`, or the last eight of a task's id. Defaults to
        /// the pane's own task.
        key: Option<String>,
        /// decision, finding, question, answer, progress, or comment.
        #[arg(long)]
        kind: String,
        /// What the entry says. Kept forever, and never edited.
        #[arg(long)]
        body: String,
        /// What was considered and turned down. Repeatable; stored as the
        /// note's structure rather than buried in its prose.
        #[arg(long = "rejected")]
        rejected: Vec<String>,
        /// The note this one replaces, by the short id `show` prints.
        #[arg(long)]
        supersedes: Option<String>,
        /// Which board the key is on. Only needed when two repositories are
        /// registered here and both use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
        /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
        /// says — always sent, never left for the runner to assume.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Ask the user something, and summon them.
    ///
    /// One command rather than two, because a question that does not move the
    /// task is a task that asks and summons nobody — and a task moved to
    /// `needs_decision` with no question on it tells whoever arrives nothing.
    Ask {
        /// A key like `fc-42`, or the last eight of a task's id. Defaults to
        /// the pane's own task.
        key: Option<String>,
        /// What only the user can answer.
        #[arg(long)]
        body: String,
        /// One answer worth offering. Repeatable.
        #[arg(long = "option")]
        option: Vec<String>,
        /// Which board the key is on. Only needed when two repositories are
        /// registered here and both use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
        /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
        /// says — always sent, never left for the runner to assume.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Record that a task is waiting on another, or clear that.
    Block {
        /// A key like `fc-42`, or the last eight of a task's id. Defaults to
        /// the pane's own task.
        key: Option<String>,
        /// The task this one waits on.
        #[arg(long = "on")]
        on: String,
        /// Why it has to wait.
        ///
        /// Only settable when the edge is written. Blocking a pair that is
        /// already blocked is refused, so correcting a reason is `--clear`
        /// and then block again.
        #[arg(long)]
        reason: Option<String>,
        /// Remove the edge instead of writing it.
        #[arg(long)]
        clear: bool,
        /// Which board the key is on. Only needed when two repositories are
        /// registered here and both use it.
        #[arg(long)]
        repo: Option<String>,
        /// Who this write is from: `user`, `manager`, or `agent:<terminal id>`.
        /// Read from FARCOOLER_ACTOR when not given, and `user` when neither
        /// says — always sent, never left for the runner to assume.
        #[arg(long)]
        actor: Option<String>,
    },
    /// Every note in a repository whose body carries a phrase.
    ///
    /// What makes the board a memory rather than a queue. "Why did we do it
    /// this way" is asked months later, about a task nobody remembers the key
    /// of. Matched literally: `%` and `_` are characters, not wildcards.
    Search {
        /// The phrase to look for, matched literally.
        query: String,
        /// Only entries of this kind: decision, finding, question, answer,
        /// progress, comment, status_change, created.
        #[arg(long)]
        kind: Option<String>,
        /// Which repository's record. Defaults to the pane's own board, then
        /// to the only one there is.
        #[arg(long)]
        repo: Option<String>,
    },
}

pub async fn task(runner: Option<&str>, cmd: TaskCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;

    match cmd {
        TaskCmd::List { repo, status, stale_for } => {
            let repository = repository_for(&mut link, repo.as_deref()).await?;
            let status = match status.as_deref() {
                Some(word) => Some(status_named(word)?),
                None => None,
            };
            let stale = match stale_for.as_deref() {
                Some(word) => Some(parse_gap(word)?),
                None => None,
            };
            let items = tasks_in(&mut link, repository, status, stale).await?;

            if json {
                println!("{}", render_list_json(&items));
                return Ok(());
            }
            if items.is_empty() {
                println!("nothing on this board");
                return Ok(());
            }
            let now = now_millis();
            for t in &items {
                println!(
                    "{:<8}  {:<14}  {:>6}  {}",
                    t.key,
                    status_word(t.status),
                    spoken_gap(stale_for_seconds(t.status_since, now)),
                    truncate(&t.title, 60)
                );
            }
        }

        TaskCmd::Show { key, fields, notes, repo } => {
            one_question_at_a_time(json, fields.as_deref())?;
            let key = wanted_key(key)?;
            let task = find_task(&mut link, repo.as_deref(), &key).await?;
            let kind = match notes.as_deref() {
                Some(word) => Some(kind_named(word)?),
                None => None,
            };
            let detail = detail_of(&mut link, uuid_of(&task.id), kind).await?;

            if json {
                println!("{}", render_show_json(&detail));
                return Ok(());
            }
            let asked = asked_fields(fields.as_deref())?;
            let borrowed: Vec<&str> = asked.iter().map(String::as_str).collect();
            print!("{}", render_show(&detail, &borrowed, kind));
        }

        TaskCmd::Create {
            title,
            intent,
            accept,
            constraint,
            label,
            workspace,
            repo,
            actor,
        } => {
            let actor = actor_for(actor.as_deref())?;
            let repository = repository_for(&mut link, repo.as_deref()).await?;
            let workspace_id = match workspace.as_deref() {
                Some(name) => Some(crate::resolve_workspace_id(&mut link, name).await?),
                None => None,
            };
            let r = link
                .call(with(
                    req_for("task.create", repository),
                    request::Payload::TaskCreate(pb::TaskCreate {
                        repository_id: id_bytes(repository),
                        title: title.trim().to_string(),
                        intent: intent.unwrap_or_default(),
                        acceptance: accept.iter().map(|t| new_acceptance(t)).collect(),
                        constraints: constraint,
                        labels: label,
                        workspace_id: workspace_id.map(id_bytes),
                        actor: actor.to_string(),
                    }),
                ))
                .await
                .map_err(|e| refused(e, "a task needs a title of at most 200 characters"))?;
            let result::Value::Task(created) = expect_value(r.value, "task")? else {
                return Err("the daemon returned the wrong resource".into());
            };

            if json {
                println!("{}", task_json(&created, now_millis()));
                return Ok(());
            }
            println!("{}  {}", created.key, created.title);
        }

        TaskCmd::Set {
            key,
            status,
            title,
            intent,
            accept,
            constraint,
            label,
            met,
            unmet,
            workspace,
            repo,
            actor,
        } => {
            let revising = title.is_some()
                || intent.is_some()
                || workspace.is_some()
                || !accept.is_empty()
                || !constraint.is_empty()
                || !label.is_empty()
                || !met.is_empty()
                || !unmet.is_empty();
            if !revising && status.is_none() {
                return Err("name something to change: --status, --title, --intent, \
                            --accept, --constraint, --label, --met or --unmet"
                    .into());
            }
            let actor = actor_for(actor.as_deref())?;
            let status = match status.as_deref() {
                Some(word) => Some(status_named(word)?),
                None => None,
            };
            let key = wanted_key(key)?;
            let mut task = find_task(&mut link, repo.as_deref(), &key).await?;

            // Revise first, move second. Both orders can leave half the work
            // done if the second call fails, and this is the half that matters:
            // a status move is what wakes somebody up, so the row should
            // already say what it should say by the time it does.
            //
            // WARNING to whoever next adds a field to `pb::Task`: this is a
            // read-modify-write over a row that came from `task.list`, and it
            // sends every field back. A field `task.list` does not populate is
            // a field `task set --title` silently blanks, and nothing in this
            // crate would notice. Populate it in `task.list`, or read the task
            // through `task.get` here first.
            if revising {
                let mut acceptance = if accept.is_empty() {
                    task.acceptance.clone()
                } else {
                    accept.iter().map(|t| new_acceptance(t)).collect()
                };
                for (n, wanted) in
                    met.iter().map(|n| (n, true)).chain(unmet.iter().map(|n| (n, false)))
                {
                    let lines = acceptance.len();
                    let each = if lines == 1 { "line" } else { "lines" };
                    let line = acceptance
                        .get_mut(n.checked_sub(1).unwrap_or(usize::MAX))
                        .ok_or_else(|| {
                            format!("this task has {lines} acceptance {each}, so there is no {n}")
                        })?;
                    line.met = wanted;
                }
                let workspace_id = match workspace.as_deref() {
                    Some(name) => Some(crate::resolve_workspace_id(&mut link, name).await?),
                    None => task.workspace_id.as_ref().map(|b| uuid_of(b.as_ref())),
                };
                let id = uuid_of(&task.id);
                let r = link
                    .call(with(
                        req_for("task.update", id),
                        request::Payload::TaskUpdate(pb::TaskUpdate {
                            task_id: id_bytes(id),
                            expected_version: task.resource_version,
                            title: title.unwrap_or_else(|| task.title.clone()),
                            intent: intent.unwrap_or_else(|| task.intent.clone()),
                            acceptance,
                            constraints: if constraint.is_empty() {
                                task.constraints.clone()
                            } else {
                                constraint
                            },
                            labels: if label.is_empty() { task.labels.clone() } else { label },
                            workspace_id: workspace_id.map(id_bytes),
                            actor: actor.to_string(),
                        }),
                    ))
                    .await
                    .map_err(|e| {
                        refused(e, "a task needs a title of at most 200 characters")
                    })?;
                let result::Value::Task(revised) = expect_value(r.value, "task")? else {
                    return Err("the daemon returned the wrong resource".into());
                };
                task = revised;
            }

            if let Some(status) = status {
                let id = uuid_of(&task.id);
                let r = link
                    .call(with(
                        req_for("task.set_status", id),
                        request::Payload::TaskSetStatus(pb::TaskSetStatus {
                            task_id: id_bytes(id),
                            status: pb_status(status),
                            actor: actor.to_string(),
                        }),
                    ))
                    .await
                    .map_err(|e| refused(e, "that is not a status this runner knows"))?;
                let result::Value::Task(moved) = expect_value(r.value, "task")? else {
                    return Err("the daemon returned the wrong resource".into());
                };
                task = moved;
            }

            if json {
                println!("{}", task_json(&task, now_millis()));
                return Ok(());
            }
            println!("{}  {}  {}", task.key, status_word(task.status), truncate(&task.title, 60));
        }

        TaskCmd::Note { key, kind, body, rejected, supersedes, repo, actor } => {
            let kind = writable_kind(&kind)?;
            let actor = actor_for(actor.as_deref())?;
            let key = wanted_key(key)?;
            let task = find_task(&mut link, repo.as_deref(), &key).await?;
            let id = uuid_of(&task.id);

            let superseded = match supersedes.as_deref() {
                Some(short) => {
                    let detail = detail_of(&mut link, id, None).await?;
                    Some(find_note(&detail.notes, short, kind)?)
                }
                None => None,
            };
            let extra = if rejected.is_empty() {
                String::new()
            } else {
                serde_json::json!({ "rejected": rejected }).to_string()
            };

            let written = append_note(
                &mut link,
                id,
                kind,
                &body,
                &extra,
                superseded,
                actor,
            )
            .await?;

            if json {
                println!("{}", note_json(&written));
                return Ok(());
            }
            println!("{}  {}  {}", task.key, short_bytes(&written.id), kind.as_str());
        }

        TaskCmd::Ask { key, body, option, repo, actor } => {
            let actor = actor_for(actor.as_deref())?;
            let key = wanted_key(key)?;
            let task = find_task(&mut link, repo.as_deref(), &key).await?;
            let id = uuid_of(&task.id);
            let extra = if option.is_empty() {
                String::new()
            } else {
                serde_json::json!({ "options": option }).to_string()
            };

            // The question is written before the task moves. If the move fails
            // the record still says what was asked; the other order would move
            // a task to `needs_decision` with nothing on it saying why, which
            // is worse for whoever it summons.
            let written =
                append_note(&mut link, id, NoteKind::Question, &body, &extra, None, actor).await?;
            let r = link
                .call(with(
                    req_for("task.set_status", id),
                    request::Payload::TaskSetStatus(pb::TaskSetStatus {
                        task_id: id_bytes(id),
                        status: pb_status(TaskStatus::NeedsDecision),
                        actor: actor.to_string(),
                    }),
                ))
                .await
                .map_err(|e| {
                    refused(e, "the question was written, but the task would not move")
                })?;
            let result::Value::Task(moved) = expect_value(r.value, "task")? else {
                return Err("the daemon returned the wrong resource".into());
            };

            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "note": note_json(&written),
                        "task": task_json(&moved, now_millis()),
                    })
                );
                return Ok(());
            }
            println!("{}  {}  waiting on you", moved.key, status_word(moved.status));
        }

        TaskCmd::Block { key, on, reason, clear, repo, actor } => {
            let actor = actor_for(actor.as_deref())?;
            let key = wanted_key(key)?;
            let task = find_task(&mut link, repo.as_deref(), &key).await?;
            let blocker = find_task(&mut link, repo.as_deref(), &on).await?;
            let id = uuid_of(&task.id);

            let r = link
                .call(with(
                    req_for("task.block", id),
                    request::Payload::TaskBlockSet(pb::TaskBlockSet {
                        task_id: id_bytes(id),
                        blocked_by: blocker.id.clone(),
                        reason: reason.unwrap_or_default(),
                        clear,
                        actor: actor.to_string(),
                    }),
                ))
                .await
                .map_err(|e| {
                    refused(e, "those two tasks would end up waiting on each other")
                })?;
            let result::Value::TaskBlockList(l) = expect_value(r.value, "task_block_list")? else {
                return Err("the daemon returned the wrong resource".into());
            };

            if json {
                let items: Vec<_> = l.items.iter().map(block_json).collect();
                println!("{}", serde_json::json!({ "blocks": items }));
                return Ok(());
            }
            if l.items.is_empty() {
                println!("{} is waiting on nothing", task.key);
                return Ok(());
            }
            for b in &l.items {
                println!("{} waits on {}  {}", task.key, short_bytes(&b.blocked_by), b.reason);
            }
        }

        TaskCmd::Search { query, kind, repo } => {
            let repository = repository_for(&mut link, repo.as_deref()).await?;
            let kind = match kind.as_deref() {
                Some(word) => Some(kind_named(word)?),
                None => None,
            };
            let r = link
                .call(with(
                    req_for("task.search", repository),
                    request::Payload::TaskSearch(pb::TaskSearchRequest {
                        repository_id: id_bytes(repository),
                        query: query.clone(),
                        kind: kind.map(pb_note_kind).unwrap_or_default(),
                    }),
                ))
                .await
                .map_err(|e| refused(e, "say what to search for"))?;
            let result::Value::TaskNoteHitList(l) = expect_value(r.value, "task_note_hit_list")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };

            // A hit names a task by id, and the whole point of this read is a
            // task nobody remembers the key of. One listing turns every id in
            // the answer back into the word a person would type next.
            let keys = key_index(&tasks_in(&mut link, repository, None, None).await?);

            if json {
                let items: Vec<_> = l
                    .items
                    .iter()
                    .map(|hit| {
                        let note = hit.note.clone().unwrap_or_default();
                        serde_json::json!({
                            "key": keys.get(&uuid_of(&note.task_id)),
                            "note": note_json(&note),
                            "superseded": hit.superseded,
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({ "hits": items }));
                return Ok(());
            }
            if l.items.is_empty() {
                println!("nothing in this board's record says that");
                return Ok(());
            }
            for hit in &l.items {
                let Some(note) = &hit.note else { continue };
                let key = keys
                    .get(&uuid_of(&note.task_id))
                    .cloned()
                    .unwrap_or_else(|| short_bytes(&note.task_id));
                let since = if hit.superseded { "  (superseded)" } else { "" };
                println!(
                    "{:<8}  {:<14}  {}{}",
                    key,
                    kind_word(note.kind),
                    truncate(first_line(&note.body), 70),
                    since
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering — the pure half, so what is printed can be tested without a daemon
// ---------------------------------------------------------------------------

/// One task's card, printing only the sections that were asked for.
///
/// `fields` empty means the whole card. Naming any narrows it to those, which
/// is what keeps a manager's wake-up affordable: history is the expensive
/// section and it is the one a survey never needs. `notes` narrows the history
/// further, to one kind — `decision` alone answers "why is it like this"
/// without a wall of progress chatter.
///
/// The key prints whatever was asked for. A card that does not say which task
/// it describes is a card that can be filed against the wrong one.
fn render_show(detail: &pb::TaskDetail, fields: &[&str], notes: Option<NoteKind>) -> String {
    let mut out = String::new();
    let Some(task) = &detail.task else { return out };
    out.push_str(&format!("{}\n", task.key));

    if wants(fields, "title") {
        out.push_str(&format!("title\n  {}\n", task.title));
    }
    if wants(fields, "status") {
        let gap = spoken_gap(stale_for_seconds(task.status_since, now_millis()));
        out.push_str(&format!("status\n  {}  for {}\n", status_word(task.status), gap));
    }
    if wants(fields, "intent") && !task.intent.is_empty() {
        out.push_str("intent\n");
        for line in task.intent.lines() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if wants(fields, "acceptance") && !task.acceptance.is_empty() {
        out.push_str("acceptance\n");
        for (n, item) in task.acceptance.iter().enumerate() {
            let box_ = if item.met { "x" } else { " " };
            out.push_str(&format!("  {:>2}. [{}] {}\n", n + 1, box_, item.text));
        }
    }
    if wants(fields, "constraints") && !task.constraints.is_empty() {
        out.push_str("constraints\n");
        for c in &task.constraints {
            out.push_str(&format!("  {c}\n"));
        }
    }
    if wants(fields, "labels") && !task.labels.is_empty() {
        out.push_str(&format!("labels\n  {}\n", task.labels.join(", ")));
    }
    if wants(fields, "blocks") && !detail.blocks.is_empty() {
        out.push_str("blocks\n");
        for b in &detail.blocks {
            out.push_str(&format!("  waits on {}  {}\n", short_bytes(&b.blocked_by), b.reason));
        }
    }
    // `--notes` is a request for history, so it brings the section with it.
    // Naming a kind and being handed no notes at all is the accept-and-ignore
    // this surface must not do: the flag was typed, so it decides something.
    if wants(fields, "history") || notes.is_some() {
        let shown: Vec<&pb::TaskNote> = detail
            .notes
            .iter()
            .filter(|n| notes.is_none_or(|k| note_kind_of(n.kind) == Some(k)))
            .collect();
        if !shown.is_empty() {
            out.push_str("history\n");
            for n in shown {
                out.push_str(&format!(
                    "  {}  {:<14}{}\n",
                    short_bytes(&n.id),
                    kind_word(n.kind),
                    n.actor
                ));
                for line in n.body.lines() {
                    out.push_str(&format!("      {line}\n"));
                }
                for line in extra_lines(&n.extra_json) {
                    out.push_str(&format!("      {line}\n"));
                }
                if let Some(older) = &n.supersedes {
                    out.push_str(&format!("      supersedes {}\n", short_bytes(older)));
                }
            }
        }
    }
    out
}

/// Whether a section was asked for. No `fields` at all means all of them.
fn wants(fields: &[&str], section: &str) -> bool {
    fields.is_empty() || fields.iter().any(|f| f.eq_ignore_ascii_case(section))
}

/// The board, as a shape a program can hold on to.
///
/// Keys and types, never a rendered table. An agent parses this, and a
/// human-formatted table that shifts when a title grows is a parser that breaks
/// on a Tuesday.
fn render_list_json(tasks: &[pb::Task]) -> String {
    let now = now_millis();
    let rows: Vec<serde_json::Value> = tasks.iter().map(|t| task_json(t, now)).collect();
    serde_json::json!({ "tasks": rows }).to_string()
}

fn render_show_json(detail: &pb::TaskDetail) -> String {
    let now = now_millis();
    serde_json::json!({
        "task": detail.task.as_ref().map(|t| task_json(t, now)),
        "notes": detail.notes.iter().map(note_json).collect::<Vec<_>>(),
        "blocks": detail.blocks.iter().map(block_json).collect::<Vec<_>>(),
    })
    .to_string()
}

fn task_json(task: &pb::Task, now: i64) -> serde_json::Value {
    serde_json::json!({
        "id": uuid_of(&task.id).to_string(),
        "short": short_bytes(&task.id),
        "repository_id": uuid_of(&task.repository_id).to_string(),
        "resource_version": task.resource_version,
        "key": task.key,
        "title": task.title,
        "status": status_word(task.status),
        "status_since": task.status_since,
        "stale_for_seconds": stale_for_seconds(task.status_since, now),
        "intent": task.intent,
        "acceptance": task.acceptance.iter().map(|a| serde_json::json!({
            "id": uuid_of(&a.id).to_string(),
            "text": a.text,
            "met": a.met,
        })).collect::<Vec<_>>(),
        "constraints": task.constraints,
        "labels": task.labels,
        "workspace_id": task.workspace_id.as_ref().map(|b| uuid_of(b).to_string()),
    })
}

fn note_json(note: &pb::TaskNote) -> serde_json::Value {
    serde_json::json!({
        "id": uuid_of(&note.id).to_string(),
        "short": short_bytes(&note.id),
        "task_id": uuid_of(&note.task_id).to_string(),
        "kind": kind_word(note.kind),
        "actor": note.actor,
        "at": note.at,
        "body": note.body,
        // Parsed rather than passed through as a string, so a reader does not
        // have to decode JSON a second time to reach a decision's rejected
        // alternatives.
        "extra": parsed_extra(&note.extra_json),
        "supersedes": note.supersedes.as_ref().map(|b| uuid_of(b).to_string()),
    })
}

fn block_json(block: &pb::TaskBlock) -> serde_json::Value {
    serde_json::json!({
        "task_id": uuid_of(&block.task_id).to_string(),
        "blocked_by": uuid_of(&block.blocked_by).to_string(),
        "short": short_bytes(&block.blocked_by),
        "reason": block.reason,
    })
}

fn parsed_extra(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({}))
}

/// A note's structure, as lines under its body.
fn extra_lines(raw: &str) -> Vec<String> {
    let value = parsed_extra(raw);
    let Some(fields) = value.as_object() else { return Vec::new() };
    fields
        .iter()
        .map(|(name, value)| {
            let text = match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(items) => items
                    .iter()
                    .map(|i| match i {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            format!("{name}: {text}")
        })
        .collect()
}

fn first_line(body: &str) -> &str {
    body.lines().next().unwrap_or("")
}

// ---------------------------------------------------------------------------
// Words on the wire, in this CLI's own vocabulary
// ---------------------------------------------------------------------------

fn pb_status(status: TaskStatus) -> i32 {
    (match status {
        TaskStatus::Backlog => pb::TaskStatus::Backlog,
        TaskStatus::Todo => pb::TaskStatus::Todo,
        TaskStatus::NeedsDecision => pb::TaskStatus::NeedsDecision,
        TaskStatus::InProgress => pb::TaskStatus::InProgress,
        TaskStatus::InReview => pb::TaskStatus::InReview,
        TaskStatus::Done => pb::TaskStatus::Done,
        TaskStatus::Cancelled => pb::TaskStatus::Cancelled,
    }) as i32
}

/// The status a number means, or `None` for one this build has no name for.
///
/// Never defaulted. A runner newer than this CLI naming a status it has not
/// heard of must not have it read as the backlog, which is how a finished task
/// gets printed as unstarted.
fn status_of(raw: i32) -> Option<TaskStatus> {
    match pb::TaskStatus::try_from(raw).ok()? {
        pb::TaskStatus::Unspecified => None,
        pb::TaskStatus::Backlog => Some(TaskStatus::Backlog),
        pb::TaskStatus::Todo => Some(TaskStatus::Todo),
        pb::TaskStatus::NeedsDecision => Some(TaskStatus::NeedsDecision),
        pb::TaskStatus::InProgress => Some(TaskStatus::InProgress),
        pb::TaskStatus::InReview => Some(TaskStatus::InReview),
        pb::TaskStatus::Done => Some(TaskStatus::Done),
        pb::TaskStatus::Cancelled => Some(TaskStatus::Cancelled),
    }
}

fn status_word(raw: i32) -> &'static str {
    status_of(raw).map(TaskStatus::as_str).unwrap_or("unknown")
}

fn pb_note_kind(kind: NoteKind) -> i32 {
    (match kind {
        NoteKind::Decision => pb::TaskNoteKind::Decision,
        NoteKind::Finding => pb::TaskNoteKind::Finding,
        NoteKind::Question => pb::TaskNoteKind::Question,
        NoteKind::Answer => pb::TaskNoteKind::Answer,
        NoteKind::Progress => pb::TaskNoteKind::Progress,
        NoteKind::Comment => pb::TaskNoteKind::Comment,
        NoteKind::StatusChange => pb::TaskNoteKind::StatusChange,
        NoteKind::Created => pb::TaskNoteKind::Created,
    }) as i32
}

fn note_kind_of(raw: i32) -> Option<NoteKind> {
    match pb::TaskNoteKind::try_from(raw).ok()? {
        pb::TaskNoteKind::Unspecified => None,
        pb::TaskNoteKind::Decision => Some(NoteKind::Decision),
        pb::TaskNoteKind::Finding => Some(NoteKind::Finding),
        pb::TaskNoteKind::Question => Some(NoteKind::Question),
        pb::TaskNoteKind::Answer => Some(NoteKind::Answer),
        pb::TaskNoteKind::Progress => Some(NoteKind::Progress),
        pb::TaskNoteKind::Comment => Some(NoteKind::Comment),
        pb::TaskNoteKind::StatusChange => Some(NoteKind::StatusChange),
        pb::TaskNoteKind::Created => Some(NoteKind::Created),
    }
}

fn kind_word(raw: i32) -> &'static str {
    note_kind_of(raw).map(NoteKind::as_str).unwrap_or("unknown")
}

/// A status a person typed.
fn status_named(word: &str) -> Result<TaskStatus, String> {
    TaskStatus::parse(word.trim()).ok_or_else(|| {
        format!(
            "{word:?} is not a status. use backlog, todo, needs_decision, \
             in_progress, in_review, done or cancelled"
        )
    })
}

/// A note kind a person typed, for reading.
fn kind_named(word: &str) -> Result<NoteKind, String> {
    NoteKind::parse(word.trim()).ok_or_else(|| {
        format!(
            "{word:?} is not a kind of note. use decision, finding, question, \
             answer, progress, comment, status_change or created"
        )
    })
}

/// A note kind a person may WRITE.
///
/// `status_change` and `created` are written by the transactions that move or
/// make a task, and the runner refuses both. Refused here too, with the sentence
/// that says what to do instead — a round trip to be told no teaches nothing.
fn writable_kind(word: &str) -> Result<NoteKind, String> {
    match kind_named(word)? {
        NoteKind::StatusChange => {
            Err("a move is recorded by `farcooler task set --status`, never written by hand"
                .to_string())
        }
        NoteKind::Created => {
            Err("a task's first entry is written when it is created, never by hand".to_string())
        }
        kind => Ok(kind),
    }
}

/// `--fields` and `--json` are answering different questions, so asking both is
/// refused rather than one being dropped.
///
/// `--fields` chooses what a PERSON reads. `--json` answers with the whole task
/// because a parser needs one shape whatever the invocation was — a key that
/// comes and goes with a flag is the instability the JSON exists to avoid.
/// `--notes` is not like this and works with both: it is a narrowing the runner
/// itself performs, so it changes what is fetched rather than what is drawn.
///
/// Refused before the first round trip, so a mistake costs nothing.
fn one_question_at_a_time(json: bool, fields: Option<&str>) -> Result<(), String> {
    if json && fields.is_some() {
        return Err("--fields chooses what a person reads. --json answers with the whole task, \
                    so a parser gets one shape every time. use --notes to narrow the history \
                    in either"
            .to_string());
    }
    Ok(())
}

/// Which sections `show` was asked for, refusing a name it does not have.
///
/// A typo that printed nothing would look exactly like a task with nothing on
/// it, which is the one answer this must never give by accident.
fn asked_fields(raw: Option<&str>) -> Result<Vec<String>, String> {
    let Some(raw) = raw else { return Ok(Vec::new()) };
    let mut asked = Vec::new();
    for word in raw.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let known = SECTIONS.iter().find(|s| s.eq_ignore_ascii_case(word));
        match known {
            Some(section) => asked.push((*section).to_string()),
            None => {
                return Err(format!(
                    "{word:?} is not a section. use {}",
                    SECTIONS.join(", ")
                ));
            }
        }
    }
    Ok(asked)
}

// ---------------------------------------------------------------------------
// Who is asking, and about what
// ---------------------------------------------------------------------------

/// The task a command is about: the one named, or the pane's own.
///
/// `FARCOOLER_TASK` is the CONTRACT with whatever dispatches a pane, and as of
/// this commit **nothing in this tree sets it** — no `Command::env`, no tmux
/// export, nothing. Read that as a promise this side keeps and the other side
/// does not yet: dispatch must export it, and until it does an agent names its
/// own key like anybody else. It is worth having anyway, because an agent that
/// has to be told its own ticket twice will eventually be told the wrong one.
///
/// An explicit key always wins: a pane may legitimately talk about another
/// task, and a command that ignored its own argument would write to the wrong
/// ticket in silence.
fn resolve_key(given: Option<String>, from_env: Option<String>) -> Option<String> {
    fn clean(raw: String) -> Option<String> {
        let trimmed = raw.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }
    given.and_then(clean).or_else(|| from_env.and_then(clean))
}

fn wanted_key(given: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    resolve_key(given, std::env::var(TASK_ENV).ok()).ok_or_else(|| {
        format!("name a task, or run this where {TASK_ENV} names one").into()
    })
}

/// Who this invocation says it is.
///
/// The trap this closes: proto3 cannot tell an omitted `actor` from an empty
/// one, and the runner reads empty as `user`. An agent that forgets to name
/// itself files its work under a person, and nothing in the record ever says
/// otherwise. So every write from this CLI names an actor out loud — `user`
/// included — and the runner's default is never the thing that decides.
///
/// A word that does not parse is refused HERE, before the round trip, so a
/// mistyped terminal id is a sentence at the terminal rather than a refusal
/// from a daemon. `Actor::parse` is the store's own reader, paired with the
/// `Display` that writes the column; there is no second vocabulary here.
fn actor_from(given: Option<&str>) -> Result<Actor, String> {
    let Some(word) = given else { return Ok(Actor::User) };
    let word = word.trim();
    Actor::parse(word).ok_or_else(|| {
        format!("{word:?} is not an actor. use user, manager, or agent:<terminal id>")
    })
}

fn actor_for(given: Option<&str>) -> Result<Actor, String> {
    match given {
        Some(word) => actor_from(Some(word)),
        None => {
            let from_env = std::env::var(ACTOR_ENV).ok();
            let named = from_env.as_deref().map(str::trim).filter(|w| !w.is_empty());
            actor_from(named)
        }
    }
}

// ---------------------------------------------------------------------------
// Reaching the board
// ---------------------------------------------------------------------------

/// Which repository a command works in.
///
/// `--repo` first. Then the only repository, when there is only one. Then the
/// repository the pane's own task lives in, since keys are per repository and
/// `FARCOOLER_TASK` therefore names one. A runner with several repositories and
/// no hint at all is asked rather than guessed at: writing a task onto the
/// wrong board is not something a person would notice for days.
async fn repository_for(
    link: &mut Link,
    given: Option<&str>,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let repositories = list_repositories(link).await?;
    if let Some(name) = given {
        return Ok(uuid_of(&resolve_repository(&repositories, name)?.id));
    }
    match repositories.len() {
        0 => Err("no repository is registered here. add one with `farcooler repo register`"
            .into()),
        1 => Ok(uuid_of(&repositories[0].id)),
        _ => {
            if let Some(key) = std::env::var(TASK_ENV).ok().filter(|k| !k.trim().is_empty()) {
                let found = find_task(link, None, &key).await?;
                return Ok(uuid_of(&found.repository_id));
            }
            let names: Vec<&str> =
                repositories.iter().map(|r| r.display_name.as_str()).collect();
            Err(format!(
                "this runner has {} repositories. name one with --repo: {}",
                names.len(),
                names.join(", ")
            )
            .into())
        }
    }
}

async fn tasks_in(
    link: &mut Link,
    repository: Uuid,
    status: Option<TaskStatus>,
    stale_after: Option<Duration>,
) -> Result<Vec<pb::Task>, Box<dyn std::error::Error>> {
    let r = link
        .call(with(
            req_for("task.list", repository),
            request::Payload::TaskList(pb::TaskListRequest {
                repository_id: id_bytes(repository),
                status: status.map(pb_status).unwrap_or_default(),
                stale_after_millis: stale_after.map(|d| d.as_millis() as u64),
            }),
        ))
        .await
        .map_err(|e| refused(e, "that board could not be read"))?;
    let result::Value::TaskList(l) = expect_value(r.value, "task_list")? else {
        return Err("the daemon returned the wrong resource".into());
    };
    Ok(l.items)
}

async fn detail_of(
    link: &mut Link,
    task: Uuid,
    notes: Option<NoteKind>,
) -> Result<pb::TaskDetail, Box<dyn std::error::Error>> {
    let r = link
        .call(with(
            req_for("task.get", task),
            request::Payload::TaskGet(pb::TaskGetRequest {
                task_id: id_bytes(task),
                note_kind: notes.map(pb_note_kind).unwrap_or_default(),
            }),
        ))
        .await
        .map_err(|e| refused(e, "that task could not be read"))?;
    let result::Value::TaskDetail(d) = expect_value(r.value, "task_detail")? else {
        return Err("the daemon returned the wrong resource".into());
    };
    Ok(d)
}

/// Append one entry to a task's record.
///
/// The only write in this file that touches a note, and it only ever adds one.
/// `supersedes` names an earlier entry and changes nothing about it.
async fn append_note(
    link: &mut Link,
    task: Uuid,
    kind: NoteKind,
    body: &str,
    extra: &str,
    supersedes: Option<Uuid>,
    actor: Actor,
) -> Result<pb::TaskNote, Box<dyn std::error::Error>> {
    let r = link
        .call(with(
            req_for("task.note", task),
            request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
                task_id: id_bytes(task),
                kind: pb_note_kind(kind),
                body: body.to_string(),
                extra_json: extra.to_string(),
                supersedes: supersedes.map(id_bytes),
                actor: actor.to_string(),
            }),
        ))
        .await
        .map_err(|e| refused(e, "an entry needs something written in it"))?;
    let result::Value::TaskNote(note) = expect_value(r.value, "task_note")? else {
        return Err("the daemon returned the wrong resource".into());
    };
    Ok(note)
}

/// One task, by the key a person types or by the short id `show` prints.
///
/// Keys are per repository, so two boards can both have an `fc-1`. Without
/// `--repo` every registered board is asked and an ambiguous answer is refused
/// rather than picked from — writing to the wrong board is the failure that
/// would be found days later.
async fn find_task(
    link: &mut Link,
    repo: Option<&str>,
    needle: &str,
) -> Result<pb::Task, Box<dyn std::error::Error>> {
    let repositories = list_repositories(link).await?;
    let searched: Vec<Uuid> = match repo {
        Some(name) => vec![uuid_of(&resolve_repository(&repositories, name)?.id)],
        None => repositories.iter().map(|r| uuid_of(&r.id)).collect(),
    };
    let wanted = needle.trim().to_lowercase();

    let mut found: Vec<pb::Task> = Vec::new();
    for repository in searched {
        for task in tasks_in(link, repository, None, None).await? {
            if task.key.to_lowercase() == wanted || short_bytes(&task.id) == wanted {
                found.push(task);
            }
        }
    }
    match found.len() {
        1 => Ok(found.swap_remove(0)),
        0 => Err(format!("no task here is called {needle:?}").into()),
        n => Err(format!(
            "{needle:?} names a task on {n} boards. say which with --repo"
        )
        .into()),
    }
}

/// One note of a task's record, by the short id `show` prints.
///
/// The kind has to match. A note correcting an entry of another kind reads as a
/// correction and is not one, and the runner refuses it — so it is refused here
/// first, where the sentence can say which entry was meant.
fn find_note(
    notes: &[pb::TaskNote],
    short: &str,
    kind: NoteKind,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let wanted = short.trim().to_lowercase().replace('-', "");
    let matches: Vec<&pb::TaskNote> = notes
        .iter()
        .filter(|n| uuid_of(&n.id).simple().to_string().ends_with(&wanted))
        .collect();
    match matches.len() {
        1 => {
            let note = matches[0];
            if note_kind_of(note.kind) != Some(kind) {
                return Err(format!(
                    "that entry is a {}, and an entry may only be superseded by one of its own \
                     kind",
                    kind_word(note.kind)
                )
                .into());
            }
            Ok(uuid_of(&note.id))
        }
        0 => Err(format!("this task's record has no entry {short:?}").into()),
        n => Err(format!("{short:?} matches {n} entries, be more specific").into()),
    }
}

fn key_index(tasks: &[pb::Task]) -> std::collections::HashMap<Uuid, String> {
    tasks.iter().map(|t| (uuid_of(&t.id), t.key.clone())).collect()
}

fn new_acceptance(text: &str) -> pb::TaskAcceptanceItem {
    // An empty id is a NEW line, and the runner mints one. A client adding a
    // line does not have to invent a uuid to do it.
    pb::TaskAcceptanceItem { id: bytes::Bytes::new(), text: text.to_string(), met: false }
}

/// A refusal from the runner, in this CLI's own words.
///
/// The runner sends a stable machine word and a message written for its own
/// log — "invalid argument: title", "resource version is stale". That text is
/// exactly what `farcooler_core::error::word` exists to keep off a screen, so
/// none of it is printed here. `invalid` is the sentence this particular call
/// owes its reader when the word says a field was wrong, because the word says
/// only that one was.
fn refused(err: ClientError, invalid: &str) -> Box<dyn std::error::Error> {
    let code = match err {
        ClientError::Daemon { code, .. } => code,
        // `Codec` is transparent over `CodecError`, which is transparent over
        // `std::io::Error`, so left alone this arm prints "Broken pipe (os
        // error 32)" — an operating system's words, on a screen, about a
        // product it has never heard of. To whoever typed the command a broken
        // pipe, a closed socket and a garbled frame are one fact, so they get
        // one sentence.
        ClientError::VersionMismatch { .. } => {
            return "this Far Cooler and the runner's speak different protocols. update both"
                .into();
        }
        ClientError::EmptyResult | ClientError::WrongResult { .. } => {
            return "the runner answered with something this Far Cooler cannot read".into();
        }
        _ => return "the runner stopped answering".into(),
    };
    let word = farcooler_core::error::word_for(code);
    let said: String = match word {
        "not-found" => "that task is not on this runner".to_string(),
        "invalid-argument" => invalid.to_string(),
        "resource-conflict" => {
            "this task changed while you were reading it. read it again and reapply the change"
                .to_string()
        }
        "scope-denied" => "this client may read the board but not write to it".to_string(),
        "capability-unsupported" => {
            "this runner's Far Cooler is older than the board. update it and try again".to_string()
        }
        "auth-required" => "this client is not paired with the runner".to_string(),
        // The machine word rather than the runner's prose: it is a stable,
        // client-facing vocabulary, and it is what a bug report needs.
        other => format!("the runner refused that ({other})"),
    };
    said.into()
}

// ---------------------------------------------------------------------------
// Time, as a person writes it and as a person reads it
// ---------------------------------------------------------------------------

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// How long a task has sat where it is. Never negative: a runner whose clock is
/// a little ahead of this one has not moved a task in the future.
fn stale_for_seconds(status_since: i64, now: i64) -> i64 {
    now.saturating_sub(status_since).max(0) / 1000
}

/// A length of time written the way a person types it: `45s`, `10m`, `2h`, `3d`.
fn parse_gap(raw: &str) -> Result<Duration, String> {
    let text = raw.trim();
    let bad = || {
        format!("{raw:?} is not a length of time. write it as 45s, 10m, 2h or 3d")
    };
    let unit = text.chars().next_back().ok_or_else(&bad)?;
    let seconds: u64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return Err(bad()),
    };
    let count: u64 = text[..text.len() - unit.len_utf8()].parse().map_err(|_| bad())?;
    Ok(Duration::from_secs(count.checked_mul(seconds).ok_or_else(bad)?))
}

/// A gap, short enough for a column.
fn spoken_gap(seconds: i64) -> String {
    match seconds {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task with a real record on it: a decision, a decision that replaced
    /// it, and the progress chatter both of those are buried in.
    ///
    /// The progress note carries the word "progress" as its KIND and "building"
    /// in its body, and both are what the negative assertions below are for. A
    /// fixture whose history happens not to contain the words would make those
    /// assertions unfailable — they would pass against a renderer that printed
    /// every note every time.
    fn a_task_with_history() -> pb::TaskDetail {
        let task = pb::Task {
            id: id_bytes(Uuid::now_v7()),
            resource_version: 4,
            repository_id: id_bytes(Uuid::now_v7()),
            key: "fc-42".to_string(),
            title: "the board an agent can read".to_string(),
            // Deliberately NOT `in_progress`: that word would put "progress"
            // into a full render for a reason that has nothing to do with the
            // history, and the assertion below is about the history.
            status: pb_status(TaskStatus::InReview),
            status_since: now_millis() - 7_200_000,
            intent: "an agent should reach the board without a person".to_string(),
            acceptance: vec![pb::TaskAcceptanceItem {
                id: id_bytes(Uuid::now_v7()),
                text: "the suite is green".to_string(),
                met: false,
            }],
            constraints: vec!["additive migrations only".to_string()],
            labels: vec!["board".to_string()],
            workspace_id: None,
        };
        let task_id = task.id.clone();
        let note = |kind: NoteKind, body: &str, extra: &str| pb::TaskNote {
            id: id_bytes(Uuid::now_v7()),
            task_id: task_id.clone(),
            kind: pb_note_kind(kind),
            actor: "manager".to_string(),
            at: now_millis(),
            body: body.to_string(),
            extra_json: extra.to_string(),
            supersedes: None,
        };
        let notes = vec![
            note(NoteKind::Decision, "we will use sqlite", r#"{"rejected":["postgres"]}"#),
            note(NoteKind::Progress, "building the store layer", ""),
            note(NoteKind::Finding, "the trigger refuses an update", ""),
        ];
        pb::TaskDetail { task: Some(task), notes, blocks: Vec::new() }
    }

    /// A manager surveying thirty tasks must be able to ask for three fields.
    /// Pulling every history on every wake-up is how the loop gets slow and
    /// expensive, and it gets slower as the board gets more useful.
    #[test]
    fn show_returns_only_the_fields_that_were_asked_for() {
        let rendered = render_show(&a_task_with_history(), &["intent", "acceptance"], None);
        assert!(rendered.contains("intent"));
        assert!(rendered.contains("acceptance"));
        assert!(!rendered.contains("progress"), "history was not asked for");
        // And the fixture really would have said it, so the line above is
        // doing work rather than passing on an accident of the data.
        let whole = render_show(&a_task_with_history(), &[], None);
        assert!(whole.contains("progress"), "the fixture carries a progress note");
    }

    #[test]
    fn decisions_can_be_read_without_the_progress_chatter() {
        let rendered = render_show(&a_task_with_history(), &[], Some(NoteKind::Decision));
        assert!(rendered.contains("use sqlite"));
        assert!(!rendered.contains("building"), "progress notes are the noise this skips");
        // The other kinds go too, not just the loud one.
        assert!(!rendered.contains("the trigger refuses"));
    }

    /// Agents parse this. A human-formatted table that shifts when a title
    /// grows is a parser that breaks on a Tuesday.
    #[test]
    fn json_output_is_a_stable_shape_and_not_a_rendered_table() {
        let row = a_task_with_history().task.expect("the fixture's row");
        let out = render_list_json(&[row]);
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        // Parsed structure, never a substring of the serialization: a rendered
        // table containing the word "key" would satisfy a substring check.
        assert!(parsed["tasks"][0]["key"].is_string());
        assert!(parsed["tasks"][0]["status"].is_string());
        assert!(parsed["tasks"][0]["stale_for_seconds"].is_number());
        assert_eq!(parsed["tasks"][0]["key"], "fc-42");
        assert_eq!(parsed["tasks"][0]["status"], "in_review");
        // Two hours of sitting still, to the second, so a renderer that
        // printed a spoken gap here instead of a number fails.
        let sat = parsed["tasks"][0]["stale_for_seconds"].as_i64().expect("a number");
        assert!((7_195..=7_205).contains(&sat), "sat for {sat}s");
        assert!(parsed["tasks"][0]["workspace_id"].is_null(), "a task with no lane says so");
    }

    /// `FARCOOLER_TASK` is what a dispatched pane is MEANT to carry — nothing
    /// in this tree sets it yet, so this pins the reading side of a contract
    /// the writing side still owes. An agent that has to be told its own ticket
    /// twice will eventually be told the wrong one.
    #[test]
    fn a_task_argument_defaults_to_the_panes_own_task() {
        assert_eq!(resolve_key(None, Some("fc-42".to_string())), Some("fc-42".to_string()));
        assert_eq!(
            resolve_key(Some("fc-9".to_string()), Some("fc-42".to_string())),
            Some("fc-9".to_string()),
            "an explicit key always wins over the environment"
        );
        assert_eq!(resolve_key(None, None), None);
        // An exported-but-empty variable is not a ticket. A shell that does
        // `export FARCOOLER_TASK=` would otherwise send an empty key.
        assert_eq!(resolve_key(None, Some("   ".to_string())), None);
    }

    /// Every command and flag under `task`, to full depth, as a person would
    /// type it: `("task note", Some("supersedes"))`, `("task note", None)`.
    ///
    /// Recursive on purpose. A walk that stopped at the first level would say
    /// nothing at all about a nested `task note edit`, and the guard below
    /// would pass by never having looked — which is worse than no guard,
    /// because the next author reads a green suite as permission.
    #[cfg(test)]
    fn every_command_and_flag(
        command: &clap::Command,
        path: &str,
    ) -> Vec<(String, Option<String>)> {
        let mut found = Vec::new();
        for sub in command.get_subcommands() {
            let here = format!("{path} {}", sub.get_name());
            found.push((here.clone(), None));
            for arg in sub.get_arguments() {
                if let Some(flag) = arg.get_long() {
                    found.push((here.clone(), Some(flag.to_string())));
                }
            }
            found.extend(every_command_and_flag(sub, &here));
        }
        found
    }

    /// The walk really does go all the way down.
    ///
    /// Guarded directly, on a tree built for the purpose, because `task` has no
    /// nested command today: asserting recursion only through the real tree
    /// would leave a walk quietly flattened to one level green until somebody
    /// added one — which is the exact moment the guard below is most needed and
    /// the exact moment nobody is looking at it.
    #[test]
    fn the_walk_goes_all_the_way_down() {
        let probe = clap::Command::new("probe").subcommand(
            clap::Command::new("outer").subcommand(
                clap::Command::new("inner").arg(clap::Arg::new("deep").long("deep")),
            ),
        );
        let surface = every_command_and_flag(&probe, "probe");
        assert!(surface.contains(&("probe outer".to_string(), None)), "{surface:?}");
        assert!(surface.contains(&("probe outer inner".to_string(), None)), "{surface:?}");
        assert!(
            surface.contains(&("probe outer inner".to_string(), Some("deep".to_string()))),
            "a flag on a subcommand of a subcommand was never visited: {surface:?}"
        );
    }

    /// The one write in this surface that touches the record only ever ADDS.
    ///
    /// `task_notes` carries a `BEFORE UPDATE` trigger that refuses
    /// unconditionally, so a flag here that edited or deleted an entry would
    /// compile, ship, and fail at runtime in front of whoever typed it. This
    /// walks the parsed command tree rather than the source, TO FULL DEPTH, so
    /// a flag added anywhere under `task` is caught wherever it was written —
    /// including on a subcommand of a subcommand, which is the shape a
    /// one-level walk would wave straight through.
    #[test]
    fn nothing_here_can_edit_or_delete_an_entry_in_the_record() {
        let command = TaskCmd::augment_subcommands(clap::Command::new("task"));
        let surface = every_command_and_flag(&command, "task");
        // A walk that visited nothing would satisfy every assertion below. This
        // is the proof it went somewhere and reached a flag, not just a name.
        assert!(
            surface.contains(&("task note".to_string(), Some("supersedes".to_string()))),
            "the walk did not reach `task note --supersedes`"
        );

        let forbidden = ["edit", "amend", "delete", "remove", "rewrite", "redact"];
        for (name, flag) in &surface {
            let typed = match flag {
                Some(flag) => format!("{name} --{flag}"),
                None => name.clone(),
            };
            for word in forbidden {
                let says = flag.as_deref().unwrap_or(name);
                assert!(!says.contains(word), "`{typed}` can change the record");
            }
        }

        // And the shape that must remain: correcting the record is a NEW entry
        // pointing at the old one.
        let note = command.get_subcommands().find(|s| s.get_name() == "note").expect("note");
        let mut flags: Vec<&str> = note
            .get_arguments()
            .filter_map(|a| a.get_long())
            .filter(|f| *f != "help")
            .collect();
        flags.sort_unstable();
        assert_eq!(flags, ["actor", "body", "kind", "rejected", "repo", "supersedes"]);
    }

    /// Two tables mapped by hand in two directions, which is exactly where an
    /// arm gets copied onto the wrong neighbor.
    #[test]
    fn every_status_and_kind_survives_the_round_trip() {
        for status in [
            TaskStatus::Backlog,
            TaskStatus::Todo,
            TaskStatus::NeedsDecision,
            TaskStatus::InProgress,
            TaskStatus::InReview,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(status_of(pb_status(status)), Some(status), "{status:?}");
            assert_eq!(status_word(pb_status(status)), status.as_str());
            assert_eq!(status_named(status.as_str()), Ok(status));
        }
        for kind in [
            NoteKind::Decision,
            NoteKind::Finding,
            NoteKind::Question,
            NoteKind::Answer,
            NoteKind::Progress,
            NoteKind::Comment,
            NoteKind::StatusChange,
            NoteKind::Created,
        ] {
            assert_eq!(note_kind_of(pb_note_kind(kind)), Some(kind), "{kind:?}");
            assert_eq!(kind_word(pb_note_kind(kind)), kind.as_str());
        }
        // A number no version of this build defines. Never defaulted: a runner
        // newer than this CLI naming a status it has not heard of must not have
        // a finished task printed as unstarted.
        assert_eq!(status_of(9_999), None);
        assert_eq!(status_word(9_999), "unknown");
        assert_eq!(note_kind_of(9_999), None);
        // The two the runner writes and refuses to be handed.
        assert!(writable_kind("status_change").is_err());
        assert!(writable_kind("created").is_err());
        assert_eq!(writable_kind("decision"), Ok(NoteKind::Decision));
    }

    /// An agent that forgets to name itself must not file its work under a
    /// person, and a typo in a terminal id must not either.
    #[test]
    fn a_write_names_who_made_it_and_a_typo_is_refused() {
        // Nobody named is a person at a client, which is what `user` means —
        // and it is SENT, rather than left for the runner's empty-string
        // default to decide.
        assert_eq!(actor_from(None), Ok(Actor::User));
        assert_eq!(Actor::User.to_string(), "user");
        assert_eq!(actor_from(Some("manager")), Ok(Actor::Manager));
        let terminal = Uuid::now_v7();
        assert_eq!(
            actor_from(Some(&format!("agent:{terminal}"))),
            Ok(Actor::Agent { terminal })
        );
        for raw in ["agent:not-a-uuid", "agent:", "AGENT", "robot", ""] {
            assert!(actor_from(Some(raw)).is_err(), "{raw:?} was accepted");
        }
    }

    #[test]
    fn a_length_of_time_reads_the_way_a_person_writes_it() {
        assert_eq!(parse_gap("45s"), Ok(Duration::from_secs(45)));
        assert_eq!(parse_gap("10m"), Ok(Duration::from_secs(600)));
        assert_eq!(parse_gap(" 2h "), Ok(Duration::from_secs(7_200)));
        assert_eq!(parse_gap("3d"), Ok(Duration::from_secs(259_200)));
        for raw in ["", "2", "2w", "d", "-1d", "two days", "9999999999999999999d"] {
            assert!(parse_gap(raw).is_err(), "{raw:?} was accepted");
        }
    }

    /// A section name nobody has is refused, rather than printing nothing.
    ///
    /// A typo that printed an empty card looks exactly like a task with nothing
    /// on it, which is the one answer this must never give by accident.
    #[test]
    fn an_unknown_section_is_refused_rather_than_printed_as_emptiness() {
        assert_eq!(asked_fields(None), Ok(Vec::new()));
        assert_eq!(
            asked_fields(Some("intent, acceptance")),
            Ok(vec!["intent".to_string(), "acceptance".to_string()])
        );
        assert!(asked_fields(Some("intent,notes")).is_err());
        assert!(asked_fields(Some("history")).is_ok());
    }

    /// The record is read by short id, and only an entry of the same kind may
    /// be superseded — a note correcting an entry of another kind reads as a
    /// correction and is not one.
    #[test]
    fn an_entry_is_superseded_by_one_of_its_own_kind() {
        let detail = a_task_with_history();
        let decision = &detail.notes[0];
        let short = short_bytes(&decision.id);
        assert_eq!(
            find_note(&detail.notes, &short, NoteKind::Decision).expect("found"),
            uuid_of(&decision.id)
        );
        assert!(find_note(&detail.notes, &short, NoteKind::Finding).is_err());
        assert!(find_note(&detail.notes, "deadbeef", NoteKind::Decision).is_err());
    }

    /// A flag that was typed decides something.
    ///
    /// `--notes decision` alongside `--fields intent` used to be accepted and
    /// then dropped on the floor: the history section was not in `--fields`, so
    /// the narrowing had nothing to narrow and the user was told nothing. The
    /// flag now brings its own section with it. (The other half of this — that
    /// `--json` with `--fields` is REFUSED rather than silently answered in
    /// full — is a check in the `Show` arm, above the first round trip.)
    #[test]
    fn asking_for_one_kind_of_note_is_asking_for_the_history() {
        let rendered = render_show(&a_task_with_history(), &["intent"], Some(NoteKind::Decision));
        assert!(rendered.contains("intent"), "the section that was named");
        assert!(rendered.contains("use sqlite"), "and the notes that were asked for");
        // Still a narrowing, not a floodgate.
        assert!(!rendered.contains("building"), "progress notes were not asked for");
        assert!(!rendered.contains("acceptance"), "nor was anything else");
    }

    /// An operating system's words never reach a screen.
    ///
    /// A socket that dies mid-request surfaces as `ClientError::Codec`, which
    /// is transparent over `CodecError`, which is transparent over
    /// `std::io::Error` — so the untreated string is "Broken pipe (os error
    /// 32)", which says nothing to whoever typed the command and names a
    /// product it has never heard of. A stable machine word crosses the wire
    /// and this CLI owns every sentence.
    #[test]
    fn a_broken_socket_is_a_sentence_and_never_an_os_error() {
        let broken = ClientError::Codec(farcooler_transport::CodecError::Io(
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe (os error 32)"),
        ));
        let said = refused(broken, "unused").to_string();
        assert!(!said.contains("os error"), "an OS error reached a screen: {said}");
        assert!(!said.contains("Broken pipe"), "an OS error reached a screen: {said}");
        assert_eq!(said, "the runner stopped answering");

        // The daemon's own refusals still say their own specific thing, and
        // still never carry its log prose.
        let denied = ClientError::Daemon {
            code: pb::ErrorCode::ScopeDenied as i32,
            retryable: false,
            message: "scope denied: control".to_string(),
        };
        let said = refused(denied, "unused").to_string();
        assert_eq!(said, "this client may read the board but not write to it");
        assert!(
            !said.contains("scope denied: control"),
            "the runner's log prose reached a screen"
        );
    }

    /// The other half of "a flag that was typed decides something".
    ///
    /// `show --json --fields intent` used to accept both and answer with the
    /// whole task, dropping `--fields` in silence. It is refused by name now,
    /// before any round trip. Each flag alone is still fine, and `--notes`
    /// works with either.
    #[test]
    fn asking_a_person_and_a_parser_the_same_question_is_refused() {
        assert!(one_question_at_a_time(true, Some("intent")).is_err());
        assert!(one_question_at_a_time(true, None).is_ok());
        assert!(one_question_at_a_time(false, Some("intent")).is_ok());
        assert!(one_question_at_a_time(false, None).is_ok());
    }

    /// A clock that runs a little ahead has not moved a task in the future.
    #[test]
    fn a_gap_is_never_negative() {
        assert_eq!(stale_for_seconds(1_000, 0), 0);
        assert_eq!(stale_for_seconds(0, 7_200_000), 7_200);
        assert_eq!(spoken_gap(0), "just now");
        assert_eq!(spoken_gap(7_200), "2h");
        assert_eq!(spoken_gap(259_200), "3d");
    }
}

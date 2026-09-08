//! The board: a task's current understanding, and the record of how it got
//! there.
//!
//! The split is the whole design. A task row holds what is understood NOW --
//! intent, acceptance, constraints, status -- and every field of it may be
//! revised freely. Everything about how that understanding was reached is a
//! note, and no note is ever edited: correcting the record is a new note
//! carrying `supersedes`. There is no function in this module that changes a
//! note, `task_notes` carries triggers that refuse one, and the reason is that
//! an agent revising a description to reflect what it has learned overwrites
//! the reasoning, and the reasoning is the part you want in three weeks.
//!
//! The other half of this module is the key people and agents actually use:
//! `fc-42`. A repository's task key prefix is computed once, at registration,
//! from its name -- and then stored, never recomputed. See `derive_prefix` and
//! `Store::assign_task_key_prefix` for why: a prefix that answered to the
//! CURRENT name would change under a rename, and every key ever written into
//! a note, spoken aloud, or handed to an agent in its opening prompt would
//! stop resolving. Keys are the one identifier in this system that leaves the
//! database.

use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use serde_json::json;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{
    Actor, NoteHit, NoteKind, Task, TaskBlock, TaskNote, TaskStatus, TaskUpdate, acceptance_to_json,
    get_uuid, row_to_note_hit, row_to_task, row_to_task_block, row_to_task_note, strings_to_json,
    uuid_blob,
};
use crate::store::Store;

/// A repository's task key prefix, from its name.
///
/// Initials for a multi-word name, the first two letters otherwise. Short
/// because a person types these and says them out loud.
///
/// Computed ONCE, when a repository is registered, and stored. See
/// `renaming_a_repository_does_not_change_its_task_keys` in this module's
/// tests for why this must never become a function of the current name.
///
/// Word-splitting is `is_ascii_alphanumeric`, so a name with no ASCII
/// letters or digits in it -- a name written entirely in a non-Latin script,
/// for instance -- collapses to the `"t"` fallback below, same as `"---"`
/// does. Not a correctness bug: `assign_task_key_prefix`'s collision
/// resolution handles two repositories landing on the same prefix regardless
/// of why they did, including two differently-named repositories that both
/// fall back to `"t"`. But it does mean prefixes for such names carry no
/// resemblance to the name they came from, worth knowing going in rather
/// than discovering it from a support ticket.
pub fn derive_prefix(repository_name: &str) -> String {
    let words: Vec<&str> = repository_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let prefix: String = if words.len() > 1 {
        words.iter().filter_map(|w| w.chars().next()).collect()
    } else {
        words.first().map(|w| w.chars().take(2).collect()).unwrap_or_default()
    };

    let prefix = prefix.to_ascii_lowercase();
    // A repository named `---` still gets keys. `t` for task, which is what
    // this is a key for, rather than a random string nobody can predict.
    if prefix.is_empty() { "t".to_string() } else { prefix }
}

/// True for any SQLite constraint failure (rusqlite's primary result code
/// does not distinguish UNIQUE from CHECK, NOT NULL, or a foreign key).
/// `assign_task_key_prefix` is the only caller, and the one constraint its
/// UPDATE can ever hit is `repositories_one_task_prefix`, so here this
/// specifically means "another repository already holds this prefix" --
/// retry with a different candidate rather than propagate.
fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(err, rusqlite::Error::SqliteFailure(e, _) if e.code == ErrorCode::ConstraintViolation)
}

impl Store {
    /// Assigns and stores this repository's task key prefix, derived once
    /// from its name at the moment this is called.
    ///
    /// A collision with a prefix already claimed by another repository on
    /// this runner is resolved by appending a digit and trying again. The
    /// database is the referee: `repositories_one_task_prefix` (the partial
    /// unique index over non-empty prefixes) is what actually notices a
    /// collision, not a count this function takes on faith, so two
    /// registrations racing each other still cannot both win the same
    /// prefix.
    ///
    /// Bumps `resource_version` in the same `UPDATE`, so a watcher or an RPC
    /// layer that treats an unchanged version as "nothing to refresh" notices
    /// this write too.
    /// There is no `expected_version` parameter here to check against --
    /// unlike this crate's versioned mutations, this one is not a client
    /// request replaying a version it read; it is called exactly once, from
    /// inside repository registration, against a row nothing else has had a
    /// chance to see yet.
    pub fn assign_task_key_prefix(&self, repo: Uuid) -> Result<String> {
        let name = self.get_repository(repo)?.display_name;
        let base = derive_prefix(&name);

        let mut candidate = base.clone();
        let mut attempt = 1u32;
        loop {
            let outcome = self.conn().execute(
                "UPDATE repositories SET task_key_prefix = ?1, resource_version = resource_version + 1
                 WHERE id = ?2",
                params![candidate, uuid_blob(repo)],
            );
            match outcome {
                Ok(_) => return Ok(candidate),
                Err(e) if is_unique_violation(&e) => {
                    attempt += 1;
                    candidate = format!("{base}{attempt}");
                }
                Err(e) => return Err(map_err(e)),
            }
        }
    }

    /// The next key this repository has not used yet: `<prefix>-<n>`.
    ///
    /// `n` is one more than the highest numeric suffix any task CURRENTLY in
    /// this repository carries, read fresh from `tasks` rather than kept in a
    /// counter column. There is deliberately no persisted high-water mark: if
    /// the task holding the highest number is later deleted, the next key
    /// issued reuses that number. The prefix itself is read from the
    /// repository row, never derived from its current name; see this
    /// module's doc for why.
    pub fn next_task_key(&self, repo: Uuid) -> Result<String> {
        let prefix = self.get_repository(repo)?.task_key_prefix;

        // A plain aggregate query always returns exactly one row, even when
        // nothing matches the WHERE clause -- it is the aggregate itself
        // (here, MAX) that comes back NULL, not the row that goes missing.
        let max: Option<i64> = self
            .conn()
            .query_row(
                "SELECT MAX(CAST(SUBSTR(key, LENGTH(?1) + 2) AS INTEGER))
                 FROM tasks WHERE repository_id = ?2 AND key LIKE ?1 || '-%'",
                params![prefix, uuid_blob(repo)],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(map_err)?;

        Ok(format!("{prefix}-{}", max.unwrap_or(0) + 1))
    }
}

/// Unix milliseconds, now.
///
/// The rest of this crate takes the clock from its caller --
/// `create_repository_root` and `mark_reviewed` both do -- and the board
/// deliberately does not. `status_since` and a note's `at` say when a task
/// actually moved, and moving one is meant to be driven from the wire, where
/// a timestamp parameter would be a clock a client could set.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The character `search_notes` uses to mark an escaped wildcard in its
/// `LIKE` pattern -- SQLite's default `LIKE` treats `%` and `_` as wildcards
/// with no way to write a literal one, which is exactly the character a
/// person searching this tool's own history is likeliest to type: a table
/// name (`task_notes`, underscore and all) or a percentage (`50%`).
const LIKE_ESCAPE: char = '\\';

/// Rewrites `raw` so every `%`, `_`, and literal `\` in it is prefixed with
/// `LIKE_ESCAPE`, and so a `LIKE ... ESCAPE '\'` pattern built by wrapping the
/// result in a leading and trailing `%` matches `raw` as literal text, not as
/// a pattern with its own wildcards. `search_notes` is the only caller: a
/// search for `50%` must not also match `"5000"`, and a search for
/// `task_notes` must not also match `"taskXnotes"` -- a search that quietly
/// widens is worse than one that finds nothing, because the person reading
/// the results has no way to tell it happened.
fn escape_like(raw: &str) -> String {
    raw.replace(LIKE_ESCAPE, "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Every column of `tasks` that `row_to_task` reads, in its order. Named once
/// because several queries share it and a drifting column order is a silent
/// field swap rather than a compile error.
const TASK_COLUMNS: &str = "id, repository_id, key, title, status, status_since, \
     intent, acceptance, constraints, labels, workspace_id, resource_version";

/// The same, for `row_to_task_note`.
const NOTE_COLUMNS: &str = "id, task_id, kind, actor, at, body, extra, supersedes";

/// The one place `task_notes` is ever written.
///
/// Takes a `&Connection` rather than reaching for the store's own so that
/// `create_task` and `set_task_status` can call it inside their transactions,
/// which is what makes a row and the note recording it land together or not
/// at all.
///
/// There is no matching update helper, and there must never be one. The table
/// carries triggers that refuse an UPDATE outright and refuse a DELETE while
/// the note's task still exists, so an edit path could not work even if
/// somebody wrote one; correcting the record is `add_note_superseding`.
fn insert_note(
    conn: &Connection,
    task: Uuid,
    kind: NoteKind,
    actor: Actor,
    body: &str,
    extra: &serde_json::Value,
    supersedes: Option<Uuid>,
) -> Result<TaskNote> {
    let note = TaskNote {
        id: Uuid::now_v7(),
        task_id: task,
        kind,
        actor,
        at: now_millis(),
        body: body.to_string(),
        extra: extra.clone(),
        supersedes,
    };
    conn.execute(
        "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra, supersedes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            uuid_blob(note.id),
            uuid_blob(note.task_id),
            note.kind.as_str(),
            note.actor.to_string(),
            note.at,
            note.body,
            note.extra.to_string(),
            note.supersedes.map(uuid_blob),
        ],
    )
    .map_err(map_err)?;
    Ok(note)
}

impl Store {
    // ---- the task row: current understanding ----

    /// A new task, in the backlog, carrying this repository's next key.
    ///
    /// The key is read before the transaction opens, not inside it, so two
    /// creations racing can both read the same next key. `UNIQUE
    /// (repository_id, key)` is the referee when they do: the loser gets a
    /// conflict rather than a duplicate key.
    ///
    /// `actor` is recorded as the first entry in the record. The row has no
    /// author column -- who made a task is history, not current understanding,
    /// and history lives in notes.
    pub fn create_task(&self, repository: Uuid, title: &str, actor: Actor) -> Result<Task> {
        // Before the connection is locked: `next_task_key` locks it itself,
        // and the mutex is not reentrant.
        let key = self.next_task_key(repository)?;
        let now = now_millis();
        let task = Task {
            id: Uuid::now_v7(),
            key,
            repository_id: repository,
            title: title.to_string(),
            status: TaskStatus::Backlog,
            status_since: now,
            intent: String::new(),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            workspace_id: None,
            labels: Vec::new(),
            resource_version: 1,
        };

        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            "INSERT INTO tasks
             (id, repository_id, key, title, status, status_since, created_at, resource_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
            params![
                uuid_blob(task.id),
                uuid_blob(task.repository_id),
                task.key,
                task.title,
                task.status.as_str(),
                task.status_since,
                now,
            ],
        )
        .map_err(map_err)?;
        insert_note(&tx, task.id, NoteKind::Created, actor, title, &json!({}), None)?;
        tx.commit().map_err(map_err)?;

        Ok(task)
    }

    pub fn get_task(&self, task: Uuid) -> Result<Task> {
        self.conn()
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                params![uuid_blob(task)],
                row_to_task,
            )
            .map_err(map_err)
    }

    /// Every task in a repository, optionally narrowed to one status.
    ///
    /// Ordered by when each was created, so a listing read twice reads the
    /// same both times. `rowid` breaks a tie between two tasks created in the
    /// same millisecond, which is why the order does not depend on how coarse
    /// the clock happens to be.
    pub fn list_tasks(&self, repository: Uuid, status: Option<TaskStatus>) -> Result<Vec<Task>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM tasks
                  WHERE repository_id = ?1 AND (?2 IS NULL OR status = ?2)
                  ORDER BY created_at, rowid"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(repository), status.map(TaskStatus::as_str)], row_to_task)
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Revise what is currently understood about a task.
    ///
    /// Writes no note, deliberately. This is the mutable half of the design:
    /// intent and acceptance are meant to be rewritten as understanding
    /// improves, and a log entry per wording change would bury the reasoning
    /// the record exists to keep.
    ///
    /// Version-checked, unlike `set_task_status` below: two clients revising
    /// the same fields must not both believe they won, while a status move
    /// touches none of these fields and so is deliberately allowed to happen
    /// underneath a revision in flight.
    pub fn update_task(
        &self,
        task: Uuid,
        expected_version: u64,
        update: &TaskUpdate,
    ) -> Result<Task> {
        self.run_versioned(
            "UPDATE tasks
                SET title = ?1, intent = ?2, acceptance = ?3, constraints = ?4, labels = ?5,
                    workspace_id = ?6, resource_version = ?7
              WHERE id = ?8 AND resource_version = ?9",
            &[
                &update.title,
                &update.intent,
                &acceptance_to_json(&update.acceptance),
                &strings_to_json(&update.constraints),
                &strings_to_json(&update.labels),
                &update.workspace_id.map(uuid_blob),
                &(expected_version as i64 + 1),
                &uuid_blob(task),
                &(expected_version as i64),
            ],
            "SELECT 1 FROM tasks WHERE id = ?1",
            &[&uuid_blob(task)],
        )?;
        self.get_task(task)
    }

    /// Move a task, and record the move in the same transaction.
    ///
    /// A status change that failed to record itself would be a hole in the log
    /// exactly where it matters, so the row and its `StatusChange` note are
    /// one write.
    ///
    /// Unversioned, unlike `update_task`, and it does not bump
    /// `resource_version` either -- the same answer `add_note` gives, for the
    /// same reason. `TaskUpdate` carries no status, so a move and a revision
    /// touch disjoint fields and neither can lose the other's work; a version
    /// bump here would only fail a manager's in-flight `update_task` because
    /// an agent moved the task meanwhile, which is a conflict about nothing.
    ///
    /// The cost is real and is paid elsewhere: a client that refetches only
    /// when `resource_version` moves will not see a status change, so the
    /// change signal for a status move has to be an announced event rather
    /// than the version.
    ///
    /// Setting the status a task already has does nothing at all -- no row
    /// write, no note. `status_since` answers "how long has this sat here",
    /// and re-asserting a status is not sitting somewhere new.
    pub fn set_task_status(&self, task: Uuid, status: TaskStatus, actor: Actor) -> Result<Task> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;

        let existing: Task = tx
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                params![uuid_blob(task)],
                row_to_task,
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)?;

        if existing.status == status {
            return Ok(existing);
        }

        let now = now_millis();
        tx.execute(
            "UPDATE tasks SET status = ?1, status_since = ?2 WHERE id = ?3",
            params![status.as_str(), now, uuid_blob(task)],
        )
        .map_err(map_err)?;
        insert_note(
            &tx,
            task,
            NoteKind::StatusChange,
            actor,
            &format!("moved from {} to {}", existing.status.as_str(), status.as_str()),
            &json!({ "from": existing.status.as_str(), "to": status.as_str() }),
            None,
        )?;
        tx.commit().map_err(map_err)?;

        Ok(Task { status, status_since: now, ..existing })
    }

    // ---- the notes: the record of how it got there ----

    /// Append one entry to a task's record.
    ///
    /// There is no counterpart that changes an entry already there. That is
    /// the whole design: an edited decision is indistinguishable from a
    /// decision that was always that way, which makes the log worth nothing
    /// exactly when somebody leans on it.
    pub fn add_note(
        &self,
        task: Uuid,
        kind: NoteKind,
        actor: Actor,
        body: &str,
        extra: serde_json::Value,
    ) -> Result<TaskNote> {
        self.append(task, kind, actor, body, extra, None)
    }

    /// Correct the record: a NEW entry, naming the one it replaces.
    ///
    /// Both entries stay readable forever. "We decided X, then learned better
    /// and decided Y" is a thing a reader in three weeks needs to see, and an
    /// in-place edit would have shown them only Y.
    ///
    /// The superseded note must be on the same task and of the same kind. The
    /// column's foreign key only proves it exists somewhere.
    ///
    /// Same task, because a note correcting another task's history reads as a
    /// correction and is not one. Same kind, because the superseded note
    /// carries no back-pointer -- the only marker that a decision was
    /// retracted lives on the newer note, so a `Progress` note superseding a
    /// `Decision` would leave `notes_for(t, Some(Decision))` returning a
    /// retracted decision with nothing in the result set saying so.
    pub fn add_note_superseding(
        &self,
        task: Uuid,
        kind: NoteKind,
        actor: Actor,
        body: &str,
        extra: serde_json::Value,
        supersedes: Uuid,
    ) -> Result<TaskNote> {
        let superseded: Option<(Vec<u8>, String)> = self
            .conn()
            .query_row(
                "SELECT task_id, kind FROM task_notes WHERE id = ?1",
                params![uuid_blob(supersedes)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_err)?;
        let matches = superseded.is_some_and(|(owner, was)| {
            owner == task.as_bytes().as_slice() && was == kind.as_str()
        });
        if !matches {
            return Err(DomainError::InvalidArgument { what: "supersedes" });
        }
        self.append(task, kind, actor, body, extra, Some(supersedes))
    }

    /// The shared body of the two appenders, and the only caller of
    /// `insert_note` outside a transaction.
    ///
    /// `StatusChange` and `Created` are refused here. Both are written by the
    /// transactions that actually move a task, and this pair of appenders is
    /// meant to be the surface a client reaches: a caller appending a
    /// `StatusChange` whose `from` and `to` no status change ever produced
    /// would be writing a lie into a log whose entire claim is that a move is
    /// recorded rather than inferred.
    fn append(
        &self,
        task: Uuid,
        kind: NoteKind,
        actor: Actor,
        body: &str,
        extra: serde_json::Value,
        supersedes: Option<Uuid>,
    ) -> Result<TaskNote> {
        if matches!(kind, NoteKind::StatusChange | NoteKind::Created) {
            return Err(DomainError::InvalidArgument { what: "kind" });
        }
        let written = insert_note(&self.conn(), task, kind, actor, body, &extra, supersedes);
        match written {
            // A note whose task is gone fails on the foreign key, which maps
            // to a version conflict -- a sentence telling somebody to retry
            // something that can never work. It is a missing task, and it
            // should say so.
            Err(DomainError::ResourceConflict) if self.get_task(task).is_err() => {
                Err(DomainError::NotFound)
            }
            other => other,
        }
    }

    /// A task's record, oldest first, optionally narrowed to one kind.
    ///
    /// Oldest first because the record is read to follow how understanding
    /// moved, and `rowid` breaks a tie between two notes written in the same
    /// millisecond so that order is the order they were appended in.
    ///
    /// Narrowing is what makes the split pay: `notes_for(t, Some(Decision))`
    /// answers "why is it like this" without reading a wall of progress
    /// chatter.
    pub fn notes_for(&self, task: Uuid, kind: Option<NoteKind>) -> Result<Vec<TaskNote>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {NOTE_COLUMNS} FROM task_notes
                  WHERE task_id = ?1 AND (?2 IS NULL OR kind = ?2)
                  ORDER BY at, rowid"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(task), kind.map(NoteKind::as_str)], row_to_task_note)
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    // ---- blocking ----

    /// Record that `task` cannot proceed until `blocked_by` does, and why.
    ///
    /// **Not idempotent, unlike `unblock` below, and the name does not say
    /// so.** `task_blocks` is keyed `PRIMARY KEY (task_id, blocked_by)` and
    /// this is a plain `INSERT`, so blocking a pair that is already blocked
    /// hits the constraint and comes back `ResourceConflict` -- see `map_err`,
    /// which maps every constraint violation onto that. A caller correcting a
    /// `reason` therefore has to `unblock` the pair and block it again;
    /// re-setting it in place is refused.
    ///
    /// That is genuinely surprising for a `set_` function and it is recorded
    /// here rather than fixed, because turning this into an upsert is a
    /// semantic change with its own test surface -- one that belongs in a
    /// change of its own rather than folded into somebody else's. Until then
    /// this doc, `TaskBlockSet` in the proto, and `farcooler task block
    /// --reason`'s help all say the same thing, so the surprise is met in
    /// writing before it is met at runtime.
    ///
    /// Refuses a cycle. Before inserting, this walks the graph forward from
    /// `blocked_by` -- what `blocked_by` itself is blocked on, and what
    /// blocks THAT, and so on -- and refuses the moment the walk reaches
    /// `task`. A self-block (`task == blocked_by`) needs no separate check:
    /// the walk starts at `blocked_by` and the very first node it looks at
    /// is `task` itself.
    ///
    /// A cycle is a deadlock the manager would never resolve, and it would
    /// present as a queue that quietly stopped moving rather than as an
    /// error -- so this is checked here, once, rather than left for whatever
    /// eventually reads the graph to notice.
    ///
    /// The whole check-then-insert runs inside one transaction, holding the
    /// store's single connection for its duration: two `set_block` calls
    /// racing each other must not both walk the graph before either has
    /// written its edge, which is the one way a real cycle could slip past a
    /// check that only ever looked at a still-clean graph.
    ///
    /// The walk itself is bounded by the number of tasks in `task`'s OWN
    /// repository, read fresh inside this same transaction, so a corrupt or
    /// enormous graph cannot hang the daemon -- past that many distinct
    /// tasks visited, the walk gives up and refuses rather than keep going.
    ///
    /// That bound is narrower than it sounds. `task_blocks`' foreign keys
    /// (see `migrate.rs`) only guarantee both ids name real rows in `tasks`
    /// -- nothing constrains `task` and `blocked_by` to the same repository,
    /// and this function does not check that itself. A block whose chain
    /// wanders into a second repository's tasks is therefore a graph this
    /// function CAN write, and if that wandering pushes the walk past `task`'s
    /// own repository's task count, a legitimate acyclic chain can be refused
    /// as a cycle. Fixing that -- a schema constraint requiring both ends of
    /// a block to share a repository, a same-repository check here, or
    /// widening the bound to every task the runner holds -- is a design
    /// decision for this plan's owner, not one this function has made.
    pub fn set_block(&self, task: Uuid, blocked_by: Uuid, reason: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;

        let repository_id: Vec<u8> = tx
            .query_row(
                "SELECT repository_id FROM tasks WHERE id = ?1",
                params![uuid_blob(task)],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?
            .ok_or(DomainError::NotFound)?;
        let task_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE repository_id = ?1",
                params![repository_id],
                |r| r.get(0),
            )
            .map_err(map_err)?;

        if blocking_walk_reaches(&tx, blocked_by, task, task_count as usize)? {
            return Err(DomainError::InvalidArgument { what: "cycle" });
        }

        tx.execute(
            "INSERT INTO task_blocks (task_id, blocked_by, reason) VALUES (?1, ?2, ?3)",
            params![uuid_blob(task), uuid_blob(blocked_by), reason],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    /// Everything currently blocking `task`, one row per thing it is waiting
    /// on. `rowid` orders it, so a listing read twice reads the same both
    /// times -- the same reasoning `notes_for` and `list_tasks` follow.
    pub fn blocks_for(&self, task: Uuid) -> Result<Vec<TaskBlock>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!("SELECT {BLOCK_COLUMNS} FROM task_blocks WHERE task_id = ?1 ORDER BY rowid"))
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(task)], row_to_task_block).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Remove one block. Not an error if it was never there: whether it was
    /// resolved a moment ago by someone else or never existed, the state
    /// afterward -- `task` is not blocked on `blocked_by` -- is the same
    /// either way.
    pub fn unblock(&self, task: Uuid, blocked_by: Uuid) -> Result<()> {
        self.conn()
            .execute(
                "DELETE FROM task_blocks WHERE task_id = ?1 AND blocked_by = ?2",
                params![uuid_blob(task), uuid_blob(blocked_by)],
            )
            .map_err(map_err)?;
        Ok(())
    }

    // ---- reads a manager can afford ----

    /// Every note across a whole repository whose body contains `query`
    /// LITERALLY, oldest first, optionally narrowed to one kind, each hit
    /// flagged with whether some other note supersedes it.
    ///
    /// `notes_for` answers "why is it like this" for one task whose key you
    /// already have. This is the read for "why did we do it this way," asked
    /// months later, about a task nobody remembers the key of -- the case the
    /// split between `tasks` and `task_notes` exists for.
    ///
    /// A `LIKE` over `body`, not FTS5: the volume here is a person's tasks on
    /// one runner, not a search engine's corpus, and a second index is a
    /// second thing that can drift from the rows it is supposed to mirror.
    ///
    /// `query` is escaped (`escape_like`) before it reaches the pattern, and
    /// the pattern carries an explicit `ESCAPE` clause: SQLite's `LIKE`
    /// otherwise treats `%` and `_` IN THE QUERY as wildcards too, so a
    /// search for `50%` would also match `"5000"`, and a search for
    /// `task_notes` would also match `"taskXnotes"` -- exactly the two kinds
    /// of term (a percentage, a table name) someone searching this tool's own
    /// history is likely to type. A search that quietly widens is worse than
    /// one that finds nothing, because nothing in the result tells the reader
    /// it happened.
    ///
    /// `NoteHit::superseded` answers "has this been revised," a narrower and
    /// much cheaper question than "what is the current word on this," which
    /// this function still cannot answer from its own result set: the flag is
    /// a correlated `EXISTS (SELECT 1 FROM task_notes WHERE supersedes =
    /// tn.id)`, true the moment ANY note names this one in its `supersedes`
    /// column, regardless of whether that retracting note's own body matches
    /// `query` -- a search for "sqlite" flags "use sqlite" as superseded even
    /// when the note that superseded it reads "use files after all" and would
    /// never itself have matched. What this cannot do is hand back the
    /// retracting note's text, or a chain more than one link long, without a
    /// second read: a caller that needs the actual current word, not just the
    /// fact that this one is stale, follows `task_id` back to `notes_for`.
    pub fn search_notes(
        &self,
        repository: Uuid,
        query: &str,
        kind: Option<NoteKind>,
    ) -> Result<Vec<NoteHit>> {
        let pattern = format!("%{}%", escape_like(query));
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT tn.id, tn.task_id, tn.kind, tn.actor, tn.at, tn.body, tn.extra, tn.supersedes,
                        EXISTS (SELECT 1 FROM task_notes s WHERE s.supersedes = tn.id) AS superseded
                   FROM task_notes tn
                   JOIN tasks t ON t.id = tn.task_id
                  WHERE t.repository_id = ?1
                    AND tn.body LIKE ?2 ESCAPE '\\'
                    AND (?3 IS NULL OR tn.kind = ?3)
                  ORDER BY tn.at, tn.rowid",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map(
                params![uuid_blob(repository), pattern, kind.map(NoteKind::as_str)],
                row_to_note_hit,
            )
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// Tasks in `repository` that have sat in their current status longer
    /// than `threshold`, oldest-sitting first.
    ///
    /// The failure mode this whole arrangement is built to surface is not an
    /// agent doing the wrong thing -- it is a task sitting in `todo` that a
    /// manager assumed was in flight. `status_since` is on the row precisely
    /// so this never has to read a history to answer; see `set_task_status`
    /// for where that column is actually kept honest.
    ///
    /// `done` and `cancelled` are excluded outright, not merely treated as
    /// unlikely to qualify: a finished task sits still forever, and a
    /// staleness view that lists every completed task next to the ones that
    /// actually need attention is a view nobody reads.
    pub fn list_tasks_stale_for(&self, repository: Uuid, threshold: Duration) -> Result<Vec<Task>> {
        let cutoff = now_millis() - threshold.as_millis() as i64;
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {TASK_COLUMNS} FROM tasks
                  WHERE repository_id = ?1
                    AND status NOT IN ('done', 'cancelled')
                    AND status_since < ?2
                  ORDER BY status_since, rowid"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(repository), cutoff], row_to_task)
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}

/// Every column of `task_blocks`, in `row_to_task_block`'s order.
const BLOCK_COLUMNS: &str = "task_id, blocked_by, reason";

/// True if, starting at `start` and repeatedly following "what does this task
/// block on", the walk ever reaches `target`.
///
/// `limit` bounds the number of distinct tasks the walk will visit before
/// giving up and reporting a cycle rather than looping forever. See
/// `Store::set_block`'s doc: that bound is sized to `task`'s own repository,
/// and a chain that wanders into another repository's tasks can exhaust it
/// and be refused even when it is not actually a cycle.
fn blocking_walk_reaches(conn: &Connection, start: Uuid, target: Uuid, limit: usize) -> Result<bool> {
    let mut stack = vec![start];
    let mut seen = std::collections::HashSet::new();

    while let Some(node) = stack.pop() {
        if node == target {
            return Ok(true);
        }
        if !seen.insert(node) {
            continue;
        }
        if seen.len() > limit {
            // Past `task`'s own repository's task count. Almost always a
            // genuine cycle or a corrupt graph, but see `set_block`'s doc:
            // a chain that wanders into another repository's tasks can also
            // land here without being one, since nothing stops a block from
            // crossing repositories.
            return Ok(true);
        }

        let mut stmt = conn
            .prepare("SELECT blocked_by FROM task_blocks WHERE task_id = ?1")
            .map_err(map_err)?;
        let next = stmt
            .query_map(params![uuid_blob(node)], |r| get_uuid(r, 0))
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        stack.extend(next);
    }

    Ok(false)
}

#[cfg(test)]
impl Store {
    /// A repository with a real, assigned task key prefix -- not the schema's
    /// bare `''` default. Every collision and every prefix-stability test
    /// needs a genuinely non-empty prefix to have anything to prove.
    pub(crate) fn register_repository_for_test(&self, name: &str) -> Uuid {
        let host = Uuid::now_v7();
        // A fresh path per call: `repository_roots.path` is globally unique,
        // and a fixture registering more than one repository (every
        // collision test does) must not trip that constraint itself.
        let path = format!("/repos/test-{}", Uuid::now_v7());
        let root = self.create_repository_root(host, &path, 0).expect("root");
        let repo = self
            .create_repository(host, root.id, name, &format!("{path}/.git"), "")
            .expect("repo");
        self.assign_task_key_prefix(repo.id).expect("prefix");
        repo.id
    }

    pub(crate) fn rename_repository_for_test(&self, id: Uuid, new_name: &str) {
        let repo = self.get_repository(id).expect("repo");
        self.update_repository(id, repo.resource_version, new_name, &repo.canonical_git_dir, &repo.remote_summary)
            .expect("rename");
    }

    /// A task whose key is whatever `next_task_key` currently says, so a test
    /// creating one and then asking for the next key exercises the exact same
    /// counting the real thing does.
    pub(crate) fn create_task_for_test(&self, repo: Uuid, title: &str) -> Uuid {
        let key = self.next_task_key(repo).expect("key");
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO tasks (id, repository_id, key, title, status, status_since, created_at, resource_version)
                 VALUES (?1, ?2, ?3, ?4, 'backlog', 0, 0, 1)",
                params![uuid_blob(id), uuid_blob(repo), key, title],
            )
            .expect("insert task");
        id
    }

    /// Moves a task's `status_since` into the past by `ago`, and nothing
    /// else. The one way a staleness test gets an old-looking task without
    /// actually waiting for one.
    pub(crate) fn backdate_status_since_for_test(&self, task: Uuid, ago: Duration) {
        let since = now_millis() - ago.as_millis() as i64;
        self.conn()
            .execute(
                "UPDATE tasks SET status_since = ?1 WHERE id = ?2",
                params![since, uuid_blob(task)],
            )
            .expect("backdate status_since");
    }

    /// A note carrying a caller-chosen `at`, not `now_millis()`'s.
    ///
    /// There is no `backdate_note_at_for_test` UPDATE-based counterpart to
    /// `backdate_status_since_for_test` above, and there cannot be one:
    /// `task_notes_forbid_update` (`migrate.rs`) refuses EVERY `UPDATE` on
    /// `task_notes` unconditionally, with no `WHEN` clause carving out a
    /// test-only column the way `task_notes_forbid_delete` carves out a
    /// note whose task is already gone. An `UPDATE ... SET at = ...` here
    /// would abort exactly the way `there_is_no_path_that_rewrites_a_note`
    /// proves a body rewrite does.
    ///
    /// An `INSERT` is not blocked, and gets a test the same fixture it
    /// needs: every note written through `add_note` alone has `at`
    /// (stamped by `now_millis()` at the moment of that note's own INSERT),
    /// `rowid` (assigned at INSERT, always increasing), and even its
    /// UUIDv7 `id` (also time-ordered) all increase together -- so no
    /// fixture built only from `add_note` can tell `search_notes`'s
    /// `ORDER BY tn.at, tn.rowid` apart from an implementation that instead
    /// orders by `rowid` or by `id`. This inserts a note directly, the way
    /// `create_task_for_test` inserts a task directly, with `at` chosen by
    /// the caller rather than the clock -- letting a test put two notes'
    /// `rowid` order and `at` order in disagreement, which nothing written
    /// through the crate's own API can ever produce.
    pub(crate) fn insert_note_with_at_for_test(
        &self,
        task: Uuid,
        kind: NoteKind,
        body: &str,
        at: i64,
    ) -> Uuid {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra)
                 VALUES (?1, ?2, ?3, 'user', ?4, ?5, '{}')",
                params![uuid_blob(id), uuid_blob(task), kind.as_str(), at, body],
            )
            .expect("insert note with chosen at");
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::models::AcceptanceItem;

    #[test]
    fn a_prefix_is_letters_from_the_name_lowercased() {
        assert_eq!(derive_prefix("Far Cooler"), "fc");
        assert_eq!(derive_prefix("overnight"), "ov");
        assert_eq!(derive_prefix("my-web-app"), "mwa");
    }

    /// A name with nothing usable in it still needs a key.
    #[test]
    fn a_name_with_no_letters_falls_back_rather_than_producing_an_empty_prefix() {
        assert!(!derive_prefix("---").is_empty());
        assert!(!derive_prefix("").is_empty());
    }

    /// The reason the prefix is stored and not computed on read.
    ///
    /// A prefix derived on every read changes when a repository is renamed,
    /// and every key ever written into a note, spoken aloud, or pasted into a
    /// prompt stops resolving. Keys are the one identifier in this system that
    /// leaves the database.
    #[test]
    fn renaming_a_repository_does_not_change_its_task_keys() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        let before = store.next_task_key(repo).expect("key");
        store.rename_repository_for_test(repo, "Something Else");
        let after = store.next_task_key(repo).expect("key");
        assert_eq!(
            before.split('-').next(),
            after.split('-').next(),
            "the prefix is the repository's, once, forever"
        );
    }

    #[test]
    fn keys_count_up_within_a_repository() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        assert_eq!(store.next_task_key(repo).unwrap(), "fc-1");
        store.create_task_for_test(repo, "first");
        assert_eq!(store.next_task_key(repo).unwrap(), "fc-2");
    }

    /// A single repository proves the mechanism works; it proves nothing
    /// about collision resolution, since there is nothing to collide with.
    /// Two repositories that would derive the SAME prefix from their names
    /// ("Far Cooler" and "Far Corral" both reduce to "fc") is the fixture
    /// that actually exercises `assign_task_key_prefix`'s retry loop -- and
    /// distinguishes a real resolver from one that only ever returns the
    /// base prefix and lets the database's unique index fail the second
    /// registration outright.
    #[test]
    fn two_repositories_that_derive_the_same_prefix_do_not_collide() {
        let store = Store::open_in_memory().expect("store");
        let first = store.register_repository_for_test("Far Cooler");
        let second = store.register_repository_for_test("Far Corral");

        let first_prefix = store.get_repository(first).unwrap().task_key_prefix;
        let second_prefix = store.get_repository(second).unwrap().task_key_prefix;

        assert_eq!(first_prefix, "fc", "the first registrant gets the plain prefix");
        assert_ne!(
            second_prefix, first_prefix,
            "the second registrant must not silently share the first's prefix"
        );
        assert!(
            second_prefix.starts_with("fc"),
            "the resolved prefix is still recognizably derived from the name, got {second_prefix}"
        );

        // And each repository's tasks count up independently under its own,
        // now-distinct, prefix.
        assert_eq!(store.next_task_key(first).unwrap(), "fc-1");
        assert_eq!(store.next_task_key(second).unwrap(), format!("{second_prefix}-1"));
    }

    /// A watcher polling on version alone must be able to tell a bare
    /// `create_repository` (version 1, empty prefix) apart from a fully
    /// registered one (version 2, real prefix), so this write bumps
    /// `resource_version`. The nearest wrong implementation is exactly what this
    /// function looked like before this test existed: an `UPDATE` that
    /// writes `task_key_prefix` and leaves `resource_version` untouched,
    /// which this test would catch by seeing `2` come back as `1`.
    #[test]
    fn assigning_a_prefix_bumps_the_repositorys_version() {
        let store = Store::open_in_memory().expect("store");
        let repo = store.register_repository_for_test("Far Cooler");
        assert_eq!(
            store.get_repository(repo).unwrap().resource_version,
            2,
            "create_repository left it at 1; assign_task_key_prefix must bump it to 2"
        );
    }

    /// Two tasks in two different repositories both counting from `-1` proves
    /// the count is scoped per repository, not global -- the nearest wrong
    /// implementation reads `MAX` over the whole `tasks` table.
    #[test]
    fn key_numbering_does_not_leak_across_repositories() {
        let store = Store::open_in_memory().expect("store");
        let one = store.register_repository_for_test("One Project");
        let two = store.register_repository_for_test("Two Project");

        store.create_task_for_test(one, "first in one");
        store.create_task_for_test(one, "second in one");

        // `two` has had no tasks created yet: if the count were global, this
        // would come back "tp-3", not "tp-1".
        assert_eq!(store.next_task_key(two).unwrap(), "tp-1");
        assert_eq!(store.next_task_key(one).unwrap(), "op-3");
    }

    // ---- the board ----

    /// The one repository every test below works in.
    ///
    /// A fixed id rather than whatever `seeded` happened to generate, so a
    /// test can name the repository without threading it through every call.
    fn repo() -> Uuid {
        Uuid::from_u128(0x0000_fc00_0000_0000_0000_0000_0000_0001)
    }

    /// A store holding exactly that repository, with its task key prefix
    /// assigned by the real `assign_task_key_prefix`, so the keys these tests
    /// see are the keys production issues.
    fn seeded() -> Store {
        let store = Store::open_in_memory().expect("store");
        let host = Uuid::now_v7();
        let root = store.create_repository_root(host, "/repos/board", 0).expect("root");
        // Inserted directly rather than through `create_repository`, which
        // picks its own id: `repo()` has to be knowable before the row exists.
        store
            .conn()
            .execute(
                "INSERT INTO repositories
                 (id, host_id, repository_root_id, display_name, canonical_git_dir,
                  remote_summary, resource_version)
                 VALUES (?1, ?2, ?3, 'Far Cooler', '/repos/board/.git', '', 1)",
                params![uuid_blob(repo()), uuid_blob(host), uuid_blob(root.id)],
            )
            .expect("repository");
        store.assign_task_key_prefix(repo()).expect("prefix");
        store
    }

    #[test]
    fn a_status_change_records_when_it_happened_and_who_did_it() {
        let store = seeded();
        let task = store.create_task(repo(), "fix the thing", Actor::User).unwrap();
        let before = task.status_since;
        std::thread::sleep(std::time::Duration::from_millis(5));

        let after = store
            .set_task_status(task.id, TaskStatus::InProgress, Actor::Manager)
            .unwrap();

        assert_eq!(after.status, TaskStatus::InProgress);
        assert!(after.status_since > before, "the board's staleness column depends on this");

        let notes = store.notes_for(task.id, None).unwrap();
        let change = notes.iter().find(|n| n.kind == NoteKind::StatusChange).expect("recorded");
        assert_eq!(change.actor, Actor::Manager, "who moved it is the point of the log");
        assert_eq!(change.extra["from"], "backlog");
        assert_eq!(change.extra["to"], "in_progress");
    }

    /// The constraint the whole design rests on, checked through the store's
    /// own API rather than only in the schema.
    #[test]
    fn correcting_the_record_supersedes_rather_than_edits() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let first = store
            .add_note(task.id, NoteKind::Decision, Actor::Manager, "use sqlite", json!({}))
            .unwrap();
        let second = store
            .add_note_superseding(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "use files after all",
                json!({}),
                first.id,
            )
            .unwrap();

        let notes = store.notes_for(task.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(notes.len(), 2, "the old decision is still there; that is the point");
        assert_eq!(second.supersedes, Some(first.id));
    }

    #[test]
    fn a_decision_keeps_what_it_rejected() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let note = store
            .add_note(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "sqlite, not files",
                json!({ "rejected": ["files: merge conflicts on every status change"] }),
            )
            .unwrap();
        assert_eq!(note.extra["rejected"][0], "files: merge conflicts on every status change");
    }

    #[test]
    fn an_agent_note_names_the_terminal_you_can_go_and_read() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let terminal = Uuid::now_v7();
        let note = store
            .add_note(task.id, NoteKind::Progress, Actor::Agent { terminal }, "building", json!({}))
            .unwrap();
        assert_eq!(note.actor, Actor::Agent { terminal });
    }

    #[test]
    fn listing_filters_by_status_and_reports_staleness() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        store.create_task(repo(), "b", Actor::User).unwrap();
        store.set_task_status(a.id, TaskStatus::InProgress, Actor::Manager).unwrap();

        let in_progress = store.list_tasks(repo(), Some(TaskStatus::InProgress)).unwrap();
        assert_eq!(in_progress.len(), 1);
        assert_eq!(in_progress[0].id, a.id);
    }

    /// `create_task` is the only thing that issues a key; nothing outside a
    /// test calls `next_task_key` directly. Two tasks, because one proves
    /// only that a key was produced: the nearest wrong implementation asks
    /// `next_task_key` for a key and never writes the row it counted, and so
    /// hands out `fc-1` twice.
    #[test]
    fn a_created_task_carries_the_repositorys_next_key() {
        let store = seeded();
        let first = store.create_task(repo(), "first", Actor::User).unwrap();
        let second = store.create_task(repo(), "second", Actor::User).unwrap();
        assert_eq!(first.key, "fc-1");
        assert_eq!(second.key, "fc-2");
        assert_eq!(first.status, TaskStatus::Backlog, "a task is born in the backlog");
    }

    /// The mutable half of the design, and the half that writes nothing to the
    /// record. The nearest wrong implementation is an `update_task` that also
    /// appends a note "recording" the revision, which would bury the reasoning
    /// the log exists for under every wording tweak.
    #[test]
    fn revising_the_understanding_rewrites_the_row_and_records_nothing() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let notes_before = store.notes_for(task.id, None).unwrap().len();
        let item = Uuid::now_v7();

        let updated = store
            .update_task(
                task.id,
                task.resource_version,
                &TaskUpdate {
                    title: "t, better understood".to_string(),
                    intent: "the daemon drops the second connection".to_string(),
                    acceptance: vec![AcceptanceItem {
                        id: item,
                        text: "a second client reconnects".to_string(),
                        met: false,
                    }],
                    constraints: vec!["never blocks the reactor".to_string()],
                    labels: vec!["daemon".to_string()],
                    workspace_id: None,
                },
            )
            .unwrap();

        assert_eq!(updated.intent, "the daemon drops the second connection");
        assert_eq!(updated.constraints, ["never blocks the reactor"]);
        assert_eq!(updated.acceptance.len(), 1);
        assert_eq!(updated.acceptance[0].id, item, "an acceptance item keeps the id it was given");
        assert_eq!(updated.resource_version, task.resource_version + 1);
        assert_eq!(
            store.notes_for(task.id, None).unwrap().len(),
            notes_before,
            "revising current understanding is not an entry in the record"
        );
        // Read back rather than trusted from the value `update_task` returned:
        // acceptance, constraints and labels all round-trip through JSON text
        // columns, and only a fresh read proves the decoding matches.
        assert_eq!(store.get_task(task.id).unwrap(), updated);
    }

    /// Versioned, unlike `set_task_status`: two clients revising the same
    /// fields must not both believe they won.
    #[test]
    fn a_revision_against_a_stale_version_is_refused() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let update = TaskUpdate {
            title: "t".to_string(),
            intent: "first".to_string(),
            acceptance: Vec::new(),
            constraints: Vec::new(),
            labels: Vec::new(),
            workspace_id: None,
        };
        store.update_task(task.id, task.resource_version, &update).expect("the first write wins");

        let err = store
            .update_task(task.id, task.resource_version, &update)
            .expect_err("the second holds a version that has moved");
        assert!(matches!(err, DomainError::ResourceConflict), "got {err}");
    }

    /// Setting the status a task already has is not a change, and must not
    /// look like one. `status_since` answers "how long has this sat here",
    /// which is the board's most important column; a manager re-asserting a
    /// status it read a moment ago would otherwise hide a week-old stall.
    #[test]
    fn re_asserting_the_status_a_task_already_has_does_not_restart_its_clock() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));

        let again = store.set_task_status(task.id, TaskStatus::Backlog, Actor::Manager).unwrap();

        assert_eq!(again.status_since, task.status_since, "nothing moved, so the clock did not");
        assert!(
            store.notes_for(task.id, Some(NoteKind::StatusChange)).unwrap().is_empty(),
            "nothing changed, so there is nothing to record"
        );
    }

    /// A status change nobody can land is `NotFound`, and leaves nothing
    /// behind.
    ///
    /// This proves the error and the empty table, and deliberately not the
    /// transaction: the foreign key refuses a note for a missing task whether
    /// or not one is open. `a_status_change_whose_note_is_refused_leaves_the_row_where_it_was`
    /// is what actually exercises the rollback.
    #[test]
    fn a_status_change_that_cannot_land_writes_no_note() {
        let store = seeded();
        let err = store
            .set_task_status(Uuid::now_v7(), TaskStatus::Done, Actor::Manager)
            .expect_err("there is no such task");
        assert!(matches!(err, DomainError::NotFound), "got {err}");

        let notes: i64 = store
            .conn()
            .query_row("SELECT count(*) FROM task_notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(notes, 0);
    }

    /// A note belongs to its task, and `notes_for` must not hand a reader
    /// another task's history. The nearest wrong implementation filters on
    /// `kind` alone.
    #[test]
    fn notes_do_not_leak_between_tasks() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        store.add_note(a.id, NoteKind::Decision, Actor::User, "a's", json!({})).unwrap();
        store.add_note(b.id, NoteKind::Decision, Actor::User, "b's", json!({})).unwrap();

        let for_a = store.notes_for(a.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(for_a.len(), 1);
        assert_eq!(for_a[0].body, "a's");
        assert_eq!(for_a[0].task_id, a.id);
    }

    /// The whole point of the split, stated as a test: there is no API that
    /// edits a note, and the schema refuses one even if somebody writes the
    /// SQL by hand. `insert_note` is the only writer -- the two appenders and
    /// the two transactions all go through it -- and it only ever INSERTs, so
    /// no caller of any of them can reach a row that already exists.
    #[test]
    fn there_is_no_path_that_rewrites_a_note() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let note =
            store.add_note(task.id, NoteKind::Decision, Actor::Manager, "as written", json!({}))
                .unwrap();

        let err = store
            .conn()
            .execute(
                "UPDATE task_notes SET body = 'rewritten' WHERE id = ?1",
                params![uuid_blob(note.id)],
            )
            .expect_err("the record refuses");
        assert!(err.to_string().contains("append-only"), "{err}");

        let decisions = store.notes_for(task.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(decisions[0].body, "as written");
    }

    /// `as_str` and `parse` are two lists that have to agree, and nothing but
    /// this notices when they stop. A single mistyped arm would make one
    /// status or kind unreadable on the way back out, and `get_status`
    /// refuses an unreadable status rather than guessing -- so the defect
    /// would present as one whole task that can no longer be read.
    #[test]
    fn every_status_and_kind_round_trips_through_its_stored_form() {
        for status in [
            TaskStatus::Backlog,
            TaskStatus::Todo,
            TaskStatus::NeedsDecision,
            TaskStatus::InProgress,
            TaskStatus::InReview,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::parse(status.as_str()), Some(status), "{status:?}");
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
            assert_eq!(NoteKind::parse(kind.as_str()), Some(kind), "{kind:?}");
        }
        let terminal = Uuid::now_v7();
        for actor in [Actor::User, Actor::Manager, Actor::Agent { terminal }] {
            assert_eq!(Actor::parse(&actor.to_string()), Some(actor), "{actor}");
        }

        // And nothing unreadable is quietly accepted as something else.
        assert_eq!(TaskStatus::parse("nonsense"), None);
        assert_eq!(NoteKind::parse("nonsense"), None);
        assert_eq!(Actor::parse("agent:not-a-uuid"), None);
    }

    /// A note about a task that does not exist is a missing task, not a
    /// version conflict. The foreign key raises the latter, and a person told
    /// their write conflicted would retry something that can never work.
    #[test]
    fn a_note_on_a_task_that_does_not_exist_says_so() {
        let store = seeded();
        let err = store
            .add_note(Uuid::now_v7(), NoteKind::Comment, Actor::User, "hello?", json!({}))
            .expect_err("there is no such task");
        assert!(matches!(err, DomainError::NotFound), "got {err}");
    }

    /// A correction has to be a correction OF something, on the task it
    /// claims to correct. The column's foreign key only proves the superseded
    /// note exists somewhere; a note pointing at another task's history would
    /// read as a correction and not be one.
    #[test]
    fn a_note_cannot_supersede_another_tasks_note() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        let theirs =
            store.add_note(b.id, NoteKind::Decision, Actor::Manager, "b's call", json!({})).unwrap();

        let err = store
            .add_note_superseding(
                a.id,
                NoteKind::Decision,
                Actor::Manager,
                "not mine to correct",
                json!({}),
                theirs.id,
            )
            .expect_err("must refuse");
        assert!(matches!(err, DomainError::InvalidArgument { .. }), "got {err}");
        assert_eq!(
            store.notes_for(b.id, Some(NoteKind::Decision)).unwrap().len(),
            1,
            "and the other task's record is untouched"
        );
    }

    /// The design's one link, read back OUT of the database rather than taken
    /// from the struct `add_note_superseding` built on the way in.
    ///
    /// `insert_note` returns a value it constructed before the INSERT, so an
    /// assertion against that value proves only that an argument reached a
    /// struct field -- it holds just as well if the column is written NULL
    /// forever. That failure is invisible from inside the process and total
    /// from outside it: two `Decision` notes with no relation between them,
    /// so "we decided X, then learned better" reads as two people deciding
    /// two different things.
    #[test]
    fn a_supersede_link_survives_the_database() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let first = store
            .add_note(task.id, NoteKind::Decision, Actor::Manager, "use sqlite", json!({}))
            .unwrap();
        let second = store
            .add_note_superseding(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "use files after all",
                json!({}),
                first.id,
            )
            .unwrap();

        let read = store.notes_for(task.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].id, first.id);
        assert_eq!(read[0].supersedes, None, "the first entry corrected nothing");
        assert_eq!(read[1].id, second.id);
        assert_eq!(
            read[1].supersedes,
            Some(first.id),
            "the link is the design; a NULL here is two unrelated decisions"
        );
    }

    /// The same weakness as the supersede link, on the two other fields whose
    /// entire value is that a reader gets them back out later: the terminal
    /// you can go and open, and what a decision rejected.
    #[test]
    fn an_agents_terminal_and_a_decisions_alternatives_survive_the_database() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let terminal = Uuid::now_v7();
        store
            .add_note(task.id, NoteKind::Progress, Actor::Agent { terminal }, "building", json!({}))
            .unwrap();
        store
            .add_note(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "sqlite, not files",
                json!({ "rejected": ["files: merge conflicts on every status change"] }),
            )
            .unwrap();

        let progress = store.notes_for(task.id, Some(NoteKind::Progress)).unwrap();
        assert_eq!(
            progress[0].actor,
            Actor::Agent { terminal },
            "an entry saying an agent did something is only useful if you can find the pane"
        );

        let decisions = store.notes_for(task.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(
            decisions[0].extra["rejected"][0],
            "files: merge conflicts on every status change"
        );
    }

    /// `create_task` takes an actor and the task row has no author column, so
    /// this note is the only place who made a task is ever written down.
    #[test]
    fn creating_a_task_records_who_made_it() {
        let store = seeded();
        let terminal = Uuid::now_v7();
        let task = store.create_task(repo(), "fix the thing", Actor::Agent { terminal }).unwrap();

        let created = store.notes_for(task.id, Some(NoteKind::Created)).unwrap();
        assert_eq!(created.len(), 1, "a task's record starts with how it started");
        assert_eq!(
            created[0].actor,
            Actor::Agent { terminal },
            "the caller's actor, not whichever one the implementation felt like"
        );
        assert_eq!(created[0].body, "fix the thing", "and what it was originally called");
    }

    /// The row and its note are one write, proven by making the note fail
    /// AFTER the row update has already succeeded -- which is the only
    /// arrangement that tells a transaction apart from two statements in a
    /// row. An implementation without one leaves the task moved and the move
    /// unrecorded, which is the hole in the log this design cannot afford.
    #[test]
    fn a_status_change_whose_note_is_refused_leaves_the_row_where_it_was() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        store
            .conn()
            .execute_batch(
                "CREATE TRIGGER refuse_status_notes BEFORE INSERT ON task_notes
                 WHEN NEW.kind = 'status_change'
                 BEGIN SELECT RAISE(ABORT, 'injected: the note cannot be written'); END;",
            )
            .expect("fault injection");

        let err = store
            .set_task_status(task.id, TaskStatus::Done, Actor::Manager)
            .expect_err("the note cannot be written, so the move cannot happen");
        assert!(matches!(err, DomainError::ResourceConflict), "got {err}");

        let after = store.get_task(task.id).unwrap();
        assert_eq!(after.status, TaskStatus::Backlog, "a move the log never saw did not happen");
        assert_eq!(after.status_since, task.status_since, "nor did its clock move");
    }

    /// A correction has to correct the same kind of thing. The superseded
    /// note carries no back-pointer, so the only marker that a decision was
    /// retracted lives on the newer note -- and a `Progress` note superseding
    /// a `Decision` leaves `notes_for(t, Some(Decision))`, whose whole job is
    /// answering "why is it like this", returning a retracted decision with
    /// nothing in the result set saying so.
    #[test]
    fn a_note_cannot_supersede_a_note_of_another_kind() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let decision = store
            .add_note(task.id, NoteKind::Decision, Actor::Manager, "sqlite", json!({}))
            .unwrap();

        let err = store
            .add_note_superseding(
                task.id,
                NoteKind::Progress,
                Actor::Manager,
                "still going",
                json!({}),
                decision.id,
            )
            .expect_err("must refuse");
        assert!(matches!(err, DomainError::InvalidArgument { .. }), "got {err}");

        let decisions = store.notes_for(task.id, Some(NoteKind::Decision)).unwrap();
        assert_eq!(decisions.len(), 1, "and the decision stands, unretracted");
        assert_eq!(decisions[0].supersedes, None);
    }

    /// `StatusChange` and `Created` are written by the two transactions that
    /// actually move a task, and by nothing else. `add_note` is meant to be
    /// callable by anyone, so a caller appending a `StatusChange` carrying a
    /// `from` and `to` that no status change ever produced would be writing a
    /// lie into a log whose entire claim is that a move is recorded rather
    /// than inferred.
    #[test]
    fn a_caller_cannot_append_a_status_change_or_a_creation_by_hand() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();

        for forged in [NoteKind::StatusChange, NoteKind::Created] {
            let err = store
                .add_note(task.id, forged, Actor::User, "never happened", json!({}))
                .expect_err("must refuse");
            assert!(matches!(err, DomainError::InvalidArgument { .. }), "{forged:?}: {err}");
        }

        assert!(
            store.notes_for(task.id, Some(NoteKind::StatusChange)).unwrap().is_empty(),
            "and nothing was written"
        );
        assert_eq!(
            store.notes_for(task.id, Some(NoteKind::Created)).unwrap().len(),
            1,
            "the one `create_task` wrote is still the only one"
        );
    }

    /// A status move and a revision touch different fields -- `TaskUpdate`
    /// carries no status -- so neither can lose the other's work, and an agent
    /// moving a task to `in_review` must not fail a manager's in-flight
    /// `update_task`. That is exactly the false conflict `add_note` is spared,
    /// for exactly the same reason, and the pair has to answer the same way.
    #[test]
    fn a_status_move_does_not_conflict_with_a_revision_in_flight() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let terminal = Uuid::now_v7();

        // The manager holds the version it read; an agent moves the task.
        store
            .set_task_status(task.id, TaskStatus::InProgress, Actor::Agent { terminal })
            .unwrap();

        let revised = store
            .update_task(
                task.id,
                task.resource_version,
                &TaskUpdate {
                    title: "t".to_string(),
                    intent: "now better understood".to_string(),
                    acceptance: Vec::new(),
                    constraints: Vec::new(),
                    labels: Vec::new(),
                    workspace_id: None,
                },
            )
            .expect("a status move is not a competing revision");

        assert_eq!(revised.intent, "now better understood");
        assert_eq!(
            revised.status,
            TaskStatus::InProgress,
            "and the move it did not conflict with still stands"
        );
    }

    /// The task ROW, read back, for the two writers that return a struct they
    /// built rather than one they read.
    ///
    /// The same defect as the supersede link, one field over: `set_task_status`
    /// could write any status it liked and return the one it was asked for.
    ///
    /// A wrong `status` on the row would still be caught elsewhere --
    /// `listing_filters_by_status_and_reports_staleness` discards the return
    /// and goes through `list_tasks`'s own SELECT, and
    /// `a_status_move_does_not_conflict_with_a_revision_in_flight` reads
    /// through `update_task`, which ends in a fresh `get_task`. A wrong
    /// `status_since` is caught here and nowhere else, which is the one that
    /// matters: that is the board's staleness column.
    #[test]
    fn a_created_and_moved_task_reads_back_as_what_was_returned() {
        let store = seeded();

        let created = store.create_task(repo(), "fix the thing", Actor::User).unwrap();
        assert_eq!(
            store.get_task(created.id).unwrap(),
            created,
            "the row `create_task` wrote, not the struct it returned"
        );

        let moved =
            store.set_task_status(created.id, TaskStatus::InReview, Actor::Manager).unwrap();
        let row = store.get_task(created.id).unwrap();
        assert_eq!(row, moved, "the row `set_task_status` wrote, not the struct it returned");
        assert_eq!(row.status, TaskStatus::InReview);
        assert_eq!(row.status_since, moved.status_since, "including the board's staleness column");
    }

    /// A task's key is scoped to its repository, and so is every listing of
    /// it. The nearest wrong implementation forgets the `repository_id`
    /// predicate and shows one runner's whole board under every repository.
    #[test]
    fn a_listing_shows_only_its_own_repositorys_tasks() {
        let store = seeded();
        let other = store.register_repository_for_test("Other Thing");
        store.create_task(repo(), "ours", Actor::User).unwrap();
        store.create_task(other, "theirs", Actor::User).unwrap();

        let ours = store.list_tasks(repo(), None).unwrap();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].title, "ours");
    }

    // ---- blocking ----

    #[test]
    fn a_task_can_be_blocked_on_several_things_each_with_a_reason() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        let c = store.create_task(repo(), "c", Actor::User).unwrap();
        store.set_block(a.id, b.id, "needs the migration first").unwrap();
        store.set_block(a.id, c.id, "needs the CLI").unwrap();

        let blocks = store.blocks_for(a.id).unwrap();
        assert_eq!(blocks.len(), 2);
        assert!(blocks.iter().any(|x| x.reason == "needs the migration first"));
    }

    /// A cycle is a deadlock the manager would never resolve, and it would
    /// present as a queue that quietly stops moving rather than as an error.
    #[test]
    fn a_cycle_is_refused_rather_than_stored() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        store.set_block(a.id, b.id, "").unwrap();

        let err = store.set_block(b.id, a.id, "").expect_err("must refuse");
        assert!(
            format!("{err}").contains("cycle"),
            "the refusal says what is wrong: {err}"
        );
    }

    /// A two-task fixture cannot tell a real cycle check apart from one that
    /// only ever refuses the immediate reverse of an edge just inserted (or
    /// only ever refuses a self-block): with just `a` and `b`, both of those
    /// narrower checks happen to give the same answer as a real walk of the
    /// graph. Three tasks, with the new edge closing the loop one hop further
    /// away than the edge it would directly reverse, is what actually forces
    /// the walk to look past its immediate neighbor.
    #[test]
    fn a_cycle_three_tasks_long_is_also_refused() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        let c = store.create_task(repo(), "c", Actor::User).unwrap();
        store.set_block(a.id, b.id, "a waits on b").unwrap();
        store.set_block(b.id, c.id, "b waits on c").unwrap();

        // Closing the loop: c waits on a, and a already (transitively) waits
        // on c. Neither `c == a` nor a row already blocking `a` on `c` is
        // true yet, so only a real walk from `a` through `b` to `c` notices.
        let err = store.set_block(c.id, a.id, "c waits on a").expect_err("must refuse");
        assert!(format!("{err}").contains("cycle"), "the refusal says what is wrong: {err}");

        // And the graph is exactly as it was before the refused call: two
        // edges, not three.
        assert_eq!(store.blocks_for(a.id).unwrap().len(), 1);
        assert_eq!(store.blocks_for(b.id).unwrap().len(), 1);
        assert!(store.blocks_for(c.id).unwrap().is_empty());
    }

    /// A revisit inside one walk is not a cycle: it means "this task was
    /// already found some other way," which is exactly what two things
    /// sharing a dependency looks like -- `p` waiting on both `q1` and `q2`,
    /// which both in turn wait on the same `r`, is legitimate and must be
    /// allowed. `blocking_walk_reaches`'s `if !seen.insert(node) { continue;
    /// }` treats a revisit as nothing new to learn; the nearby-wrong version
    /// -- `return Ok(true)` on that same line -- would refuse every
    /// multi-parent convergence, and no straight-line fixture would ever
    /// notice, including the three-task cycle above: a straight line never
    /// revisits a node, so it cannot exercise this line at all.
    #[test]
    fn a_diamond_of_shared_dependencies_is_allowed() {
        let store = seeded();
        let p = store.create_task(repo(), "p", Actor::User).unwrap();
        let q1 = store.create_task(repo(), "q1", Actor::User).unwrap();
        let q2 = store.create_task(repo(), "q2", Actor::User).unwrap();
        let r = store.create_task(repo(), "r", Actor::User).unwrap();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();

        // p waits on both q1 and q2, and both of those wait on the same r.
        store.set_block(p.id, q1.id, "").unwrap();
        store.set_block(p.id, q2.id, "").unwrap();
        store.set_block(q1.id, r.id, "").unwrap();
        store.set_block(q2.id, r.id, "").unwrap();

        // a's walk from p reaches r twice -- once through q1, once through
        // q2 -- which is the revisit this test exists to exercise.
        store.set_block(a.id, p.id, "").expect("a diamond below p is not a cycle");

        assert_eq!(store.blocks_for(a.id).unwrap().len(), 1);
        assert_eq!(store.blocks_for(p.id).unwrap().len(), 2, "p still waits on both q1 and q2");
    }

    #[test]
    fn a_task_cannot_block_on_itself() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        assert!(store.set_block(a.id, a.id, "").is_err());
    }

    /// The other half of the pair, read back OUT of the database rather than
    /// taken on faith from `unblock` returning `Ok(())`: a no-op that quietly
    /// left the row behind would be invisible from `unblock`'s own return
    /// value and total from `blocks_for`'s.
    #[test]
    fn unblocking_removes_the_edge_and_only_that_edge() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        let c = store.create_task(repo(), "c", Actor::User).unwrap();
        store.set_block(a.id, b.id, "needs the migration first").unwrap();
        store.set_block(a.id, c.id, "needs the CLI").unwrap();

        store.unblock(a.id, b.id).unwrap();

        let remaining = store.blocks_for(a.id).unwrap();
        assert_eq!(remaining.len(), 1, "only the one edge named should go");
        assert_eq!(remaining[0].blocked_by, c.id);
    }

    /// Unblocking something that was never a block, or was already removed,
    /// ends in the same state either way -- not an error a caller has to
    /// special-case.
    #[test]
    fn unblocking_something_never_blocked_is_not_an_error() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        store.unblock(a.id, b.id).unwrap();
        assert!(store.blocks_for(a.id).unwrap().is_empty());
    }

    /// Once a block is lifted, the edge that would have closed a cycle is
    /// free to be added: a refusal is about the graph as it stands, not a
    /// permanent memory of a pair of ids.
    #[test]
    fn unblocking_a_link_in_a_cycle_lets_it_be_closed_the_other_way() {
        let store = seeded();
        let a = store.create_task(repo(), "a", Actor::User).unwrap();
        let b = store.create_task(repo(), "b", Actor::User).unwrap();
        store.set_block(a.id, b.id, "").unwrap();
        store.set_block(b.id, a.id, "").expect_err("still a cycle while a->b stands");

        store.unblock(a.id, b.id).unwrap();
        store.set_block(b.id, a.id, "now it's b waiting on a").unwrap();

        let blocks = store.blocks_for(b.id).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].blocked_by, a.id);
    }

    // ---- reads a manager can afford ----

    /// "Why did we do it this way" is asked months later, about a task nobody
    /// remembers the key of. Without this the board is a queue, not a memory.
    ///
    /// The brief's fixture, plus one assertion this fix round added: the one
    /// hit here was never superseded, so `NoteHit::superseded` must read
    /// `false` -- without this line, `search_flags_a_hit_that_was_later_superseded`
    /// below could pass against an implementation that returns `true` for
    /// every row unconditionally, and this test alone would never notice.
    #[test]
    fn decisions_are_searchable_across_every_task_in_a_repository() {
        let store = seeded();
        let a = store.create_task(repo(), "storage", Actor::User).unwrap();
        let b = store.create_task(repo(), "unrelated", Actor::User).unwrap();
        store
            .add_note(a.id, NoteKind::Decision, Actor::Manager, "sqlite over files", json!({}))
            .unwrap();
        store
            .add_note(b.id, NoteKind::Progress, Actor::Manager, "sqlite is installed", json!({}))
            .unwrap();

        let hits = store.search_notes(repo(), "sqlite", Some(NoteKind::Decision)).unwrap();
        assert_eq!(hits.len(), 1, "narrowing to decisions skips the progress chatter");
        assert_eq!(hits[0].note.task_id, a.id);
        assert!(!hits[0].superseded, "nothing has ever replaced this decision");

        let all = store.search_notes(repo(), "sqlite", None).unwrap();
        assert_eq!(all.len(), 2);
    }

    /// The whole reason this fix round exists: SQLite's `LIKE` treats `%` and
    /// `_` as wildcards even when they appear IN THE SEARCH TERM, and the two
    /// terms most likely to appear in a search of this tool's own history are
    /// exactly those characters -- a table name (`task_notes`) and a
    /// percentage (`50%`). An unescaped search silently returns more than it
    /// claims to, and nothing in the result says so.
    #[test]
    fn a_percent_or_underscore_in_the_query_is_matched_literally_not_as_a_wildcard() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        store.add_note(task.id, NoteKind::Finding, Actor::User, "grep task_notes", json!({})).unwrap();
        store.add_note(task.id, NoteKind::Finding, Actor::User, "taskXnotes typo", json!({})).unwrap();
        store.add_note(task.id, NoteKind::Finding, Actor::User, "hit rate is 50%", json!({})).unwrap();
        store.add_note(task.id, NoteKind::Finding, Actor::User, "5000 requests", json!({})).unwrap();

        let underscore_hits = store.search_notes(repo(), "task_notes", None).unwrap();
        assert_eq!(underscore_hits.len(), 1, "`_` must match only a literal underscore");
        assert_eq!(underscore_hits[0].note.body, "grep task_notes");

        let percent_hits = store.search_notes(repo(), "50%", None).unwrap();
        assert_eq!(percent_hits.len(), 1, "`%` must match only a literal percent sign");
        assert_eq!(percent_hits[0].note.body, "hit rate is 50%");
    }

    /// The escape character itself has to be escaped, and correct only "by
    /// inspection" until a test proves it. `escape_like` runs three
    /// `.replace` calls in sequence -- backslash, then `%`, then `_` -- and
    /// the order is load-bearing: escaping backslash FIRST means the
    /// backslashes `%` and `_` insert next are never re-escaped by a step
    /// that already ran. A query with only a literal `\` and no `%` or `_`
    /// cannot tell that order apart from its reverse (nothing exists yet
    /// for a misordered backslash step to corrupt), so this combines a
    /// literal backslash with a `%` in the SAME query, which is exactly the
    /// shape that breaks under the wrong order: escaping `%` first inserts
    /// a backslash, and THEN escaping backslash (wrongly, last) doubles
    /// that inserted backslash too, along with the query's own literal one,
    /// scrambling the pattern.
    #[test]
    fn a_literal_backslash_in_the_query_is_matched_literally() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        store
            .add_note(task.id, NoteKind::Finding, Actor::User, "rate is 50%\\d always", json!({}))
            .unwrap();
        store
            .add_note(task.id, NoteKind::Finding, Actor::User, "5000d nothing else", json!({}))
            .unwrap();

        let hits = store.search_notes(repo(), "50%\\d", None).unwrap();
        assert_eq!(hits.len(), 1, "the literal backslash and percent must both match literally");
        assert_eq!(hits[0].note.body, "rate is 50%\\d always");
    }

    /// The brief's own fixture above cannot tell a repository-scoped search
    /// apart from one that searches every repository on the runner: both
    /// tasks it creates live in `repo()`. A second repository holding a note
    /// with the same matching body is what actually exercises the join's
    /// `WHERE t.repository_id = ?1` -- the same gap `notes_do_not_leak_between_tasks`
    /// and `a_listing_shows_only_its_own_repositorys_tasks` close for
    /// `notes_for` and `list_tasks`.
    #[test]
    fn search_does_not_leak_across_repositories() {
        let store = seeded();
        let other = store.register_repository_for_test("Other Thing");
        let ours = store.create_task(repo(), "ours", Actor::User).unwrap();
        let theirs = store.create_task(other, "theirs", Actor::User).unwrap();
        store.add_note(ours.id, NoteKind::Decision, Actor::User, "sqlite", json!({})).unwrap();
        store.add_note(theirs.id, NoteKind::Decision, Actor::User, "sqlite", json!({})).unwrap();

        let hits = store.search_notes(repo(), "sqlite", None).unwrap();
        assert_eq!(hits.len(), 1, "the other repository's matching note must not appear");
        assert_eq!(hits[0].note.task_id, ours.id);
    }

    /// Search does not hide a note that was later superseded, the same
    /// choice `notes_for` makes -- see `search_notes`'s doc for why filtering
    /// it out here would not actually close the gap it looks like it would
    /// close. Proven against what the database hands back, not asserted in
    /// prose: both notes come back when both match, `TaskNote::supersedes`
    /// still names the one the newer note replaced, and `NoteHit::superseded`
    /// tells a reader which of the two is the stale one without them having
    /// to notice the link themselves.
    #[test]
    fn search_does_not_hide_a_note_that_was_later_superseded() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let first = store
            .add_note(task.id, NoteKind::Decision, Actor::Manager, "sqlite over files", json!({}))
            .unwrap();
        let second = store
            .add_note_superseding(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "sqlite, but see the earlier note about files",
                json!({}),
                first.id,
            )
            .unwrap();

        let hits = store.search_notes(repo(), "sqlite", None).unwrap();
        assert_eq!(hits.len(), 2, "the retracted decision is still a hit, same as notes_for");
        assert_eq!(hits[0].note.id, first.id);
        assert_eq!(hits[0].note.supersedes, None);
        assert!(hits[0].superseded, "the second note replaced this one");
        assert_eq!(hits[1].note.id, second.id);
        assert_eq!(
            hits[1].note.supersedes,
            Some(first.id),
            "the link a reader would need to notice the retraction without the flag"
        );
        assert!(!hits[1].superseded, "nothing has replaced the second note");
    }

    /// The coordinator's ruling, proven directly: `search_notes` flags a hit
    /// as superseded by inspecting `task_notes.supersedes` for ANY row that
    /// names it, not by checking whether the retracting note's own body
    /// matches `query`. So this is the case
    /// `search_does_not_hide_a_note_that_was_later_superseded` above cannot
    /// exercise -- there, both notes happen to contain "sqlite," which would
    /// let a wrong implementation that only flags a hit when the retracting
    /// note ALSO matches the query pass anyway. Here the retracting note
    /// deliberately shares no words with the query, so only a real
    /// `EXISTS` over the unfiltered table gets this right.
    #[test]
    fn search_flags_a_hit_that_was_later_superseded_even_when_the_retracting_note_does_not_match() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();
        let first = store
            .add_note(task.id, NoteKind::Decision, Actor::Manager, "use sqlite", json!({}))
            .unwrap();
        store
            .add_note_superseding(
                task.id,
                NoteKind::Decision,
                Actor::Manager,
                "use files after all",
                json!({}),
                first.id,
            )
            .unwrap();

        let hits = store.search_notes(repo(), "sqlite", None).unwrap();
        assert_eq!(hits.len(), 1, "only the original decision's body matches \"sqlite\"");
        assert_eq!(hits[0].note.id, first.id);
        assert!(
            hits[0].superseded,
            "flagged as revised even though the retracting note never matched the query"
        );
    }

    /// The two-hit ordering `search_does_not_hide_a_note_that_was_later_superseded`
    /// happens to exercise is not adversarial: every note built through
    /// `add_note` alone has `at`, `rowid`, and its UUIDv7 `id` all increase
    /// together, so nothing in this file's other fixtures can tell
    /// `search_notes`'s documented "oldest first" (`ORDER BY tn.at,
    /// tn.rowid`) apart from an implementation that instead orders by
    /// `rowid` or by `id`. `insert_note_with_at_for_test` breaks that
    /// correlation on purpose: `second_inserted` is written after
    /// `first_inserted` (so its `rowid` and `id` both say it came later),
    /// but carries the EARLIER `at`. Only a real sort on `at` gets the order
    /// right.
    #[test]
    fn search_orders_by_at_not_by_insertion_order() {
        let store = seeded();
        let task = store.create_task(repo(), "t", Actor::User).unwrap();

        let first_inserted =
            store.insert_note_with_at_for_test(task.id, NoteKind::Finding, "sqlite alpha", 2_000);
        let second_inserted =
            store.insert_note_with_at_for_test(task.id, NoteKind::Finding, "sqlite beta", 1_000);

        let hits = store.search_notes(repo(), "sqlite", None).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].note.id, second_inserted,
            "the earlier `at` sorts first, even though it was written to the table second"
        );
        assert_eq!(hits[1].note.id, first_inserted);
    }

    /// The failure mode of the whole arrangement is not an agent doing the
    /// wrong thing. It is a task sitting in `todo` that you assumed was in
    /// flight.
    #[test]
    fn a_task_that_has_not_moved_can_be_found_by_how_long_it_has_sat() {
        let store = seeded();
        let old = store.create_task(repo(), "forgotten", Actor::User).unwrap();
        store.backdate_status_since_for_test(old.id, Duration::from_secs(3 * 86_400));
        store.create_task(repo(), "fresh", Actor::User).unwrap();

        let stale = store.list_tasks_stale_for(repo(), Duration::from_secs(86_400)).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, old.id);
    }

    /// One stale task, above, proves a task can be found this way at all --
    /// it says nothing about the order several of them come back in. An
    /// implementation with no `ORDER BY`, or one ordered by `created_at`
    /// instead of `status_since`, passes every test above and would still
    /// hand a manager the wrong task first, which defeats the point of a
    /// staleness view: surfacing the worst stall first. Creation order here
    /// is deliberately the OPPOSITE of staleness order -- `less_stale` is
    /// created first but backdated less -- so an implementation ordering by
    /// `created_at` returns these two rows swapped.
    #[test]
    fn several_stale_tasks_sort_worst_stall_first() {
        let store = seeded();
        let less_stale = store.create_task(repo(), "less stale", Actor::User).unwrap();
        store.backdate_status_since_for_test(less_stale.id, Duration::from_secs(2 * 86_400));
        let more_stale = store.create_task(repo(), "more stale", Actor::User).unwrap();
        store.backdate_status_since_for_test(more_stale.id, Duration::from_secs(5 * 86_400));

        let stale = store.list_tasks_stale_for(repo(), Duration::from_secs(86_400)).unwrap();
        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].id, more_stale.id, "the one that has sat longest sorts first");
        assert_eq!(stale[1].id, less_stale.id);
    }

    /// Done and cancelled tasks sit still forever and are not stale, they are
    /// finished. A staleness view that lists every completed task is a view
    /// nobody reads.
    #[test]
    fn finished_tasks_are_never_stale() {
        let store = seeded();
        let t = store.create_task(repo(), "shipped", Actor::User).unwrap();
        store.set_task_status(t.id, TaskStatus::Done, Actor::Manager).unwrap();
        store.backdate_status_since_for_test(t.id, Duration::from_secs(30 * 86_400));
        assert!(store.list_tasks_stale_for(repo(), Duration::from_secs(86_400)).unwrap().is_empty());
    }

    /// The brief's own fixture above only exercises `done`. The
    /// implementation excludes `status NOT IN ('done', 'cancelled')`
    /// together, and a `done`-only fixture cannot tell that apart from an
    /// implementation that excludes `done` alone and lets a stale, abandoned,
    /// cancelled task keep showing up on the board forever.
    #[test]
    fn cancelled_tasks_are_never_stale_either() {
        let store = seeded();
        let t = store.create_task(repo(), "abandoned", Actor::User).unwrap();
        store.set_task_status(t.id, TaskStatus::Cancelled, Actor::Manager).unwrap();
        store.backdate_status_since_for_test(t.id, Duration::from_secs(30 * 86_400));
        assert!(store.list_tasks_stale_for(repo(), Duration::from_secs(86_400)).unwrap().is_empty());
    }

    /// Staleness is scoped to one repository, for the same reason
    /// `search_does_not_leak_across_repositories` exists: the brief's own
    /// fixture never registers a second repository, so it cannot distinguish
    /// a scoped query from one that stale-checks the whole runner.
    #[test]
    fn staleness_does_not_leak_across_repositories() {
        let store = seeded();
        let other = store.register_repository_for_test("Other Thing");
        let theirs = store.create_task(other, "theirs, forgotten", Actor::User).unwrap();
        store.backdate_status_since_for_test(theirs.id, Duration::from_secs(3 * 86_400));

        let ours_stale = store.list_tasks_stale_for(repo(), Duration::from_secs(86_400)).unwrap();
        assert!(ours_stale.is_empty(), "another repository's stale task must not appear in ours");
    }
}

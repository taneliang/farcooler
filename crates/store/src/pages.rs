//! Orchestrator pages (ov-269, ov-282): a document per slot per workspace,
//! beside the board and never in it.
//!
//! # Additive and removable
//!
//! Two tables of their own. `board_pages` points at `workspaces` by cascade and
//! at nothing else: a page names a card, a lane or a theme by text, and its
//! theme anchor is the theme's id kept as text with no foreign key, because a
//! key into the plan layer would make removing it cascade into pages. Nothing
//! in `tasks.rs`, `waits.rs`, `workers.rs`, `workspaces.rs` or `plan.rs` names
//! these tables (`scripts/page-layer-lint.py` fails CI if one does), and the
//! drill in `pages_tests.rs` drops both from a populated database and checks
//! every board read and every plan read is the same bytes.
//!
//! A page's history is `page_events`, which keeps no content: when, who, how
//! big, and the page's shape (block and reference counts), so which shapes
//! recur can be measured without reading anyone's page.
//!
//! # What the store enforces
//!
//! The document arrives validated (`farcooler_core::page_doc`). The store holds
//! the rules that need the board: twelve pages per workspace, three anchored to
//! a theme, and no more than thirty changing writes to one slot in an hour.

use std::collections::{BTreeMap, HashSet};

use rusqlite::{OptionalExtension, Transaction, params};
use uuid::Uuid;

use farcooler_core::page_doc::{MAX_ANCHORED_PER_THEME, MAX_PAGES, MAX_WRITES_PER_HOUR, Page, SLOT_MAX, valid_slot};
use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, get_uuid, uuid_blob};
use crate::store::Store;
use crate::tasks::now_millis;

/// Two new tables that only this file touches.
///
/// `Older::Welcome`: a build from before them reads every table it knows
/// exactly as it did, and their rows go with their workspace by cascade
/// whichever build deletes it. No trigger on an existing table and no new
/// column on one.
pub(crate) fn migration_0025_pages(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE board_pages (
            id BLOB PRIMARY KEY NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            slot TEXT NOT NULL,
            -- Copied out of the document so the list doesn't parse every page.
            title TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            -- '' or 'theme', and the theme's id as text. No foreign key: a
            -- page must outlive the plan layer.
            anchor_kind TEXT NOT NULL DEFAULT '',
            anchor TEXT NOT NULL DEFAULT '',
            doc_json TEXT NOT NULL,
            schema_v INTEGER NOT NULL,
            bytes INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            ordinal INTEGER NOT NULL,
            actor TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            resource_version INTEGER NOT NULL,
            UNIQUE (workspace_id, slot)
        );

        CREATE TABLE page_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            at INTEGER NOT NULL,
            workspace_id BLOB NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            slot TEXT NOT NULL,
            actor TEXT NOT NULL,
            kind TEXT NOT NULL,
            bytes INTEGER NOT NULL,
            revision INTEGER NOT NULL,
            shape TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX page_events_by_slot ON page_events (workspace_id, slot, at);
        "#,
    )
}

const ANCHOR_THEME: &str = "theme";

/// A stored page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPage {
    /// Its id.
    pub id: Uuid,
    /// The workspace it belongs to.
    pub workspace_id: Uuid,
    /// Its name within the workspace.
    pub slot: String,
    /// The document's title.
    pub title: String,
    /// The document's summary.
    pub summary: String,
    /// `""`, or `"theme"`.
    pub anchor_kind: String,
    /// The theme's id as text when anchored, else `""`.
    pub anchor: String,
    /// The validated, normalized document.
    pub doc_json: String,
    /// The document's schema version.
    pub schema_v: u32,
    /// The document's size in bytes.
    pub bytes: u32,
    /// Counts each write that changed the page, from 1.
    pub revision: u64,
    /// Where it sorts in the list.
    pub ordinal: u32,
    /// Who wrote it last.
    pub actor: String,
    /// When it was last changed, in Unix milliseconds.
    pub updated_at: i64,
    /// When its slot was first written.
    pub created_at: i64,
    /// Moves with every change, as on a task.
    pub resource_version: u64,
}

/// What a write does to the page's anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// Leave it as it is: none for a new page.
    Keep,
    /// Not anchored.
    Clear,
    /// Drawn as a section of this theme, by its id as text.
    Theme(String),
}

/// A page to publish.
#[derive(Debug, Clone)]
pub struct PageWrite<'a> {
    /// The slot, which replaces what's there.
    pub slot: &'a str,
    /// The validated document.
    pub page: &'a Page,
    /// The anchor.
    pub anchor: Anchor,
    /// Where it sorts: the end of the list when new, unchanged otherwise.
    pub ordinal: Option<u32>,
    /// Refuse when the page's revision isn't this (0 for a slot with no page).
    pub if_revision: Option<u64>,
}

/// What a write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// The same bytes, anchor and place: nothing was written and no event
    /// should be sent.
    Unchanged(StoredPage),
    /// The page changed, or is new.
    Written(StoredPage),
}

impl SetOutcome {
    /// The page as it stands.
    pub fn page(&self) -> &StoredPage {
        match self {
            SetOutcome::Unchanged(p) | SetOutcome::Written(p) => p,
        }
    }
}

/// How one slot has been published, from `page_events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotStats {
    /// The slot.
    pub slot: String,
    /// Writes that changed it.
    pub sets: u32,
    /// Times it was removed.
    pub removes: u32,
    /// The earliest event counted.
    pub first_at: i64,
    /// The latest event counted.
    pub last_at: i64,
    /// Block and reference counts summed over its writes, by the names
    /// `Page::shape` uses (`table`, `ref-lane`).
    pub shape: BTreeMap<String, u32>,
}

const PAGE_COLS: &str = "id, workspace_id, slot, title, summary, anchor_kind, anchor, doc_json, schema_v, bytes, revision, \
     ordinal, actor, updated_at, created_at, resource_version";

fn row_to_page(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPage> {
    Ok(StoredPage {
        id: get_uuid(r, 0)?,
        workspace_id: get_uuid(r, 1)?,
        slot: r.get(2)?,
        title: r.get(3)?,
        summary: r.get(4)?,
        anchor_kind: r.get(5)?,
        anchor: r.get(6)?,
        doc_json: r.get(7)?,
        schema_v: r.get(8)?,
        bytes: r.get(9)?,
        revision: r.get::<_, i64>(10)? as u64,
        ordinal: r.get(11)?,
        actor: r.get(12)?,
        updated_at: r.get(13)?,
        created_at: r.get(14)?,
        resource_version: r.get::<_, i64>(15)? as u64,
    })
}

fn refused(said: impl Into<String>) -> DomainError {
    DomainError::PageRefused { said: said.into() }
}

fn workspace_exists(tx: &Transaction, workspace: Uuid) -> Result<()> {
    tx.query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |_| Ok(()))
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)
}

fn page_in(tx: &Transaction, workspace: Uuid, slot: &str) -> Result<Option<StoredPage>> {
    tx.query_row(
        &format!("SELECT {PAGE_COLS} FROM board_pages WHERE workspace_id = ?1 AND slot = ?2"),
        params![uuid_blob(workspace), slot],
        row_to_page,
    )
    .optional()
    .map_err(map_err)
}

fn count(tx: &Transaction, sql: &str, args: impl rusqlite::Params) -> Result<usize> {
    let n: i64 = tx.query_row(sql, args, |r| r.get(0)).map_err(map_err)?;
    Ok(n as usize)
}

impl Store {
    /// Every page on a board, in list order.
    pub fn list_pages(&self, workspace: Uuid) -> Result<Vec<StoredPage>> {
        let conn = self.conn();
        let exists = conn
            .query_row("SELECT 1 FROM workspaces WHERE id = ?1", params![uuid_blob(workspace)], |_| Ok(()))
            .optional()
            .map_err(map_err)?;
        if exists.is_none() {
            return Err(DomainError::NotFound);
        }
        let mut stmt = conn
            .prepare(&format!("SELECT {PAGE_COLS} FROM board_pages WHERE workspace_id = ?1 ORDER BY ordinal, slot"))
            .map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace)], row_to_page).map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }

    /// One page by slot.
    pub fn get_page(&self, workspace: Uuid, slot: &str) -> Result<StoredPage> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        page_in(&tx, workspace, slot)?.ok_or(DomainError::NotFound)
    }

    /// Publish a page: replace the slot whole.
    pub fn set_page(&self, workspace: Uuid, write: &PageWrite<'_>, actor: Actor) -> Result<SetOutcome> {
        self.set_page_at(workspace, write, actor, now_millis())
    }

    /// `set_page` against a given clock, so the hourly cap can be tested.
    pub(crate) fn set_page_at(&self, workspace: Uuid, write: &PageWrite<'_>, actor: Actor, now: i64) -> Result<SetOutcome> {
        if !valid_slot(write.slot) {
            return Err(refused(format!(
                "A page's slot is lowercase letters, digits and hyphens, at most {SLOT_MAX} characters."
            )));
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        workspace_exists(&tx, workspace)?;
        let existing = page_in(&tx, workspace, write.slot)?;
        if let Some(want) = write.if_revision {
            let have = existing.as_ref().map_or(0, |p| p.revision);
            if want != have {
                return Err(DomainError::ResourceConflict);
            }
        }
        let doc_json = write.page.to_json();
        let (anchor_kind, anchor) = match (&write.anchor, &existing) {
            (Anchor::Theme(id), _) => (ANCHOR_THEME.to_string(), id.clone()),
            (Anchor::Clear, _) | (Anchor::Keep, None) => (String::new(), String::new()),
            (Anchor::Keep, Some(p)) => (p.anchor_kind.clone(), p.anchor.clone()),
        };
        let ordinal = match (write.ordinal, &existing) {
            (Some(o), _) => o,
            (None, Some(p)) => p.ordinal,
            (None, None) => {
                let top: Option<i64> = tx
                    .query_row("SELECT MAX(ordinal) FROM board_pages WHERE workspace_id = ?1", params![uuid_blob(workspace)], |r| r.get(0))
                    .map_err(map_err)?;
                top.map_or(0, |t| t as u32 + 1)
            }
        };
        if let Some(p) = &existing {
            if p.doc_json == doc_json && p.anchor == anchor && p.ordinal == ordinal {
                return Ok(SetOutcome::Unchanged(p.clone()));
            }
        }
        if existing.is_none() {
            let pages = count(&tx, "SELECT count(*) FROM board_pages WHERE workspace_id = ?1", params![uuid_blob(workspace)])?;
            if pages >= MAX_PAGES {
                return Err(refused(format!(
                    "A workspace has at most {MAX_PAGES} pages. Remove one with page rm, then publish this one."
                )));
            }
        }
        if !anchor.is_empty() {
            let there = count(
                &tx,
                "SELECT count(*) FROM board_pages WHERE workspace_id = ?1 AND anchor = ?2 AND slot <> ?3",
                params![uuid_blob(workspace), anchor, write.slot],
            )?;
            if there >= MAX_ANCHORED_PER_THEME {
                return Err(refused(format!(
                    "A theme draws at most {MAX_ANCHORED_PER_THEME} pages. Anchor this one to another theme, or free a slot."
                )));
            }
        }
        let recent = count(
            &tx,
            "SELECT count(*) FROM page_events WHERE workspace_id = ?1 AND slot = ?2 AND kind = 'set' AND at > ?3",
            params![uuid_blob(workspace), write.slot, now - 3_600_000],
        )?;
        if recent >= MAX_WRITES_PER_HOUR {
            return Err(refused(format!(
                "This page changed {MAX_WRITES_PER_HOUR} times in the last hour. Pages are for checkpoints, not a live log."
            )));
        }
        let bytes = doc_json.len() as u32;
        let (id, revision) = match &existing {
            Some(p) => {
                let revision = p.revision + 1;
                tx.execute(
                    "UPDATE board_pages SET title = ?1, summary = ?2, anchor_kind = ?3, anchor = ?4, doc_json = ?5, \
                     schema_v = ?6, bytes = ?7, revision = ?8, ordinal = ?9, actor = ?10, updated_at = ?11, \
                     resource_version = resource_version + 1 WHERE id = ?12",
                    params![
                        write.page.title,
                        write.page.summary,
                        anchor_kind,
                        anchor,
                        doc_json,
                        write.page.v,
                        bytes,
                        revision as i64,
                        ordinal,
                        actor.to_string(),
                        now,
                        uuid_blob(p.id)
                    ],
                )
                .map_err(map_err)?;
                (p.id, revision)
            }
            None => {
                let id = Uuid::now_v7();
                tx.execute(
                    "INSERT INTO board_pages (id, workspace_id, slot, title, summary, anchor_kind, anchor, doc_json, \
                     schema_v, bytes, revision, ordinal, actor, updated_at, created_at, resource_version) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?13, ?13, 1)",
                    params![
                        uuid_blob(id),
                        uuid_blob(workspace),
                        write.slot,
                        write.page.title,
                        write.page.summary,
                        anchor_kind,
                        anchor,
                        doc_json,
                        write.page.v,
                        bytes,
                        ordinal,
                        actor.to_string(),
                        now
                    ],
                )
                .map_err(map_err)?;
                (id, 1)
            }
        };
        tx.execute(
            "INSERT INTO page_events (at, workspace_id, slot, actor, kind, bytes, revision, shape) \
             VALUES (?1, ?2, ?3, ?4, 'set', ?5, ?6, ?7)",
            params![now, uuid_blob(workspace), write.slot, actor.to_string(), bytes, revision as i64, write.page.shape()],
        )
        .map_err(map_err)?;
        let stored = tx
            .query_row(&format!("SELECT {PAGE_COLS} FROM board_pages WHERE id = ?1"), params![uuid_blob(id)], row_to_page)
            .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(SetOutcome::Written(stored))
    }

    /// Remove a page and log it. `NotFound` when the slot has none.
    pub fn remove_page(&self, workspace: Uuid, slot: &str, actor: Actor) -> Result<StoredPage> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let page = page_in(&tx, workspace, slot)?.ok_or(DomainError::NotFound)?;
        tx.execute("DELETE FROM board_pages WHERE id = ?1", params![uuid_blob(page.id)]).map_err(map_err)?;
        tx.execute(
            "INSERT INTO page_events (at, workspace_id, slot, actor, kind, bytes, revision, shape) \
             VALUES (?1, ?2, ?3, ?4, 'remove', 0, ?5, '')",
            params![now_millis(), uuid_blob(workspace), slot, actor.to_string(), page.revision as i64],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(page)
    }

    /// How each slot has been published since `since_ms`, most-written first.
    /// What `page stats` prints; it reads shapes, never content.
    pub fn page_stats(&self, workspace: Uuid, since_ms: i64) -> Result<Vec<SlotStats>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT slot, at, kind, shape FROM page_events WHERE workspace_id = ?1 AND at >= ?2 ORDER BY at")
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(workspace), since_ms], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
            })
            .map_err(map_err)?;
        let mut by_slot: BTreeMap<String, SlotStats> = BTreeMap::new();
        for row in rows {
            let (slot, at, kind, shape) = row.map_err(map_err)?;
            let stats = by_slot.entry(slot.clone()).or_insert_with(|| SlotStats {
                slot,
                sets: 0,
                removes: 0,
                first_at: at,
                last_at: at,
                shape: BTreeMap::new(),
            });
            stats.last_at = at;
            if kind == "set" {
                stats.sets += 1;
                for token in shape.split_whitespace() {
                    if let Some((name, n)) = token.rsplit_once(':') {
                        *stats.shape.entry(name.to_string()).or_insert(0) += n.parse::<u32>().unwrap_or(0);
                    }
                }
            } else {
                stats.removes += 1;
            }
        }
        let mut out: Vec<SlotStats> = by_slot.into_values().collect();
        out.sort_by(|a, b| b.sets.cmp(&a.sets).then(a.slot.cmp(&b.slot)));
        Ok(out)
    }

    /// The keys of the cards on a board, so a page's references can be checked
    /// against it. Read-only, and by name: nothing here is stored on the card.
    pub fn page_card_keys(&self, workspace: Uuid) -> Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key FROM tasks WHERE workspace_id = ?1").map_err(map_err)?;
        let rows = stmt.query_map(params![uuid_blob(workspace)], |r| r.get::<_, String>(0)).map_err(map_err)?;
        let keys = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)?;
        Ok(keys.into_iter().map(|k| k.to_ascii_lowercase()).collect())
    }
}

#[cfg(test)]
#[path = "pages_tests.rs"]
mod tests;

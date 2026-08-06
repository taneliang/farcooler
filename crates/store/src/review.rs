//! The review buffer, and the outbox that keeps dispatch honest.
//!
//! A review entry is one thought a reviewer had. Most of them are not about a
//! line — more than half, by the only measured workflow we have — so the anchor
//! is optional and the schema treats "no anchor" as ordinary rather than
//! degenerate.
//!
//! ## Why the outbox
//!
//! Sending fourteen comments to an agent is not a database write. The daemon can
//! die between marking the entries dispatched and the agent receiving anything,
//! and ACP offers no acknowledgement to tell the two apart: `session/prompt` is
//! sent without waiting, and its response means end-of-turn, not receipt.
//!
//! So the row is written in the same transaction that marks the entries, and it
//! moves to `Observed` only when the daemon SEES the prompt in that terminal's
//! own event stream — positive evidence, not an inference. On restart, anything
//! still `Pending` becomes `Unknown`, and the user is asked. Never resent
//! automatically: a crash before observation is indistinguishable from a crash
//! before the write, so an automatic resend can hand an agent the same fourteen
//! instructions twice, and no resend can lose them.
//!
//! ```text
//!   dispatch ──> Pending ──see the prompt in the stream──> Observed
//!                   │
//!                   └──restart, or terminal gone──> Unknown ──> "Send Again"
//! ```

use farcooler_core::{DomainError, Result};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::error::map_err;
use crate::models::{get_uuid, uuid_blob};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Fix this. Goes to a terminal as an instruction.
    Fix,
    /// Answer this. Goes to a terminal as a question, and the answer comes back
    /// beside the entry without touching code.
    Ask,
    /// Neither. A note to self that never leaves the buffer.
    Note,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Disposition::Fix => "fix",
            Disposition::Ask => "ask",
            Disposition::Note => "note",
        }
    }
    pub fn parse(s: &str) -> Option<Disposition> {
        match s {
            "fix" => Some(Disposition::Fix),
            "ask" => Some(Disposition::Ask),
            "note" => Some(Disposition::Note),
            _ => None,
        }
    }
}

/// Where the entry is in the USER's workflow.
///
/// Not where it is relative to the code — that is derived on every read. See the
/// migration note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStatus {
    Open,
    Dispatched,
    /// A question that came back with an answer.
    Answered,
    /// The user is done with it.
    Resolved,
    /// It was dispatched, and the daemon cannot prove the agent ever saw it.
    DispatchUnknown,
}

impl EntryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryStatus::Open => "open",
            EntryStatus::Dispatched => "dispatched",
            EntryStatus::Answered => "answered",
            EntryStatus::Resolved => "resolved",
            EntryStatus::DispatchUnknown => "dispatch_unknown",
        }
    }
    pub fn parse(s: &str) -> Option<EntryStatus> {
        match s {
            "open" => Some(EntryStatus::Open),
            "dispatched" => Some(EntryStatus::Dispatched),
            "answered" => Some(EntryStatus::Answered),
            "resolved" => Some(EntryStatus::Resolved),
            "dispatch_unknown" => Some(EntryStatus::DispatchUnknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchState {
    /// Written, send attempted, nothing observed yet.
    Pending,
    /// The prompt was seen in the target terminal's own event stream.
    Observed,
    /// It cannot be established either way. The user decides.
    Unknown,
}

impl DispatchState {
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchState::Pending => "pending",
            DispatchState::Observed => "observed",
            DispatchState::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<DispatchState> {
        match s {
            "pending" => Some(DispatchState::Pending),
            "observed" => Some(DispatchState::Observed),
            "unknown" => Some(DispatchState::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEntry {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub body: String,
    pub disposition: Disposition,
    pub status: EntryStatus,
    /// `farcooler_review::Anchor`, as JSON. JSON rather than columns because the
    /// anchor is a tagged union whose arms carry different fields, and the Rust
    /// definition is the one both apps already decode — restating it as a dozen
    /// nullable columns would create a second definition to keep in step.
    pub anchor_json: String,
    /// `farcooler_review::CaptureManifest`, as JSON. Immutable once written.
    pub manifest_json: String,
    pub dispatch_id: Option<Uuid>,
    pub answer_text: Option<String>,
    pub answer_terminal_id: Option<Uuid>,
    pub answer_correlation: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub resource_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub terminal_id: Uuid,
    pub disposition: Disposition,
    pub entry_ids: Vec<Uuid>,
    pub prompt: String,
    pub state: DispatchState,
    pub created_at: i64,
    pub observed_at: Option<i64>,
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<ReviewEntry> {
    Ok(ReviewEntry {
        id: get_uuid(row, 0)?,
        workspace_id: get_uuid(row, 1)?,
        body: row.get(2)?,
        disposition: Disposition::parse(&row.get::<_, String>(3)?).unwrap_or(Disposition::Note),
        status: EntryStatus::parse(&row.get::<_, String>(4)?).unwrap_or(EntryStatus::Open),
        anchor_json: row.get(5)?,
        manifest_json: row.get(6)?,
        dispatch_id: row.get::<_, Option<Vec<u8>>>(7)?.and_then(|b| Uuid::from_slice(&b).ok()),
        answer_text: row.get(8)?,
        answer_terminal_id: row
            .get::<_, Option<Vec<u8>>>(9)?
            .and_then(|b| Uuid::from_slice(&b).ok()),
        answer_correlation: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        resource_version: row.get::<_, i64>(13)? as u64,
    })
}

const ENTRY_COLUMNS: &str = "id, workspace_id, body, disposition, status, anchor_json, \
     manifest_json, dispatch_id, answer_text, answer_terminal_id, answer_correlation, \
     created_at, updated_at, resource_version";

impl Store {
    pub fn capture_review_entry(
        &self,
        workspace_id: Uuid,
        body: &str,
        disposition: Disposition,
        anchor_json: &str,
        manifest_json: &str,
        now_millis: i64,
    ) -> Result<ReviewEntry> {
        let id = Uuid::now_v7();
        self.conn()
            .execute(
                "INSERT INTO review_entries
                 (id, workspace_id, body, disposition, status, anchor_json, manifest_json,
                  created_at, updated_at, resource_version)
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?7, 1)",
                params![
                    uuid_blob(id),
                    uuid_blob(workspace_id),
                    body,
                    disposition.as_str(),
                    anchor_json,
                    manifest_json,
                    now_millis
                ],
            )
            .map_err(map_err)?;
        Ok(ReviewEntry {
            id,
            workspace_id,
            body: body.to_string(),
            disposition,
            status: EntryStatus::Open,
            anchor_json: anchor_json.to_string(),
            manifest_json: manifest_json.to_string(),
            dispatch_id: None,
            answer_text: None,
            answer_terminal_id: None,
            answer_correlation: None,
            created_at: now_millis,
            updated_at: now_millis,
            resource_version: 1,
        })
    }

    pub fn review_entry(&self, id: Uuid) -> Result<Option<ReviewEntry>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!("SELECT {ENTRY_COLUMNS} FROM review_entries WHERE id = ?1"))
            .map_err(map_err)?;
        stmt.query_row(params![uuid_blob(id)], row_to_entry).optional().map_err(map_err)
    }

    pub fn list_review_entries(&self, workspace_id: Uuid) -> Result<Vec<ReviewEntry>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {ENTRY_COLUMNS} FROM review_entries
                 WHERE workspace_id = ?1 ORDER BY created_at ASC"
            ))
            .map_err(map_err)?;
        let rows = stmt
            .query_map(params![uuid_blob(workspace_id)], row_to_entry)
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }

    /// Update an entry's text or disposition, with the usual version
    /// precondition. Anchor and manifest are immutable: an entry that could be
    /// re-anchored after the fact would be a different comment wearing the same
    /// id, and its manifest would no longer describe what the writer saw.
    pub fn update_review_entry(
        &self,
        id: Uuid,
        body: &str,
        disposition: Disposition,
        expected_version: u64,
        now_millis: i64,
    ) -> Result<ReviewEntry> {
        let changed = self
            .conn()
            .execute(
                "UPDATE review_entries
                 SET body = ?2, disposition = ?3, updated_at = ?4,
                     resource_version = resource_version + 1
                 WHERE id = ?1 AND resource_version = ?5",
                params![
                    uuid_blob(id),
                    body,
                    disposition.as_str(),
                    now_millis,
                    expected_version as i64
                ],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(DomainError::ResourceConflict);
        }
        self.review_entry(id)?.ok_or(DomainError::NotFound)
    }

    pub fn delete_review_entry(&self, id: Uuid) -> Result<()> {
        self.conn()
            .execute("DELETE FROM review_entries WHERE id = ?1", params![uuid_blob(id)])
            .map_err(map_err)?;
        Ok(())
    }

    pub fn set_review_entry_status(&self, id: Uuid, status: EntryStatus, now: i64) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE review_entries
                 SET status = ?2, updated_at = ?3, resource_version = resource_version + 1
                 WHERE id = ?1",
                params![uuid_blob(id), status.as_str(), now],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Create the outbox row and mark its entries dispatched, atomically.
    ///
    /// One transaction, because the two halves must not be observable apart: a
    /// marked entry with no outbox row is an entry nobody will ever retry, and
    /// an outbox row with unmarked entries dispatches them twice.
    ///
    /// Every entry's version is checked. Explicit ids stop a NEWLY CAPTURED
    /// entry being swept in; versions stop an entry that was edited, resolved or
    /// already dispatched between the client rendering it and the user pressing
    /// the button. A mismatch refuses the whole dispatch rather than sending an
    /// older wording of somebody's comment.
    #[allow(clippy::too_many_arguments)]
    pub fn open_dispatch(
        &self,
        workspace_id: Uuid,
        terminal_id: Uuid,
        disposition: Disposition,
        entries: &[(Uuid, u64)],
        prompt: &str,
        now_millis: i64,
    ) -> Result<Dispatch> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;

        for (id, expected) in entries {
            let changed = tx
                .execute(
                    "UPDATE review_entries
                     SET status = 'dispatched', updated_at = ?3,
                         resource_version = resource_version + 1
                     WHERE id = ?1 AND resource_version = ?2 AND status IN ('open', 'answered')",
                    params![uuid_blob(*id), *expected as i64, now_millis],
                )
                .map_err(map_err)?;
            if changed == 0 {
                // Rolls back on drop. Nothing was marked, nothing was sent.
                return Err(DomainError::ResourceConflict);
            }
        }

        let id = Uuid::now_v7();
        let ids: Vec<String> = entries.iter().map(|(i, _)| i.to_string()).collect();
        let entry_ids_json = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_string());

        tx.execute(
            "INSERT INTO review_dispatches
             (id, workspace_id, terminal_id, disposition, entry_ids_json, prompt, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                uuid_blob(id),
                uuid_blob(workspace_id),
                uuid_blob(terminal_id),
                disposition.as_str(),
                entry_ids_json,
                prompt,
                now_millis
            ],
        )
        .map_err(map_err)?;

        for (entry_id, _) in entries {
            tx.execute(
                "UPDATE review_entries SET dispatch_id = ?2 WHERE id = ?1",
                params![uuid_blob(*entry_id), uuid_blob(id)],
            )
            .map_err(map_err)?;
        }

        tx.commit().map_err(map_err)?;

        Ok(Dispatch {
            id,
            workspace_id,
            terminal_id,
            disposition,
            entry_ids: entries.iter().map(|(i, _)| *i).collect(),
            prompt: prompt.to_string(),
            state: DispatchState::Pending,
            created_at: now_millis,
            observed_at: None,
        })
    }

    pub fn dispatch(&self, id: Uuid) -> Result<Option<Dispatch>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, terminal_id, disposition, entry_ids_json, prompt,
                        state, created_at, observed_at
                 FROM review_dispatches WHERE id = ?1",
            )
            .map_err(map_err)?;
        stmt.query_row(params![uuid_blob(id)], row_to_dispatch).optional().map_err(map_err)
    }

    /// Every dispatch still `pending`. Read on daemon start, so that nothing is
    /// left claiming to have been delivered when nobody watched it happen.
    pub fn pending_dispatches(&self) -> Result<Vec<Dispatch>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, workspace_id, terminal_id, disposition, entry_ids_json, prompt,
                        state, created_at, observed_at
                 FROM review_dispatches WHERE state = 'pending' ORDER BY created_at ASC",
            )
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], row_to_dispatch)
            .map_err(map_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_err)?;
        Ok(rows)
    }

    /// The prompt was seen in the target terminal's stream.
    pub fn mark_dispatch_observed(&self, id: Uuid, now_millis: i64) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE review_dispatches SET state = 'observed', observed_at = ?2
                 WHERE id = ?1 AND state = 'pending'",
                params![uuid_blob(id), now_millis],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// It cannot be established. Entries move with it, so the user sees
    /// `DispatchUnknown` rather than a confident `Dispatched`.
    pub fn mark_dispatch_unknown(&self, id: Uuid, now_millis: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        tx.execute(
            "UPDATE review_dispatches SET state = 'unknown' WHERE id = ?1 AND state = 'pending'",
            params![uuid_blob(id)],
        )
        .map_err(map_err)?;
        tx.execute(
            "UPDATE review_entries
             SET status = 'dispatch_unknown', updated_at = ?2,
                 resource_version = resource_version + 1
             WHERE dispatch_id = ?1 AND status = 'dispatched'",
            params![uuid_blob(id), now_millis],
        )
        .map_err(map_err)?;
        tx.commit().map_err(map_err)?;
        Ok(())
    }

    pub fn record_answer(
        &self,
        entry_id: Uuid,
        terminal_id: Uuid,
        text: &str,
        correlation: &str,
        now_millis: i64,
    ) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE review_entries
                 SET answer_text = ?2, answer_terminal_id = ?3, answer_correlation = ?4,
                     status = 'answered', updated_at = ?5,
                     resource_version = resource_version + 1
                 WHERE id = ?1",
                params![
                    uuid_blob(entry_id),
                    text,
                    uuid_blob(terminal_id),
                    correlation,
                    now_millis
                ],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Mark a file read AT a content hash. Reading the row back only counts as
    /// viewed when the hash still matches, so an agent editing the file clears
    /// the mark without anything having to notice and delete it.
    pub fn mark_viewed(
        &self,
        workspace_id: Uuid,
        branch: &str,
        path: &str,
        content_hash: &str,
        now_millis: i64,
    ) -> Result<()> {
        self.conn()
            .execute(
                "INSERT INTO review_viewed (workspace_id, branch, path, content_hash, viewed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(workspace_id, branch, path)
                 DO UPDATE SET content_hash = excluded.content_hash, viewed_at = excluded.viewed_at",
                params![uuid_blob(workspace_id), branch, path, content_hash, now_millis],
            )
            .map_err(map_err)?;
        Ok(())
    }

    pub fn is_viewed(
        &self,
        workspace_id: Uuid,
        branch: &str,
        path: &str,
        current_hash: &str,
    ) -> Result<bool> {
        let conn = self.conn();
        let stored: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM review_viewed
                 WHERE workspace_id = ?1 AND branch = ?2 AND path = ?3",
                params![uuid_blob(workspace_id), branch, path],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_err)?;
        Ok(stored.is_some_and(|h| h == current_hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `store::tests::terminals_table_has_no_runtime_state_column`.
    ///
    /// A schema that CAN hold a line number will eventually hold a stale one,
    /// and a comment pointing confidently at code it is not about is the exact
    /// failure this whole design exists to prevent.
    #[test]
    fn review_entries_table_has_no_line_number_column() {
        let s = Store::open_in_memory().unwrap();
        let cols = s.column_names("review_entries");
        for banned in ["line", "line_number", "start_line", "end_line", "old_line", "new_line"] {
            assert!(
                !cols.iter().any(|c| c.eq_ignore_ascii_case(banned)),
                "review_entries must never carry a `{banned}` column: an anchor is resolved \
                 against the worktree on every read, never remembered, found {cols:?}"
            );
        }
    }

    /// `status` is where the USER put the entry. Whether it is outdated or wants
    /// re-reading is computed, and an entry is routinely dispatched AND in need
    /// of a re-read at once — one column could not say both.
    #[test]
    fn review_entries_table_has_no_derived_anchor_state_column() {
        let s = Store::open_in_memory().unwrap();
        let cols = s.column_names("review_entries");
        for banned in ["anchor_state", "outdated", "needs_reread", "ambiguous", "resolved_line"] {
            assert!(
                !cols.iter().any(|c| c.eq_ignore_ascii_case(banned)),
                "review_entries must never carry a `{banned}` column, found {cols:?}"
            );
        }
    }

    /// The rule `push.rs` applies to the relay token, applied to screenshots: a
    /// database copied for support must not carry a picture of whatever was on
    /// screen when somebody wrote a comment.
    #[test]
    fn review_attachments_table_holds_a_hash_and_never_bytes() {
        let s = Store::open_in_memory().unwrap();
        let cols = s.column_names("review_attachments");
        for banned in ["bytes", "data", "blob", "content", "image"] {
            assert!(
                !cols.iter().any(|c| c.eq_ignore_ascii_case(banned)),
                "review_attachments holds a hash and metadata, never bytes, found {cols:?}"
            );
        }
        assert!(cols.iter().any(|c| c == "sha256"), "content addressing is the mechanism");
    }
}

fn row_to_dispatch(row: &rusqlite::Row) -> rusqlite::Result<Dispatch> {
    let ids_json: String = row.get(4)?;
    let entry_ids: Vec<Uuid> = serde_json::from_str::<Vec<String>>(&ids_json)
        .unwrap_or_default()
        .iter()
        .filter_map(|s| Uuid::parse_str(s).ok())
        .collect();
    Ok(Dispatch {
        id: get_uuid(row, 0)?,
        workspace_id: get_uuid(row, 1)?,
        terminal_id: get_uuid(row, 2)?,
        disposition: Disposition::parse(&row.get::<_, String>(3)?).unwrap_or(Disposition::Fix),
        entry_ids,
        prompt: row.get(5)?,
        state: DispatchState::parse(&row.get::<_, String>(6)?).unwrap_or(DispatchState::Unknown),
        created_at: row.get(7)?,
        observed_at: row.get(8)?,
    })
}

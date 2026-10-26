//! The crashes the owner's own phone reported (ov-797), one row per
//! signature.
//!
//! A phone sends `crash.report` on the launch after a crash or hang, once per
//! MetricKit diagnostic. The same bug crashing every launch is one row whose
//! `count` climbs, not a row per crash: the daemon's `crash_reports` keys the
//! row by a digest of the signature, and files one card for it. What is kept
//! is the signature the phone sent, which is all it may send: exception
//! type, code and signal, the faulting thread's top frames, build and OS.
//!
//! **One transaction per report**: the count, a new card with its notes, and
//! a repeat's raised intent land together or not at all, so a phone that
//! retries after a failure neither counts twice nor leaves a card behind.
//!
//! **At most `daily_cap` new signatures a day.** Past it a report counts on
//! one `OVERFLOW` row and files nothing, so no caller, the owner's phone or a
//! process on the runner, can fill a board with cards.

use rusqlite::{OptionalExtension, params};
use serde_json::json;
use uuid::Uuid;

use farcooler_core::{DomainError, Result};

use crate::error::map_err;
use crate::models::{Actor, NoteKind, TaskStatus, strings_to_json, uuid_blob};
use crate::store::Store;
use crate::tasks::{insert_note, next_key_under, now_millis};

/// One new table.
///
/// `Older::Welcome`: a build from before it never names the table, and has
/// no `crash.report` to write it.
pub(crate) fn migration_0061_crash_signatures(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE crash_signatures (
             -- The daemon's digest of the signature, lowercase hex.
             signature TEXT PRIMARY KEY NOT NULL,
             -- The signature as first reported, JSON.
             detail TEXT NOT NULL,
             count INTEGER NOT NULL,
             first_seen INTEGER NOT NULL,
             last_seen INTEGER NOT NULL,
             -- The app build of the latest report.
             last_build TEXT NOT NULL,
             -- The card filed for it, if any. No key into tasks: the record
             -- outlives a card deleted.
             task_id BLOB
         );",
    )
}

/// The row reports past the day's cap count on. No card is filed for it.
pub const OVERFLOW: &str = "overflow";

/// How long the cap's day is.
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// One report, and what to file for it.
pub struct CrashFiling<'a> {
    pub signature: &'a str,
    /// The signature as JSON, kept with a new row.
    pub detail: &'a str,
    pub build: &'a str,
    /// New signatures allowed in a day.
    pub daily_cap: u32,
    /// The board a new signature's card goes on; none files no card.
    pub board: Option<Uuid>,
    /// A new card's title, labels and first Finding note.
    pub title: &'a str,
    pub labels: &'a [String],
    pub finding: &'a str,
    /// The card's intent at a count.
    pub intent: &'a dyn Fn(u32) -> String,
    /// The Finding a closed card gets when the report is from another build
    /// than the last one.
    pub again: &'a str,
}

/// What `record_crash` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecord {
    /// Reports of this signature, the one just recorded included; of
    /// `OVERFLOW`'s when `over_cap`.
    pub count: u32,
    /// The card the signature is on, if any.
    pub task: Option<Uuid>,
    /// Past the day's cap: counted on `OVERFLOW`, nothing filed.
    pub over_cap: bool,
}

impl Store {
    /// Count one report and file what it needs, in one transaction: a new
    /// row and card for a new signature (inside the day's cap), or one more
    /// on the row there and its card's intent, with a Finding when the card
    /// was closed and this is another build.
    pub fn record_crash(&self, f: &CrashFiling<'_>) -> Result<CrashRecord> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let now = now_millis();
        let before: Option<(String, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT last_build, task_id FROM crash_signatures WHERE signature = ?1",
                [f.signature],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_err)?;
        if before.is_none() {
            let today: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM crash_signatures WHERE first_seen > ?1 AND signature != ?2",
                    params![now - DAY_MILLIS, OVERFLOW],
                    |r| r.get(0),
                )
                .map_err(map_err)?;
            if today >= i64::from(f.daily_cap) {
                let count = bump(&tx, OVERFLOW, "{}", f.build, now)?;
                tx.commit().map_err(map_err)?;
                return Ok(CrashRecord { count, task: None, over_cap: true });
            }
        }
        let count = bump(&tx, f.signature, f.detail, f.build, now)?;
        let (previous_build, card) = match before {
            Some((build, task)) => (Some(build), task.and_then(|b| Uuid::from_slice(&b).ok())),
            None => (None, None),
        };
        let status: Option<String> = match card {
            Some(task) => tx
                .query_row("SELECT status FROM tasks WHERE id = ?1", [uuid_blob(task)], |r| r.get(0))
                .optional()
                .map_err(map_err)?,
            None => None,
        };
        let task = match (card, status) {
            (Some(task), Some(status)) => {
                let closed = status == TaskStatus::Done.as_str() || status == TaskStatus::Cancelled.as_str();
                if closed && previous_build.as_deref() != Some(f.build) {
                    insert_note(&tx, task, NoteKind::Finding, Actor::Runner, f.again, &json!({}), None)?;
                }
                tx.execute(
                    "UPDATE tasks SET intent = ?2, edited_at = ?3, resource_version = resource_version + 1 WHERE id = ?1",
                    params![uuid_blob(task), (f.intent)(count), now],
                )
                .map_err(map_err)?;
                Some(task)
            }
            // A new signature, or a card that was deleted: file one.
            _ => match f.board {
                Some(board) => Some(file_in(&tx, f, board, count, now)?),
                None => None,
            },
        };
        tx.commit().map_err(map_err)?;
        Ok(CrashRecord { count, task, over_cap: false })
    }

    /// Every signature, most recently seen first: `(signature, detail,
    /// count, last_build)`. `OVERFLOW` among them once the cap was passed.
    pub fn crash_signatures(&self) -> Result<Vec<(String, String, u32, String)>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT signature, detail, count, last_build FROM crash_signatures ORDER BY last_seen DESC, signature")
            .map_err(map_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, u32::try_from(r.get::<_, i64>(2)?).unwrap_or(u32::MAX), r.get(3)?))
            })
            .map_err(map_err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(map_err)
    }
}

/// One more report of `signature`: a new row at 1 holding `detail`, or one
/// more on the row there, whose `detail` stays as first written.
fn bump(tx: &rusqlite::Transaction, signature: &str, detail: &str, build: &str, now: i64) -> Result<u32> {
    tx.execute(
        "INSERT INTO crash_signatures (signature, detail, count, first_seen, last_seen, last_build)
         VALUES (?1, ?2, 1, ?3, ?3, ?4)
         ON CONFLICT(signature) DO UPDATE SET count = count + 1, last_seen = ?3, last_build = ?4",
        params![signature, detail, now, build],
    )
    .map_err(map_err)?;
    let count: i64 = tx
        .query_row("SELECT count FROM crash_signatures WHERE signature = ?1", [signature], |r| r.get(0))
        .map_err(map_err)?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// A new card on `board` for `f`, its Created and Finding notes, and the
/// row's link to it, inside the report's transaction. In Backlog, as
/// `create_task` files one.
fn file_in(tx: &rusqlite::Transaction, f: &CrashFiling<'_>, board: Uuid, count: u32, now: i64) -> Result<Uuid> {
    let (repository, prefix): (Vec<u8>, String) = tx
        .query_row("SELECT repository_id, task_prefix FROM workspaces WHERE id = ?1", [uuid_blob(board)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .map_err(map_err)?
        .ok_or(DomainError::NotFound)?;
    let key = next_key_under(tx, &prefix).map_err(map_err)?;
    let id = Uuid::now_v7();
    tx.execute(
        "INSERT INTO tasks
         (id, repository_id, workspace_id, key, title, status, status_since, created_at, intent, labels,
          resource_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9, 1)",
        params![
            uuid_blob(id),
            repository,
            uuid_blob(board),
            key,
            f.title,
            TaskStatus::Backlog.as_str(),
            now,
            (f.intent)(count),
            strings_to_json(f.labels),
        ],
    )
    .map_err(map_err)?;
    insert_note(tx, id, NoteKind::Created, Actor::Runner, f.title, &json!({}), None)?;
    insert_note(tx, id, NoteKind::Finding, Actor::Runner, f.finding, &json!({}), None)?;
    tx.execute("UPDATE crash_signatures SET task_id = ?2 WHERE signature = ?1", params![f.signature, uuid_blob(id)])
        .map_err(map_err)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filing<'a>(signature: &'a str, build: &'a str, board: Option<Uuid>, intent: &'a dyn Fn(u32) -> String) -> CrashFiling<'a> {
        CrashFiling {
            signature,
            detail: "{\"first\":true}",
            build,
            daily_cap: 3,
            board,
            title: "iOS: crashes",
            labels: &[],
            finding: "frames",
            intent,
            again: "again",
        }
    }

    /// A first report is a row at 1; a repeat is the same row at 2 keeping
    /// the detail it was first given; another signature is its own row.
    #[test]
    fn a_repeat_counts_on_one_row() {
        let s = Store::open_in_memory().unwrap();
        let intent = |n: u32| format!("seen {n}");
        assert_eq!(s.record_crash(&filing("aa", "100", None, &intent)).unwrap(), CrashRecord { count: 1, task: None, over_cap: false });
        let mut again = filing("aa", "101", None, &intent);
        again.detail = "{\"first\":false}";
        assert_eq!(s.record_crash(&again).unwrap().count, 2);
        s.record_crash(&filing("bb", "101", None, &intent)).unwrap();
        let rows = s.crash_signatures().unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let aa = rows.iter().find(|r| r.0 == "aa").unwrap();
        assert_eq!((aa.1.as_str(), aa.2, aa.3.as_str()), ("{\"first\":true}", 2, "101"));
    }

    /// Past the day's cap a new signature counts on the overflow row and is
    /// given no row or card of its own; a signature already kept still counts.
    #[test]
    fn past_the_cap_new_signatures_overflow() {
        let s = Store::open_in_memory().unwrap();
        let intent = |n: u32| format!("seen {n}");
        for sig in ["a", "b", "c"] {
            assert!(!s.record_crash(&filing(sig, "1", None, &intent)).unwrap().over_cap);
        }
        for n in 1..=2 {
            assert_eq!(s.record_crash(&filing("d", "1", None, &intent)).unwrap(), CrashRecord { count: n, task: None, over_cap: true });
        }
        assert_eq!(s.record_crash(&filing("a", "1", None, &intent)).unwrap().count, 2, "a kept one still counts");
        let names: Vec<String> = s.crash_signatures().unwrap().into_iter().map(|r| r.0).collect();
        assert!(!names.contains(&"d".to_string()) && names.contains(&OVERFLOW.to_string()), "{names:?}");
        assert_eq!(names.len(), 4);
    }

    /// The migration is the 52nd (0061) and `Welcome`.
    #[test]
    fn the_migration_is_welcome() {
        let it = &crate::migrate::MIGRATIONS[51];
        assert!(std::ptr::fn_addr_eq(it.0, migration_0061_crash_signatures as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
        assert_eq!(it.1, crate::compat::Older::Welcome);
        assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 52);
    }
}

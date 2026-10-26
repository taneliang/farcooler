//! The crashes the owner's own phone reported (ov-797), one row per
//! signature.
//!
//! A phone sends `crash.report` on the launch after a crash or hang, once per
//! MetricKit diagnostic. The same bug crashing every launch is one row whose
//! `count` climbs, not a row per crash: the daemon's `crash_reports` keys the
//! row by a digest of the signature, and files one card for it. What is kept
//! is the signature the phone sent, which is all it may send: exception
//! type, code and signal, the faulting thread's top frames, build and OS.

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::uuid_blob;
use crate::store::Store;
use crate::tasks::now_millis;

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

/// One signature's row, as `record_crash` left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecord {
    /// Reports of this signature, the one just recorded included.
    pub count: u32,
    /// The build of the report before this one, `None` on the first.
    pub previous_build: Option<String>,
    /// The card filed for it.
    pub task: Option<Uuid>,
}

impl Store {
    /// Count one report of `signature`: a new row at 1 holding `detail`, or
    /// one more on the row there, whose `detail` is kept as first written.
    /// One transaction, so two reports at once count two.
    pub fn record_crash(&self, signature: &str, detail: &str, build: &str) -> Result<CrashRecord> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let now = now_millis();
        let before: Option<(String, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT last_build, task_id FROM crash_signatures WHERE signature = ?1",
                [signature],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_err)?;
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
        tx.commit().map_err(map_err)?;
        let (previous_build, task) = match before {
            Some((build, task)) => (Some(build), task.and_then(|b| Uuid::from_slice(&b).ok())),
            None => (None, None),
        };
        Ok(CrashRecord { count: u32::try_from(count).unwrap_or(u32::MAX), previous_build, task })
    }

    /// Note the card filed for `signature`.
    pub fn set_crash_task(&self, signature: &str, task: Uuid) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE crash_signatures SET task_id = ?2 WHERE signature = ?1",
                params![signature, uuid_blob(task)],
            )
            .map_err(map_err)?;
        Ok(())
    }

    /// Every signature, most recently seen first: `(signature, detail,
    /// count, last_build)`.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A first report is a row at 1 with no build before it; a repeat is the
    /// same row at 2, naming the build it last saw and keeping the detail it
    /// was first given; another signature is its own row.
    #[test]
    fn a_repeat_counts_on_one_row() {
        let s = Store::open_in_memory().unwrap();
        let first = s.record_crash("aa", "{\"first\":true}", "100").unwrap();
        assert_eq!(first, CrashRecord { count: 1, previous_build: None, task: None });
        let task = Uuid::now_v7();
        s.set_crash_task("aa", task).unwrap();
        let again = s.record_crash("aa", "{\"first\":false}", "101").unwrap();
        assert_eq!(again, CrashRecord { count: 2, previous_build: Some("100".into()), task: Some(task) });
        s.record_crash("bb", "{}", "101").unwrap();
        let rows = s.crash_signatures().unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let aa = rows.iter().find(|r| r.0 == "aa").unwrap();
        assert_eq!((aa.1.as_str(), aa.2, aa.3.as_str()), ("{\"first\":true}", 2, "101"));
    }
}

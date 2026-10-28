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
//!
//! **One signature, several places.** The signature is coarse (the daemon
//! keys it on the exception and the frames' binaries), so two bugs that
//! crash the same way share a row. Each report also names its `place`, the
//! build and the app's own frames' offsets, and a place the row hasn't seen
//! adds one Finding with that report's frames to its card: "same shape,
//! another place" while it is open, "came back" once it is closed. At most
//! `FINDINGS_PER_DAY` of the runner's Findings land on one card a day.

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
             task_id BLOB,
             -- Every place it was reported from, JSON: the build and the
             -- app's own frames' offsets, as the daemon spells them.
             places TEXT NOT NULL DEFAULT '[]'
         );",
    )
}

/// The row reports past the day's cap count on. No card is filed for it.
pub const OVERFLOW: &str = "overflow";

/// How long the cap's day is.
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// The most Findings the runner adds to one crash card in a day, the first
/// one included.
pub const FINDINGS_PER_DAY: i64 = 3;

/// The most places a row keeps. Past it the oldest goes, so a place not
/// reported for a long while may add a Finding again.
pub const MAX_PLACES: usize = 50;

/// One report, and what to file for it.
pub struct CrashFiling<'a> {
    pub signature: &'a str,
    /// The signature as JSON, kept with a new row.
    pub detail: &'a str,
    pub build: &'a str,
    /// Where it was reported from: its build and the app's own frames'
    /// offsets. A place the row hasn't seen adds a Finding.
    pub place: &'a str,
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
    /// The Finding an open card gets for a report from a new place.
    pub elsewhere: &'a str,
    /// The Finding a closed card gets for a report from a new place.
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
    /// on the row there and its card's intent. A report from a place the row
    /// hasn't kept adds one Finding with its frames (`elsewhere` on an open
    /// card, `again` on a closed one) while the card has had fewer than
    /// `FINDINGS_PER_DAY` of the runner's Findings in the last day, and the
    /// place is kept only when that Finding is written: a place turned away
    /// by the cap gets its Finding from its next report after the cap frees.
    /// A new card's first Finding keeps its report's place the same way.
    pub fn record_crash(&self, f: &CrashFiling<'_>) -> Result<CrashRecord> {
        self.record_crash_at(f, now_millis())
    }

    /// `record_crash` with the clock given, so a test can step past a day.
    /// Notes are still written at the real time.
    pub(crate) fn record_crash_at(&self, f: &CrashFiling<'_>, now: i64) -> Result<CrashRecord> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        let before: Option<(String, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT places, task_id FROM crash_signatures WHERE signature = ?1",
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
        let (mut places, card): (Vec<String>, _) = match before {
            Some((places, task)) => {
                (serde_json::from_str(&places).unwrap_or_default(), task.and_then(|b| Uuid::from_slice(&b).ok()))
            }
            None => (Vec::new(), None),
        };
        let new_place = !places.iter().any(|p| p == f.place);
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
                if new_place && findings_today(&tx, task, now)? < FINDINGS_PER_DAY {
                    let body = if closed { f.again } else { f.elsewhere };
                    insert_note(&tx, task, NoteKind::Finding, Actor::Runner, body, &json!({}), None)?;
                    keep_place(&tx, f, &mut places)?;
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
                Some(board) => {
                    let task = file_in(&tx, f, board, count, now)?;
                    keep_place(&tx, f, &mut places)?;
                    Some(task)
                }
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

/// Keep `f`'s place among `places`, the oldest dropped past `MAX_PLACES`.
fn keep_place(tx: &rusqlite::Transaction, f: &CrashFiling<'_>, places: &mut Vec<String>) -> Result<()> {
    places.push(f.place.to_string());
    if places.len() > MAX_PLACES {
        places.drain(..places.len() - MAX_PLACES);
    }
    tx.execute(
        "UPDATE crash_signatures SET places = ?2 WHERE signature = ?1",
        params![f.signature, serde_json::to_string(&places).unwrap_or_else(|_| "[]".into())],
    )
    .map_err(map_err)?;
    Ok(())
}

/// The runner's Findings on `task` in the day before `now`.
fn findings_today(tx: &rusqlite::Transaction, task: Uuid, now: i64) -> Result<i64> {
    tx.query_row(
        "SELECT COUNT(*) FROM task_notes WHERE task_id = ?1 AND kind = ?2 AND actor = ?3 AND at > ?4",
        params![uuid_blob(task), NoteKind::Finding.as_str(), Actor::Runner.to_string(), now - DAY_MILLIS],
        |r| r.get(0),
    )
    .map_err(map_err)
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
            place: build,
            daily_cap: 3,
            board,
            title: "iOS: crashes",
            labels: &[],
            finding: "frames",
            intent,
            elsewhere: "elsewhere",
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

    fn places(s: &Store, signature: &str) -> Vec<String> {
        let raw: String = s
            .conn()
            .query_row("SELECT places FROM crash_signatures WHERE signature = ?1", [signature], |r| r.get(0))
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    fn findings(s: &Store, task: Uuid) -> usize {
        s.notes_for(task, Some(NoteKind::Finding)).unwrap().len()
    }

    /// A runner's Finding on `task`, written `ago` milliseconds back.
    /// `insert_note_with_at_for_test` writes as `user`, and the cap counts
    /// only the runner's, so this writes the row itself.
    fn runner_finding_at(s: &Store, task: Uuid, at: i64) {
        s.conn()
            .execute(
                "INSERT INTO task_notes (id, task_id, kind, actor, at, body, extra) VALUES (?1, ?2, 'finding', 'runner', ?3, 'old', '{}')",
                params![uuid_blob(Uuid::now_v7()), uuid_blob(task), at],
            )
            .unwrap();
    }

    /// A board to file on.
    fn board(s: &Store) -> Uuid {
        let repo = s.register_repository_for_test("app");
        s.ensure_main_workspace(repo).unwrap().id
    }

    /// **A place turned away by the cap isn't kept**, so its frames aren't
    /// lost: once the day's Findings age out, its next report gets its
    /// Finding. A place kept is never noted twice.
    #[test]
    fn a_capped_place_gets_its_finding_once_the_cap_frees() {
        let s = Store::open_in_memory().unwrap();
        let board = board(&s);
        let intent = |n: u32| format!("seen {n}");
        let now = now_millis();
        let at_time = |place: &'static str, now: i64| {
            let mut f = filing("aa", "1", Some(board), &intent);
            f.place = place;
            s.record_crash_at(&f, now).unwrap()
        };
        let at = |place: &'static str| at_time(place, now);
        let task = at("p0").task.unwrap();
        at("p1");
        at("p2");
        assert_eq!(findings(&s, task), 3);
        at("p3");
        assert_eq!(findings(&s, task), 3, "capped");
        assert_eq!(places(&s, "aa"), ["p0", "p1", "p2"], "the capped place isn't kept");
        let tomorrow = now + DAY_MILLIS + 1_000;
        at_time("p3", tomorrow);
        assert_eq!(findings(&s, task), 4, "the cap freed: p3's Finding at last");
        at_time("p3", tomorrow);
        assert_eq!(findings(&s, task), 4, "kept now, so noted once");
        assert_eq!(places(&s, "aa"), ["p0", "p1", "p2", "p3"]);
    }

    /// **The cap's day is 24 hours.** A runner Finding older than a day
    /// doesn't count; one inside it does.
    #[test]
    fn findings_older_than_a_day_do_not_count() {
        let s = Store::open_in_memory().unwrap();
        let board = board(&s);
        let intent = |n: u32| format!("seen {n}");
        let task = s.record_crash(&filing("aa", "first", Some(board), &intent)).unwrap().task.unwrap();
        let now = now_millis();
        runner_finding_at(&s, task, now - DAY_MILLIS - 60_000);
        runner_finding_at(&s, task, now - DAY_MILLIS - 120_000);
        let mut f = filing("aa", "1", Some(board), &intent);
        f.place = "second";
        s.record_crash(&f).unwrap();
        assert_eq!(findings(&s, task), 4, "two old ones don't count against today's three");
        runner_finding_at(&s, task, now - DAY_MILLIS + 60_000);
        f.place = "third";
        s.record_crash(&f).unwrap();
        assert_eq!(findings(&s, task), 5, "the first, second and the inside-the-day one make three: third is turned away");
    }

    /// A row keeps at most `MAX_PLACES` places, the oldest going first.
    #[test]
    fn places_are_bounded() {
        let s = Store::open_in_memory().unwrap();
        let board = board(&s);
        let intent = |n: u32| format!("seen {n}");
        let names: Vec<String> = (0..MAX_PLACES + 5).map(|n| format!("p{n}")).collect();
        let task = s.record_crash(&filing("aa", "first", Some(board), &intent)).unwrap().task.unwrap();
        // A day apart, so the Finding cap never turns one away.
        let start = now_millis();
        for (n, name) in names.iter().enumerate() {
            let mut f = filing("aa", "1", Some(board), &intent);
            f.place = name;
            s.record_crash_at(&f, start + (n as i64 + 1) * (DAY_MILLIS + 1_000)).unwrap();
        }
        assert_eq!(findings(&s, task), names.len() + 1);
        let kept = places(&s, "aa");
        assert_eq!(kept.len(), MAX_PLACES);
        assert_eq!(kept.last(), names.last());
        assert_eq!(kept.first().map(String::as_str), Some("p5"));
    }

    /// **A failed write keeps nothing.** A card that can't be filed (its board
    /// is gone) fails the report, and neither the row nor its count is kept,
    /// so the phone's retry counts it once.
    #[test]
    fn a_failed_write_keeps_nothing() {
        let s = Store::open_in_memory().unwrap();
        let intent = |n: u32| format!("seen {n}");
        let err = s.record_crash(&filing("aa", "1", Some(Uuid::now_v7()), &intent)).unwrap_err();
        assert!(matches!(err, DomainError::NotFound), "{err:?}");
        assert!(s.crash_signatures().unwrap().is_empty());
    }

    /// The migration is the 52nd (0061) and `Welcome`.
    #[test]
    fn the_migration_is_welcome() {
        let it = &crate::migrate::MIGRATIONS[51];
        assert!(std::ptr::fn_addr_eq(it.0, migration_0061_crash_signatures as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
        assert_eq!(it.1, crate::compat::Older::Welcome);
        assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 55);
    }
}

//! What the runner last read of CI (ov-309, ov-306): one row per subject a
//! board names, so a train's pushed SHA and a page's CI reference draw from
//! the same read.
//!
//! A **subject** is `sha:<sha>` (every run on that commit), `run:<id>` (one
//! run) or `main` (every run on the default branch's newest commit that has
//! any). The runner reads them through `gh`, read only, while something on the
//! board names them (`daemon/src/ci_watch.rs`), and writes each read here
//! whole. Nothing is derived from a guess: a subject `gh` couldn't answer for
//! reads `unknown`, never green.
//!
//! Part of the plan layer and as removable: the table is migration 0027's
//! (`trains.rs`), and the removal drill drops it with the rest.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use farcooler_core::Result;

use crate::error::map_err;
use crate::models::uuid_blob;
use crate::store::Store;
use crate::tasks::now_millis;

/// The subject a SHA is read under.
pub fn sha_subject(sha: &str) -> String {
    format!("sha:{}", sha.to_ascii_lowercase())
}

/// The subject a run is read under.
pub fn run_subject(run: u64) -> String {
    format!("run:{run}")
}

/// The default branch's subject.
pub const MAIN_SUBJECT: &str = "main";

/// Where a subject's CI stands, over all its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiStatus {
    /// Every run finished and none failed.
    Passed,
    /// A run finished failed, timed out, was canceled or needs approval.
    Failed,
    /// A run is going.
    Running,
    /// Runs are waiting for a runner, none going yet.
    Queued,
    /// GitHub has no run for it (yet).
    None,
    /// `gh` couldn't say: logged out, offline, no such run.
    Unknown,
}

impl CiStatus {
    /// Every status's stored word.
    pub const WORDS: [&'static str; 6] = ["passed", "failed", "running", "queued", "none", "unknown"];

    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            CiStatus::Passed => "passed",
            CiStatus::Failed => "failed",
            CiStatus::Running => "running",
            CiStatus::Queued => "queued",
            CiStatus::None => "none",
            CiStatus::Unknown => "unknown",
        }
    }

    /// The status a stored word names.
    pub fn parse(raw: &str) -> Option<CiStatus> {
        Some(match raw {
            "passed" => CiStatus::Passed,
            "failed" => CiStatus::Failed,
            "running" => CiStatus::Running,
            "queued" => CiStatus::Queued,
            "none" => CiStatus::None,
            "unknown" => CiStatus::Unknown,
            _ => return None,
        })
    }

    /// Whether nothing more will happen without someone re-running it.
    pub fn is_finished(self) -> bool {
        matches!(self, CiStatus::Passed | CiStatus::Failed)
    }
}

/// One job, as a person reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiJob {
    /// The workflow's name and the job's: "CI / rust (ubuntu)".
    pub name: String,
    /// `passed`, `failed`, `running`, `queued`, `skipped` or `canceled`.
    pub state: String,
    /// Its page on GitHub, when GitHub gave one.
    pub url: String,
}

/// Jobs as the table keeps them.
fn jobs_json(jobs: &[CiJob]) -> String {
    serde_json::Value::Array(
        jobs.iter().map(|j| serde_json::json!({ "name": j.name, "state": j.state, "url": j.url })).collect(),
    )
    .to_string()
}

/// Jobs as the table kept them; what doesn't read is left out.
fn jobs_from(raw: &str) -> Vec<CiJob> {
    let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| CiJob { name: text(v, "name"), state: text(v, "state"), url: text(v, "url") })
            .collect(),
        _ => Vec::new(),
    }
}

/// What the runner last read of one subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiRead {
    pub subject: String,
    /// The commit its runs are for, in full; empty until a run names it.
    pub sha: String,
    pub status: CiStatus,
    /// The run's page, or for several runs, the first failing one's, else the
    /// first one's.
    pub url: String,
    pub jobs: Vec<CiJob>,
    /// When the runner last asked.
    pub fetched_at: i64,
    /// When what it heard last changed.
    pub changed_at: i64,
}

/// How GitHub's words for a job, or a run, read here: `status` is queued,
/// in_progress, waiting, pending, requested or completed, and `conclusion` is
/// set once it's completed.
pub fn job_state(status: &str, conclusion: Option<&str>) -> &'static str {
    match (status, conclusion) {
        ("completed", Some("success" | "neutral")) => "passed",
        ("completed", Some("skipped")) => "skipped",
        ("completed", Some("cancelled")) => "canceled",
        ("completed", _) => "failed",
        ("in_progress", _) => "running",
        _ => "queued",
    }
}

/// A subject's status from its runs' states (as `job_state` words): failed if
/// any failed or was canceled, else running if any runs, else queued if any
/// waits, else passed; none without runs.
pub fn status_of(run_states: &[&str]) -> CiStatus {
    if run_states.is_empty() {
        CiStatus::None
    } else if run_states.iter().any(|s| matches!(*s, "failed" | "canceled")) {
        CiStatus::Failed
    } else if run_states.contains(&"running") {
        CiStatus::Running
    } else if run_states.contains(&"queued") {
        CiStatus::Queued
    } else {
        CiStatus::Passed
    }
}

const COLS: &str = "subject, sha, status, url, jobs, fetched_at, changed_at";

fn row_to_read(r: &rusqlite::Row) -> rusqlite::Result<CiRead> {
    let status: String = r.get(2)?;
    let jobs: String = r.get(4)?;
    Ok(CiRead {
        subject: r.get(0)?,
        sha: r.get(1)?,
        status: CiStatus::parse(&status).unwrap_or(CiStatus::Unknown),
        url: r.get(3)?,
        jobs: jobs_from(&jobs),
        fetched_at: r.get(5)?,
        changed_at: r.get(6)?,
    })
}

fn read_in(conn: &Connection, workspace: Uuid, subject: &str) -> Result<Option<CiRead>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM board_ci WHERE workspace_id = ?1 AND subject = ?2"),
        params![uuid_blob(workspace), subject],
        row_to_read,
    )
    .optional()
    .map_err(map_err)
}

/// What one write of a read changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CiWrite {
    /// What was heard differs from the last read (its status, SHA, link or a
    /// job), so a client with the plan open should read it again.
    pub changed: bool,
    /// Trains moved to green, red or back to pushed by it.
    pub trains_moved: Vec<Uuid>,
}

impl Store {
    /// Keep what the runner just heard about `read.subject` on `workspace`'s
    /// board, and move the trains that follow it. An `unknown` read never
    /// replaces a known one: GitHub being unreachable says nothing new about
    /// the runs, so only `fetched_at` moves.
    pub fn record_ci(&self, workspace: Uuid, read: &CiRead) -> Result<CiWrite> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(map_err)?;
        crate::plan::board_exists(&tx, workspace)?;
        let now = now_millis();
        let before = read_in(&tx, workspace, &read.subject)?;
        if read.status == CiStatus::Unknown {
            if let Some(before) = &before {
                tx.execute(
                    "UPDATE board_ci SET fetched_at = ?3 WHERE workspace_id = ?1 AND subject = ?2",
                    params![uuid_blob(workspace), before.subject, now],
                )
                .map_err(map_err)?;
                tx.commit().map_err(map_err)?;
                return Ok(CiWrite::default());
            }
        }
        let changed = before.as_ref().is_none_or(|b| {
            (b.sha.as_str(), b.status, b.url.as_str(), &b.jobs) != (read.sha.as_str(), read.status, read.url.as_str(), &read.jobs)
        });
        let changed_at = match (&before, changed) {
            (Some(b), false) => b.changed_at,
            _ => now,
        };
        tx.execute(
            "INSERT INTO board_ci (workspace_id, subject, sha, status, url, jobs, fetched_at, changed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (workspace_id, subject) DO UPDATE SET
                 sha = excluded.sha, status = excluded.status, url = excluded.url, jobs = excluded.jobs,
                 fetched_at = excluded.fetched_at, changed_at = excluded.changed_at",
            params![
                uuid_blob(workspace),
                read.subject,
                read.sha,
                read.status.as_str(),
                read.url,
                jobs_json(&read.jobs),
                now,
                changed_at,
            ],
        )
        .map_err(map_err)?;
        let trains_moved = crate::trains::follow_ci(&tx, workspace, &read.subject, read.status)?;
        tx.commit().map_err(map_err)?;
        Ok(CiWrite { changed: changed || !trains_moved.is_empty(), trains_moved })
    }

    /// Forget the reads of subjects `workspace`'s board no longer names. How
    /// many went.
    pub fn keep_ci(&self, workspace: Uuid, named: &[String]) -> Result<usize> {
        let conn = self.conn();
        let have = ci_of(&conn, workspace)?;
        let mut gone = 0;
        for read in have.iter().filter(|r| !named.contains(&r.subject)) {
            gone += conn
                .execute(
                    "DELETE FROM board_ci WHERE workspace_id = ?1 AND subject = ?2",
                    params![uuid_blob(workspace), read.subject],
                )
                .map_err(map_err)?;
        }
        Ok(gone)
    }

    /// One subject's last read on `workspace`'s board.
    pub fn ci_read(&self, workspace: Uuid, subject: &str) -> Result<Option<CiRead>> {
        read_in(&self.conn(), workspace, subject)
    }
}

/// Every read a board holds, by subject.
pub(crate) fn ci_of(conn: &Connection, workspace: Uuid) -> Result<Vec<CiRead>> {
    let mut stmt = conn
        .prepare(&format!("SELECT {COLS} FROM board_ci WHERE workspace_id = ?1 ORDER BY subject"))
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![uuid_blob(workspace)], row_to_read)
        .map_err(map_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_err)?;
    Ok(rows)
}

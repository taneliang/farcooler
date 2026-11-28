//! Transactional outbox for non-gating build cleanup (ov-1126). Results remain readable by the board.
use crate::{Store, workflow::Result};
use farcooler_workflow::{cleanup::{CleanupEffect, CleanupResult}, State};
use rusqlite::{OptionalExtension, params, Transaction};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupRecord { pub effect: CleanupEffect, pub result: Option<CleanupResult> }
/// Older writers are refused: they cannot retain and honor an explicit build-output deletion policy.
pub(crate) fn migration(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE engine_build_cleanup (
        workspace TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL, result TEXT,
        PRIMARY KEY(workspace, id));")
}
pub(crate) fn select(tx: &Transaction<'_>, before: &State, after: &State) -> Result<()> {
    for effect in farcooler_workflow::cleanup::effects(before, after) {
        tx.execute("INSERT INTO engine_build_cleanup(workspace,id,payload) VALUES (?1,?2,?3)",
            params![effect.workspace, effect.id, serde_json::to_string(&effect)?])?;
    }
    Ok(())
}
impl Store {
    pub fn workflow_cleanup_has_authority(&self) -> Result<bool> {
        Ok(crate::engine_cutover::engine_allowed(&self.conn(), self.engine_incarnation)?)
    }

    /// The dispatch state of a directory's creating effect once the engine no longer runs it:
    /// `settled` or `suppressed` once inactive, or `uncertain` (the runner lost track of it, active
    /// or not). `None` while it runs or is pending, or when it is missing or quarantined. A missing
    /// or replaced owner is never evidence of quiescence; the runtime decides what an `uncertain`
    /// owner's run means (ov-1516).
    pub fn workflow_cleanup_owner_state(&self, effect: &CleanupEffect, owner: &str) -> Result<Option<String>> {
        let conn = self.conn();
        if !crate::engine_cutover::engine_allowed(&conn, self.engine_incarnation)? { return Ok(None); }
        Ok(conn.query_row("SELECT dispatch_state FROM workflow_outbox o
            WHERE workspace=?1 AND id=?2 AND (active=0 OR dispatch_state='uncertain')
            AND NOT EXISTS(SELECT 1 FROM engine_cutover_quarantine WHERE resource='workspace:'||?1
            OR resource=json_extract(o.payload,'$.resource'))",
            params![effect.workspace, owner], |r| r.get(0)).optional()?)
    }

    /// Recovery can retain a new, deterministic build-output intent for a known finished group.
    /// Never overwrite a prior attempt.
    pub fn workflow_cleanup_enqueue(&self, effect: &CleanupEffect) -> Result<()> {
        self.conn().execute("INSERT OR IGNORE INTO engine_build_cleanup(workspace,id,payload) VALUES (?1,?2,?3)",
            params![effect.workspace, effect.id, serde_json::to_string(effect)?])?;
        Ok(())
    }

    /// Only the exclusive runtime's recovery pass may revive an interrupted cleanup
    /// whose path has retained ownership evidence. Ordinary run cleanup is never replayed.
    pub fn workflow_cleanup_recover(&self, effect: &CleanupEffect, reason: &str) -> Result<()> {
        let resumed = CleanupResult { retry: true, skipped: vec![reason.into()], ..CleanupResult::default() };
        self.conn().execute("UPDATE engine_build_cleanup SET result=?3 WHERE workspace=?1 AND id=?2 AND result=?4",
            params![effect.workspace, effect.id, serde_json::to_string(&resumed)?, serde_json::to_string(&interrupted())?])?;
        Ok(())
    }

    /// A stale cleanup must not consume a later build's output, or bypass cutover quarantine.
    pub fn workflow_cleanup_skip(&self, effect: &CleanupEffect) -> Result<Option<String>> {
        let conn = self.conn();
        if !crate::engine_cutover::engine_allowed(&conn, self.engine_incarnation)? {
            return Ok(Some("engine authority unavailable".into()));
        }
        let blocked: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM engine_cutover_quarantine
            WHERE resource='workspace:'||?1 OR resource IN
                (SELECT json_extract(state,'$.items.'||json_quote(?2)||'.resource') FROM workflow_states WHERE workspace=?1))",
            params![effect.workspace, effect.item], |r| r.get(0))?;
        if blocked { return Ok(Some("resource quarantined".into())); }
        let newer: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM workflow_outbox WHERE workspace=?1
            AND revision>=?2 AND json_extract(payload,'$.item')=?3
            AND json_extract(payload,'$.kind.Execute') IN ('Provision','Build'))",
            params![effect.workspace, effect.revision, effect.item], |r| r.get(0))?;
        Ok(newer.then(|| "a later build owns this tree".into()))
    }

    pub fn workflow_cleanups(&self, workspace: &str) -> Result<Vec<CleanupRecord>> {
        self.read_workflow_cleanups(workspace, false)
    }

    pub fn workflow_pending_cleanups(&self, workspace: &str) -> Result<Vec<CleanupRecord>> {
        self.read_workflow_cleanups(workspace, true)
    }

    /// Every output directory a pending cleanup still names, across all boards.
    pub fn workflow_pending_output_paths(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT json_extract(payload,'$.directory.path') FROM engine_build_cleanup
            WHERE json_extract(payload,'$.directory.path') IS NOT NULL
            AND (result IS NULL OR json_extract(result,'$.retry')=1)")?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    fn read_workflow_cleanups(&self, workspace: &str, pending_only: bool) -> Result<Vec<CleanupRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT payload,result FROM engine_build_cleanup WHERE workspace=?1
            AND (NOT ?2 OR result IS NULL OR json_extract(result,'$.retry')=1) ORDER BY rowid")?;
        let rows = stmt.query_map(params![workspace, pending_only], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
        rows.map(|r| { let (effect, result) = r?;
            Ok(CleanupRecord { effect: serde_json::from_str(&effect)?, result: result.map(|s| serde_json::from_str(&s)).transpose()? })
        }).collect()
    }

    pub fn workflow_unsettled_writers(&self, workspace: &str) -> Result<Vec<farcooler_workflow::Effect>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT payload FROM workflow_outbox WHERE workspace=?1
            AND dispatch_state IN ('admitted','uncertain') AND json_type(payload,'$.kind.Execute') IS NOT NULL")?;
        let rows = stmt.query_map([workspace], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    /// Claim the observed pending or retryable result before filesystem mutation.
    /// The compare-and-swap prevents concurrent or stale sweeps from repeating an attempt.
    /// An interrupted attempt stays terminal until explicit group recovery; its byte count is unknown.
    pub fn workflow_cleanup_begin(&self, effect: &CleanupEffect, previous: Option<&CleanupResult>) -> Result<bool> {
        if previous.is_some_and(|r| !r.retry) { return Ok(false); }
        Ok(self.conn().execute("UPDATE engine_build_cleanup SET result=?3 WHERE workspace=?1 AND id=?2 AND result IS ?4",
            params![effect.workspace, effect.id, serde_json::to_string(&interrupted())?, previous.map(serde_json::to_string).transpose()?])? == 1)
    }
    /// Only the caller which successfully began this attempt may settle it. A later release has a new id.
    pub fn workflow_cleanup_result(&self, effect: &CleanupEffect, result: &CleanupResult) -> Result<()> {
        self.conn().execute("UPDATE engine_build_cleanup SET result=?3 WHERE workspace=?1 AND id=?2 AND result=?4",
            params![effect.workspace, effect.id, serde_json::to_string(result)?, serde_json::to_string(&interrupted())?])?;
        Ok(())
    }
}

fn interrupted() -> CleanupResult {
    CleanupResult { skipped: vec!["cleanup attempt interrupted; bytes freed unknown; deletion will not be replayed".into()], ..CleanupResult::default() }
}

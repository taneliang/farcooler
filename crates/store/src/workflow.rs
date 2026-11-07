//! Atomic workflow state, effects, selected informational notices and receipts.
//! Legacy task/line/landing authority remains untouched until coordinated replacement.
use farcooler_workflow::{Command, Input, State, Effect, Refusal, Grant, digest, owns_resource, step};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use crate::Store;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowError {
    #[error("workflow storage failed: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("workflow encoding failed: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error("command identity was reused with different content")]
    IdentityConflict,
    #[error("workflow item matches multiple tasks in its workspace")]
    AmbiguousTask,
    #[error("workflow authority is frozen or this Store incarnation is stale")]
    Authority,
    #[error("workflow input exceeds the admission bound")]
    Oversized,
    #[error("the board could not follow the run: {0}")]
    Board(String),
}
pub type Result<T> = std::result::Result<T, WorkflowError>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub result: std::result::Result<u64, Refusal>,
    pub effects: Vec<String>,
}
/// Stable delivery identity; reading or withholding this is never execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNotice {
    pub id: String,
    pub revision: u64,
    pub body: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEffect {
    pub effect: Effect,
    /// True means referenced by current state, not necessarily eligible for dispatch.
    pub active: bool,
    pub dispatch_state: String,
    pub resource_generation: Option<u64>,
    pub lease_until: Option<i64>,
}

impl Store {
    /// All new workflow mutations pass through this transaction. An accepted input
    /// cannot leave state without its event/outbox/receipt, even if a later SQL write fails.
    pub fn workflow_apply(&self, input: &Input) -> Result<Receipt> {
        let payload = serde_json::to_string(input)?;
        if payload.len() > 65536 { return Err(WorkflowError::Oversized); }

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::engine_cutover::engine_allowed(&tx, self.engine_incarnation)? { return Err(WorkflowError::Authority); }
        let mut moved = Vec::new();
        let receipt = apply_in(&tx, input, &payload, &mut moved)?;
        tx.commit()?;
        drop(conn);
        self.announce_board_moves(&moved);
        Ok(receipt)
    }

    /// Recovery discovers persisted aggregates without a connected controller.
    pub fn workflow_workspaces(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT workspace FROM workflow_states ORDER BY workspace")?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn workflow_state(&self, workspace: &str) -> Result<Option<State>> {
        let raw: Option<String> = self.conn().query_row(
            "SELECT state FROM workflow_states WHERE workspace = ?1", [workspace], |r| r.get(0),
        ).optional()?;
        raw.map(|s| serde_json::from_str(&s)).transpose().map_err(Into::into)
    }
    pub fn workflow_events(&self, workspace: &str) -> Result<Vec<Input>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT payload FROM workflow_events WHERE workspace = ?1 ORDER BY revision")?;
        let rows = stmt.query_map([workspace], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn workflow_notices(&self, workspace: &str, after_revision: u64) -> Result<Vec<WorkflowNotice>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT revision, notices FROM workflow_events WHERE workspace = ?1 AND revision > ?2 ORDER BY revision"
        )?;
        let rows = stmt.query_map(params![workspace, after_revision], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))?;
        let mut notices = Vec::new();
        for row in rows {
            let (revision, raw) = row?;
            for (index, body) in serde_json::from_str::<Vec<String>>(&raw)?.into_iter().enumerate() {
                notices.push(WorkflowNotice { id: digest(&("notice/v1", workspace, revision, index)), revision, body });
            }
        }
        Ok(notices)
    }
    pub fn workflow_outbox(&self, workspace: &str) -> Result<Vec<StoredEffect>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT payload, active, dispatch_state, resource_generation, lease_until FROM workflow_outbox WHERE workspace = ?1 ORDER BY revision, id")?;
        let rows = stmt.query_map([workspace], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<u64>>(3)?, r.get::<_, Option<i64>>(4)?)))?;
        rows.map(|r| { let (payload, active, dispatch_state, resource_generation, lease_until) = r?;
            Ok(StoredEffect { effect: serde_json::from_str(&payload)?, active, dispatch_state, resource_generation, lease_until }) }).collect()
    }
}

// Registry position is the version. Refuse older writers even though the tables
// are additive: those writers cannot preserve workflow admission/resource fences.
pub(crate) fn migration_workflow(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE workflow_states (
            workspace TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(revision > 0),
            epoch INTEGER NOT NULL, state TEXT NOT NULL);
         CREATE TABLE workflow_events (
            workspace TEXT NOT NULL REFERENCES workflow_states(workspace) DEFERRABLE INITIALLY DEFERRED,
            revision INTEGER NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL,
            payload_hash TEXT NOT NULL, notices TEXT NOT NULL,
            PRIMARY KEY(workspace, revision), UNIQUE(workspace, id));
         CREATE TABLE workflow_receipts (
            workspace TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL,
            payload_hash TEXT NOT NULL, receipt TEXT NOT NULL, PRIMARY KEY(workspace, id));
         CREATE TABLE workflow_outbox (
            workspace TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL,
            payload TEXT NOT NULL, payload_hash TEXT NOT NULL, active INTEGER NOT NULL CHECK(active IN (0,1)),
            dispatch_state TEXT NOT NULL DEFAULT 'pending'
                CHECK(dispatch_state IN ('pending','claimed','admitted','uncertain','settled','suppressed')),
            lease_owner TEXT, lease_generation INTEGER NOT NULL DEFAULT 0, lease_until INTEGER,
            resource_generation INTEGER,
            PRIMARY KEY(workspace, id), FOREIGN KEY(workspace, revision)
                REFERENCES workflow_events(workspace, revision));
         CREATE TABLE workflow_resources (
            resource TEXT PRIMARY KEY, workspace TEXT NOT NULL REFERENCES workflow_states(workspace),
            item TEXT NOT NULL, run TEXT NOT NULL, fence INTEGER NOT NULL, generation INTEGER NOT NULL,
            UNIQUE(workspace, item));
         CREATE TABLE workflow_resource_generations (resource TEXT PRIMARY KEY, generation INTEGER NOT NULL);
         CREATE TRIGGER workflow_events_no_update BEFORE UPDATE ON workflow_events
            BEGIN SELECT RAISE(ABORT, 'immutable workflow event'); END;
         CREATE TRIGGER workflow_events_no_delete BEFORE DELETE ON workflow_events
            BEGIN SELECT RAISE(ABORT, 'immutable workflow event'); END;
         CREATE TRIGGER workflow_receipts_no_update BEFORE UPDATE ON workflow_receipts
            BEGIN SELECT RAISE(ABORT, 'immutable workflow receipt'); END;
         CREATE TRIGGER workflow_receipts_no_delete BEFORE DELETE ON workflow_receipts
            BEGIN SELECT RAISE(ABORT, 'immutable workflow receipt'); END;
         CREATE TRIGGER workflow_outbox_no_payload_update
            BEFORE UPDATE OF workspace, id, revision, payload, payload_hash ON workflow_outbox
            BEGIN SELECT RAISE(ABORT, 'immutable workflow effect'); END;
         CREATE TRIGGER workflow_outbox_no_delete BEFORE DELETE ON workflow_outbox
            BEGIN SELECT RAISE(ABORT, 'immutable workflow effect'); END;"
    )
}

#[cfg(test)]
mod tests;

pub(crate) fn apply_in(tx: &Transaction<'_>, input: &Input, payload: &str, moved: &mut Vec<crate::workflow_projection::BoardMove>) -> Result<Receipt> {
        let previous: Option<(String, String)> = tx.query_row(
            "SELECT payload, receipt FROM workflow_receipts WHERE workspace = ?1 AND id = ?2",
            params![input.workspace, input.id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        if let Some((original, receipt)) = previous {
            if original != payload { return Err(WorkflowError::IdentityConflict); }
            return Ok(serde_json::from_str(&receipt)?);
        }
        let workspace_frozen:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM engine_cutover_quarantine WHERE resource=?1)",[format!("workspace:{}",input.workspace)],|r|r.get(0))?;
        if workspace_frozen {return Err(WorkflowError::Authority);}
        let raw: Option<String> = tx.query_row(
            "SELECT state FROM workflow_states WHERE workspace = ?1", [&input.workspace], |r| r.get(0),
        ).optional()?;
        let state = match &raw {
            Some(raw) => serde_json::from_str(raw)?,
            None => State::empty(input.workspace.clone(), input.epoch),
        };
        // The transaction, not a caller, supplies external occupancy. Persist it
        // with the accepted event so replay needs no database/global state reads.
        let mut admitted = input.clone();
        let mut relevant: std::collections::BTreeSet<_> = state.items.values().map(|i| i.resource.clone()).collect();
        if let farcooler_workflow::Command::Enqueue { resource, .. } = &input.command {
            relevant.insert(resource.clone());
        }
        admitted.busy_resources = tx.prepare(
            "SELECT resource FROM workflow_resources WHERE workspace != ?1
             AND resource IN (SELECT value FROM json_each(?2))"
        )?.query_map(params![input.workspace, serde_json::to_string(&relevant)?], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        let blocked:std::collections::BTreeSet<String>=tx.prepare("SELECT resource FROM engine_cutover_quarantine")?
            .query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let target=match &input.command {
            Command::Enqueue {resource,..} => Some(resource.as_str()),
            Command::Approve {item,..}|Command::Review {item,..}|Command::Observe {item,..}|Command::Cancel {item}|Command::Retry {item}|Command::Resolve {item,..} => state.items.get(item).map(|i|i.resource.as_str()),
            Command::Advance => None,
        };
        let quarantined=target.is_some_and(|resource|blocked.contains(resource));
        admitted.busy_resources.extend(blocked.iter().filter(|resource|relevant.contains(*resource)).cloned());
        let mut result = if quarantined { Err(Refusal::Resource) } else if raw.is_none() && input.grant != Grant::Controller {
            Err(Refusal::Unauthorized)
        } else { step(&state, &admitted) };
        if result.as_ref().is_ok_and(|transition|transition.effects.iter().any(|effect|blocked.contains(&effect.resource))) { result=Err(Refusal::Resource); }
        if result.is_ok() && !crate::workflow_ownership::available(tx, input)? {
            result = Err(Refusal::Resource);
        }
        if result.is_ok() {
            if let farcooler_workflow::Command::Observe { effect, .. }
                | farcooler_workflow::Command::Resolve { effect, .. } = &input.command
            {
                let status: Option<String> = tx.query_row(
                    "SELECT dispatch_state FROM workflow_outbox WHERE workspace = ?1 AND id = ?2",
                    params![input.workspace, effect], |r| r.get(0),
                ).optional()?;
                let required = if matches!(input.command, farcooler_workflow::Command::Resolve { .. }) {
                    "uncertain"
                } else { "admitted" };
                if status.as_deref() != Some(required) { result = Err(Refusal::Phase); }
            }
        }
        if let Ok(transition) = &result {
            for item in transition.state.items.values().filter(|i| owns_resource(i)) {
                let owner: Option<String> = tx.query_row(
                    "SELECT workspace FROM workflow_resources WHERE resource = ?1",
                    [&item.resource], |r| r.get(0),
                ).optional()?;
                if owner.as_ref().is_some_and(|owner| owner != &input.workspace) {
                    result = Err(Refusal::Resource);
                    break;
                }
            }
        }
        let receipt = match result {
            Err(reason) => Receipt { id: input.id.clone(), result: Err(reason), effects: vec![] },
            Ok(transition) => {
                crate::workflow_ownership::reserve(tx, input)?;
                let revision = transition.state.revision;
                tx.execute(
                    "INSERT INTO workflow_events (workspace, revision, id, payload, payload_hash, notices)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![input.workspace, revision, input.id, serde_json::to_string(&admitted)?, digest(&admitted),
                        serde_json::to_string(&transition.notices)?],
                )?;
                tx.execute(
                    "INSERT INTO workflow_states (workspace, revision, epoch, state) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(workspace) DO UPDATE SET revision = excluded.revision,
                     epoch = excluded.epoch, state = excluded.state",
                    params![input.workspace, revision, input.epoch, serde_json::to_string(&transition.state)?],
                )?;
                crate::workflow_notices::select(tx, &admitted, &state, &transition.state)?;
                moved.extend(crate::workflow_projection::project(tx, &input.workspace, &transition.state)?);
                crate::workflow_effects::reserve(tx, &transition.state)?;
                // Never erase operation history. Cancellation/rework makes old intents inert.
                tx.execute("UPDATE workflow_outbox SET active = 0 WHERE workspace = ?1", [&input.workspace])?;
                for effect in &transition.effects {
                    tx.execute(
                        "INSERT INTO workflow_outbox (workspace, id, revision, payload, payload_hash, active)
                         VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                        params![input.workspace, effect.id, revision, serde_json::to_string(effect)?, digest(effect)],
                    )?;
                }
                for item in transition.state.items.values() {
                    if let Some(effect) = &item.pending {
                        tx.execute("UPDATE workflow_outbox SET active = 1 WHERE workspace = ?1 AND id = ?2",
                            params![input.workspace, effect])?;
                    }
                }
                if let farcooler_workflow::Command::Observe { effect, outcome, .. }
                    | farcooler_workflow::Command::Resolve { effect, outcome, .. } = &input.command
                {
                    let status = if matches!(outcome, farcooler_workflow::Outcome::Uncertain) {
                        "uncertain"
                    } else { "settled" };
                    tx.execute("UPDATE workflow_outbox SET dispatch_state = ?3 WHERE workspace = ?1 AND id = ?2",
                        params![input.workspace, effect, status])?;
                }
                if let farcooler_workflow::Command::Observe { outcome: farcooler_workflow::Outcome::Stopped { operation, .. }, .. }
                    | farcooler_workflow::Command::Resolve { outcome: farcooler_workflow::Outcome::Stopped { operation, .. }, .. } = &input.command
                {
                    tx.execute("UPDATE workflow_outbox SET dispatch_state = 'settled' WHERE workspace = ?1 AND id = ?2",
                        params![input.workspace, operation])?;
                }
                tx.execute(
                    "UPDATE workflow_outbox SET dispatch_state = CASE
                        WHEN dispatch_state IN ('admitted', 'uncertain') THEN 'uncertain'
                        ELSE 'suppressed' END
                     WHERE workspace = ?1 AND active = 0 AND dispatch_state NOT IN ('settled', 'suppressed')",
                    [&input.workspace],
                )?;
                Receipt { id: input.id.clone(), result: Ok(revision),
                    effects: transition.effects.iter().map(|e| e.id.clone()).collect() }
            }
        };
        tx.execute(
            "INSERT INTO workflow_receipts (workspace, id, payload, payload_hash, receipt)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input.workspace, input.id, payload, digest(input), serde_json::to_string(&receipt)?],
        )?;
        Ok(receipt)
}

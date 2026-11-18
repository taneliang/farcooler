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
    /// Immutable publication or grant audit, written with the selection and receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<serde_json::Value>,
    pub id: String,
    pub result: std::result::Result<u64, Refusal>,
    pub effects: Vec<String>,
    /// Who sent the wire command this answers (ov-1001). Absent on the engine's own receipts, and on a wire
    /// receipt written before anyone was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<WorkflowActor>,
}
impl Receipt {
    pub fn send_back_reason(&self) -> Option<String> {
        match &self.result {
            Err(Refusal::SendBackRefused(reason)) => Some(format!("{reason} (receipt {})", self.id)),
            _ => None,
        }
    }
}
/// Who sent a wire command (ov-1001): the person, and the device or pane it came from. The daemon reads it from
/// the caller's admission; nothing a client sends names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowActor {
    /// `owner`, or `member:<handle>`: whose the caller is, an orchestrator pane being its login's person.
    pub person: String,
    /// The device id the command came from, if it came from a device rather than the local socket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// The pane it came from: the orchestrator's, or the person's own shell. Absent for the local socket outside
    /// any pane (the Mac app, the owner's own terminal) and for a device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<uuid::Uuid>,
    /// Whether `pane` is the workspace's orchestrator, which the board writes as `manager`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orchestrator: bool,
}
impl WorkflowActor {
    /// Who the board's history says moved a card this command moved (ruling R-108): the orchestrator as
    /// `manager`, the owner as `user`, a member as themself. Anything else is the runner's, as before.
    pub fn board_actor(&self) -> crate::models::Actor {
        use crate::models::{Actor, Handle};
        if self.orchestrator { return Actor::Manager; }
        match self.person.strip_prefix("member:").map(Handle::new) {
            _ if self.person == "owner" => Actor::User,
            Some(Some(handle)) => Actor::Member { handle },
            _ => Actor::Runner,
        }
    }
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
    pub fn workflow_apply(&self, input: &Input) -> Result<Receipt> { self.workflow_apply_by(input, None) }
    /// `workflow_apply`, with the card history naming `by` for any card the command moves (R-108, R-111 L4).
    pub fn workflow_apply_by(&self, input: &Input, by: Option<&crate::models::Actor>) -> Result<Receipt> {
        self.workflow_apply_snapshot_by(input, by).map(|(receipt, _)| receipt)
    }
    /// Apply through ordinary admission and return the committed state for the scheduler's scan snapshot.
    /// Refusals and receipt replays return no replacement state.
    pub fn workflow_apply_snapshot(&self, input: &Input) -> Result<(Receipt, Option<State>)> {
        self.workflow_apply_snapshot_by(input, None)
    }
    fn workflow_apply_snapshot_by(&self, input: &Input, by: Option<&crate::models::Actor>) -> Result<(Receipt, Option<State>)> {
        let payload = serde_json::to_string(input)?;
        let mut request = input.clone();
        if let Command::Start { start_load, .. } = &mut request.command { *start_load = None; }
        if serde_json::to_vec(&request)?.len() > 65536 { return Err(WorkflowError::Oversized); }

        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::engine_cutover::engine_allowed(&tx, self.engine_incarnation)? { return Err(WorkflowError::Authority); }
        let mut moved = Vec::new();
        let applied = apply_in_snapshot(&tx, input, &payload, by, &mut moved)?;
        tx.commit()?;
        drop(conn);
        self.announce_board_moves(&moved);
        if applied.0.result.is_ok() { self.announce_commit(&input.workspace); }
        Ok(applied)
    }

    /// Whether `workspace` names a board that was deleted: a key that is a board id with no board behind it.
    /// Other keys are fixtures' and hold (the rule `named_by_a_workflow` uses, ov-1045). So does every key in a
    /// store with no boards at all: a real one always has Main, and a runtime opened alone has none to compare with.
    pub fn workflow_board_deleted(&self, workspace: &str) -> Result<bool> {
        let Ok(id) = workspace.parse::<uuid::Uuid>() else { return Ok(false) };
        let conn = self.conn();
        let boards: i64 = conn.query_row("SELECT count(*) FROM workspaces", [], |r| r.get(0))?;
        let found: Option<i64> = conn.query_row("SELECT 1 FROM workspaces WHERE id = ?1", [crate::models::uuid_blob(id)], |r| r.get(0)).optional()?;
        Ok(boards > 0 && found.is_none())
    }
    /// Recovery discovers persisted aggregates without a connected controller.
    pub fn workflow_workspaces(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT workspace FROM workflow_states ORDER BY workspace")?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Published constraints apply even before a board has its first tracked ticket or run.
    pub fn workflow_scheduling_workspaces(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT workspace FROM workflow_states
            UNION SELECT workspace FROM workflow_bundle_selections ORDER BY workspace")?;
        Ok(stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Recover the runner-wide load gap from accepted starts, even after completion or archival.
    /// Events contain only accepted commands, so a refused start cannot postpone admission.
    pub fn workflow_last_automatic_start_ms(&self) -> Result<Option<i64>> {
        Ok(self.conn().query_row("SELECT max(json_extract(payload, '$.now_ms')) FROM workflow_events
            WHERE json_extract(payload, '$.source') = 'runtime'
            AND json_type(payload, '$.command.Start') = 'object'", [], |r| r.get(0))?)
    }

    /// Cheap cache invalidation without loading the aggregate JSON.
    pub fn workflow_state_revision(&self, workspace: &str) -> Result<Option<u64>> {
        Ok(self.conn().query_row("SELECT revision FROM workflow_states WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?)
    }
    /// Only needed when classifying a changed board without a start policy. Tracker-only boards can be skipped.
    pub fn workflow_has_runs(&self, workspace: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT EXISTS(SELECT 1 FROM workflow_states s, json_each(s.state, '$.items') i
            WHERE s.workspace=?1 AND json_extract(i.value, '$.definition.source') != ?2)",
            params![workspace, farcooler_workflow::TICKET_SOURCE], |r| r.get(0))?)
    }

    pub fn workflow_state(&self, workspace: &str) -> Result<Option<State>> {
        let raw: Option<String> = self.conn().query_row(
            "SELECT state FROM workflow_states WHERE workspace = ?1", [workspace], |r| r.get(0),
        ).optional()?;
        raw.map(|s| serde_json::from_str(&s)).transpose().map_err(Into::into)
    }
    /// Test-only: replace a workspace's state as it stands, outside the reducer, for a fixture that needs a
    /// history the reducer would take thousands of steps to build, such as hundreds of archived runs (ov-1298).
    #[cfg(any(test, feature = "testing"))]
    pub fn workflow_state_for_tests(&self, state: &State) -> Result<()> {
        self.conn().execute("UPDATE workflow_states SET state = ?2 WHERE workspace = ?1",
            params![state.workspace, serde_json::to_string(state)?])?;
        Ok(())
    }
    /// The card a run's item is, if it is one (a standalone item has none).
    pub fn workflow_item_task(&self, workspace: &str, item: &str) -> Result<Option<uuid::Uuid>> {
        let raw: Option<Option<Vec<u8>>> = self.conn().query_row(
            "SELECT task_id FROM workflow_owned_items WHERE workspace = ?1 AND item = ?2", [workspace, item], |r| r.get(0),
        ).optional()?;
        Ok(raw.flatten().and_then(|b| uuid::Uuid::from_slice(&b).ok()))
    }
    /// The workspace and item of the run a card is, if the engine owns it (`workflow_item_task` the other way).
    pub fn workflow_item_of_task(&self, task: uuid::Uuid) -> Result<Option<(String, String)>> {
        Ok(self.conn().query_row(
            "SELECT workspace, item FROM workflow_owned_items WHERE task_id = ?1", [task.as_bytes().as_slice()], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?)
    }
    /// The last refused send-back for each current run. Old requests stay in their immutable receipts.
    pub fn workflow_send_back_refusals(&self, state: &State) -> Result<std::collections::BTreeMap<String, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT payload, receipt FROM workflow_wire_receipts
             WHERE workspace = ?1 AND json_type(receipt, '$.result.Err.SendBackRefused') = 'text'
             ORDER BY rowid DESC")?;
        let mut rows = stmt.query([&state.workspace])?;
        let mut reasons = std::collections::BTreeMap::new();
        while let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            let (_, action): (String, farcooler_protocol::workflow_commands::Action) = serde_json::from_str(&payload)?;
            let farcooler_protocol::workflow_commands::Action::SendBack { item, run, pin, mutation, .. } = action else { continue };
            if mutation.epoch != state.epoch || reasons.contains_key(&item)
                || !state.items.get(&item).is_some_and(|i| i.run == run && i.pin == pin) {
                continue;
            }
            let raw: String = row.get(1)?;
            if let Some(reason) = serde_json::from_str::<Receipt>(&raw)?.send_back_reason() {
                reasons.insert(item, reason);
            }
        }
        Ok(reasons)
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

/// `by` is who the board says moved a card this input moves: a wire command's sender (ov-1001), else the runner.
pub(crate) fn apply_in(tx: &Transaction<'_>, input: &Input, payload: &str, by: Option<&crate::models::Actor>,
    moved: &mut Vec<crate::workflow_projection::BoardMove>) -> Result<Receipt> {
    apply_in_snapshot(tx, input, payload, by, moved).map(|(receipt, _)| receipt)
}
fn apply_in_snapshot(tx: &Transaction<'_>, input: &Input, payload: &str, by: Option<&crate::models::Actor>,
    moved: &mut Vec<crate::workflow_projection::BoardMove>) -> Result<(Receipt, Option<State>)> {
        let previous: Option<(String, String)> = tx.query_row(
            "SELECT payload, receipt FROM workflow_receipts WHERE workspace = ?1 AND id = ?2",
            params![input.workspace, input.id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?;
        if let Some((original, receipt)) = previous {
            if original != payload { return Err(WorkflowError::IdentityConflict); }
            return Ok((serde_json::from_str(&receipt)?, None));
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
        if let farcooler_workflow::Command::Enqueue { resource, .. } | farcooler_workflow::Command::Start { resource, .. }
            | farcooler_workflow::Command::Reopen { resource, .. } = &input.command {
            relevant.insert(resource.clone());
        }
        // Queue can create a landing resource not held by any item yet. Include published and pinned
        // group resources before asking the reducer to acquire one, across workspaces as well.
        relevant.extend(state.items.values().flat_map(|i| i.definition.groups.values().map(|g| g.exclusive.clone())));
        if let Some(grouping) = &state.grouping {
            relevant.extend(grouping.definition.groups.values().map(|g| g.exclusive.clone()));
        }
        if let Command::RefreshGroups { definition, .. } = &input.command {
            relevant.extend(definition.groups.values().map(|g| g.exclusive.clone()));
        }
        admitted.busy_resources = tx.prepare(
            "SELECT resource FROM workflow_resources WHERE workspace != ?1
             AND resource IN (SELECT value FROM json_each(?2))"
        )?.query_map(params![input.workspace, serde_json::to_string(&relevant)?], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        let blocked:std::collections::BTreeSet<String>=tx.prepare("SELECT resource FROM engine_cutover_quarantine")?
            .query_map([],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let target=match &input.command {
            Command::Enqueue {resource,..} | Command::Start {resource,..} | Command::Reopen {resource,..} => Some(resource.as_str()),
            Command::Approve {item,..}|Command::Review {item,..}|Command::Observe {item,..}|Command::Cancel {item,..}|Command::Retry {item}|Command::Queue {item}|Command::Leave {item}|Command::Move {item,..}|Command::Resolve {item,..}
                |Command::Decide {item,..}|Command::SendBack {item,..}|Command::Close {item,..}|Command::Land {item,..}
                |Command::RefreshJoin {item,..}|Command::RepinPending {item,..} => state.items.get(item).map(|i|i.resource.as_str()),
            Command::Advance | Command::RefreshGroups { .. } => None,
        };
        // A Start takes its followers with it (ov-970), so their quarantine holds it back as the lead's does.
        let followers:Vec<&str>=match &input.command { Command::Start {with,..} => with.iter().filter_map(|f|state.items.get(f)).map(|i|i.resource.as_str()).collect(), _ => Vec::new() };
        let quarantined=target.into_iter().chain(followers).any(|resource|blocked.contains(resource));
        admitted.busy_resources.extend(blocked.iter().filter(|resource|relevant.contains(*resource)).cloned());
        let mut result = if quarantined { Err(Refusal::Resource) } else if raw.is_none() && input.grant != Grant::Controller {
            Err(Refusal::Unauthorized)
        } else { step(&state, &admitted) };
        if result.as_ref().is_ok_and(|transition|transition.effects.iter().any(|effect|blocked.contains(&effect.resource))) { result=Err(Refusal::Resource); }
        if result.is_ok() {
            if let Some(why) = crate::workflow_ownership::start_held(tx, input)? { result = Err(Refusal::Held(why)); }
            else if !crate::workflow_ownership::available(tx, input)? { result = Err(Refusal::Resource); }
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
        if result.is_ok() && let Command::Decide { item, .. } = &input.command {
            let effect = state.items.get(item).and_then(|i| i.pending.as_ref());
            let status: Option<String> = tx.query_row(
                "SELECT dispatch_state FROM workflow_outbox WHERE workspace=?1 AND id=?2",
                params![input.workspace, effect], |r| r.get(0)).optional()?;
            if status.as_deref() != Some("admitted") { result = Err(Refusal::Phase); }
        }
        if result.is_ok() && let Command::RepinPending { effect, policy_pin, definition, .. } = &input.command {
            // Serialize re-pinning with publication and with the original effect's admission.
            let selected: Option<String> = tx.query_row(
                "SELECT b.definition_pin FROM workflow_bundle_selections s JOIN workflow_bundles b ON b.pin=s.pin
                 WHERE s.workspace=?1 AND s.pin=?2", params![input.workspace, policy_pin], |r| r.get(0),
            ).optional()?;
            if selected.is_none_or(|pin| definition.as_ref().is_some_and(|definition| definition.pin() != pin)) {
                result = Err(Refusal::Stale);
            } else {
                let unstarted: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM workflow_outbox WHERE workspace=?1 AND id=?2 AND active=1
                     AND dispatch_state IN ('pending','claimed') AND json_type(payload, '$.kind.Execute') IS NOT NULL)",
                    params![input.workspace, effect], |r| r.get(0),
                )?;
                if !unstarted || !crate::workflow_effects::executable_payload(tx, &input.workspace, effect)? {
                    result = Err(Refusal::Phase);
                }
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
        let mut applied = None;
        // A Start refused for a resource says which, and what holds it (ov-1220).
        if matches!(result, Err(Refusal::Resource)) {
            if let Some(why) = crate::workflow_ownership::resource_held(tx, input, &state)? { result = Err(Refusal::Held(why)); }
        }
        let receipt = match result {
            Err(reason) => Receipt { change:None, id: input.id.clone(), result: Err(reason), effects: vec![], actor: None },
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
                crate::engine_stage::sync(tx, &input.workspace, &transition.state)?;
                crate::workflow_cleanup::select(tx, &state, &transition.state)?;
                crate::workflow_notices::select(tx, &admitted, &state, &transition.state)?;
                moved.extend(crate::workflow_projection::project(tx, &input.workspace, &transition.state, by)?);
                crate::workflow_projection::start_load_event(tx, &admitted, by)?;
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
                let effects = transition.effects.iter().map(|e| e.id.clone()).collect();
                applied = Some(transition.state);
                Receipt { change:None, id: input.id.clone(), result: Ok(revision), effects, actor: None }
            }
        };
        tx.execute(
            "INSERT INTO workflow_receipts (workspace, id, payload, payload_hash, receipt)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input.workspace, input.id, payload, digest(input), serde_json::to_string(&receipt)?],
        )?;
        Ok((receipt, applied))
}

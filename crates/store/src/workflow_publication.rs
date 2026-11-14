//! Publication and authenticated wire receipts share the workflow transaction.
use crate::{Store, workflow::{Receipt, Result, WorkflowActor, WorkflowError, apply_in}};
use farcooler_protocol::workflow_config::{AdapterConfiguration, NotificationPolicy};
use farcooler_workflow::{Definition, Input, Refusal};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration { pub pin: String, pub definition: Definition, pub adapter: Option<AdapterConfiguration>, #[serde(default, skip_serializing_if = "Option::is_none")] pub notifications: Option<NotificationPolicy> }
pub fn configuration_pin(definition: &Definition, adapter: Option<&AdapterConfiguration>) -> String {
    adapter.map_or_else(|| definition.pin(), |a| farcooler_workflow::digest(&("workflow-bundle/v1", definition, a)))
}
pub fn notification_configuration_pin(definition: &Definition, adapter: Option<&AdapterConfiguration>, notifications: Option<&NotificationPolicy>) -> String {
    notifications.map_or_else(|| configuration_pin(definition, adapter), |p| farcooler_workflow::digest(&("workflow-bundle/v2", definition, adapter, p)))
}
/// ov-1201: whether a publication runs exactly what `previous` (the configuration it replaces) runs, under the
/// approvals it ran under. Every process the engine starts comes from the adapter configuration: gate and check
/// commands with their environments, the harness with the hooks files it pins, the reviewer, CI, landing and pull
/// request commands. All of them run as the runner's Unix user, with whatever credentials its files hold, so only
/// a person may change them. A step's own `operation.environment` may be dropped, or kept as the same step had it,
/// never added to.
///
/// The definition says which of those commands run and who must say yes first (review a2472ebc), so with the
/// adapter unchanged each step that runs something must be one a step of `previous` ran, behind at least the
/// approvals and person reviews that step was behind on every way to it. Routes, models, deciders, limits and
/// notifications are free to change within that: a new route may not pass a decider by, a decider may not be
/// dropped or swapped, and a configured command no step ran stays unrun.
pub fn runs_the_same(previous: Option<&Configuration>, definition: &Definition, adapter: Option<&AdapterConfiguration>) -> bool {
    fn environments(definition: &Definition) -> std::collections::BTreeMap<(&str, &str), &std::collections::BTreeMap<String, String>> {
        definition.stages.iter().map(|s| ("", s))
            .chain(definition.groups.iter().flat_map(|(group, g)| g.steps.iter().map(move |s| (group.as_str(), s))))
            .map(|(group, s)| ((group, s.id.as_str()), &s.operation.environment)).collect()
    }
    let before = previous.map(|c| environments(&c.definition)).unwrap_or_default();
    let earlier = previous.map(|c| guarded::steps(&c.definition)).unwrap_or_default();
    previous.and_then(|c| c.adapter.as_ref()) == adapter
        && environments(definition).into_iter().all(|(step, e)| e.is_empty() || before.get(&step) == Some(&e))
        && guarded::steps(definition).into_iter().filter(|(s, _)| !guarded::runs_nothing(s)).all(|(s, guards)|
            earlier.iter().any(|(was, had)| was.kind == s.kind && was.operation == s.operation && match (&guards, had) {
                // A step no run reaches still runs nothing new.
                (None, _) => true,
                (Some(now), Some(had)) => had.is_subset(now),
                (Some(_), None) => false,
            }))
}
/// Who must have said yes before each step of a definition starts (ov-1201).
mod guarded {
    use farcooler_workflow::{Definition, Progression, Reviewer, Stage, StageKind, Target};
    use std::collections::{BTreeMap, BTreeSet};
    /// The principals every way to a step passes; `None` when no run reaches it.
    pub(super) type Guards = Option<BTreeSet<String>>;
    /// A step that starts no process: a ticket's column, a join, a path route, a person's or the orchestrator's review.
    pub(super) fn runs_nothing(stage: &Stage) -> bool {
        matches!(stage.kind, StageKind::Tracker | StageKind::Join { .. } | StageKind::Route
            | StageKind::Review(Reviewer::Person(_) | Reviewer::Orchestrator(_)))
    }
    fn approver(stage: &Stage) -> Option<String> {
        match &stage.progression { Progression::AwaitApproval { principal } => Some(principal.clone()), Progression::Automatic => None }
    }
    /// A review's decider, whose verdict is behind its `passed` route only.
    fn reviewer(stage: &Stage) -> Option<String> {
        match &stage.kind { StageKind::Review(Reviewer::Person(p) | Reviewer::Orchestrator(p)) => Some(p.clone()), _ => None }
    }
    /// Every step of `definition` and its groups, with its guards: the deciders on every way to it, its own approver
    /// included. A group's steps also have the ones on every way to a join that queues into it.
    pub(super) fn steps(definition: &Definition) -> Vec<(&Stage, Guards)> {
        let top = must_pass(definition, Some(BTreeSet::new()));
        let mut out: Vec<(&Stage, Guards)> = definition.stages.iter().zip(top.iter().cloned()).collect();
        for (name, group) in &definition.groups {
            let entry = definition.stages.iter().zip(&top)
                .filter(|(s, _)| matches!(&s.kind, StageKind::Join { group, .. } if group == name))
                .fold(None, |all: Guards, (_, g)| meet(all, g.clone()));
            let inner = definition.group_definition(name).map_or_else(|| vec![None; group.steps.len()], |d| must_pass(&d, entry));
            out.extend(group.steps.iter().zip(inner));
        }
        out
    }
    /// The deciders both ways pass.
    fn meet(a: Guards, b: Guards) -> Guards {
        match (a, b) { (None, x) | (x, None) => x, (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()) }
    }
    /// Each step's guards, from the first step's `entry`. Every route out counts, a split's step and its fallback,
    /// falling forward on an unrouted `passed`, a rework, and the orchestrator's send-back to any build step.
    fn must_pass(definition: &Definition, entry: Guards) -> Vec<Guards> {
        let stages = &definition.stages;
        let index: BTreeMap<&str, usize> = stages.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        let edges = |i: usize| -> Vec<(usize, bool)> {
            let stage = &stages[i];
            let mut out = Vec::new();
            if definition.schema == 2 {
                for (label, target) in &stage.routes {
                    let mut target = target;
                    loop {
                        match target {
                            Target::Step(id) => { out.extend(index.get(id.as_str()).map(|j| (*j, label == "passed"))); break }
                            Target::Split { step, otherwise } => {
                                out.extend(index.get(step.as_str()).map(|j| (*j, label == "passed")));
                                target = otherwise;
                            }
                            Target::Done | Target::Fail => break,
                        }
                    }
                }
                if !stage.routes.contains_key("passed") && i + 1 < stages.len() { out.push((i + 1, true)); }
            } else {
                if i + 1 < stages.len() { out.push((i + 1, true)); }
                if matches!(stage.kind, StageKind::Review(_) | StageKind::Evaluate(_)) {
                    out.extend(definition.rework_target.as_deref().and_then(|t| index.get(t)).map(|j| (*j, false)));
                }
            }
            out.extend(stages.iter().enumerate().filter(|(_, s)| s.kind == StageKind::Build).map(|(j, _)| (j, false)));
            out
        };
        let mut at: Vec<Guards> = vec![None; stages.len()];
        if let Some(first) = at.first_mut() { *first = entry; }
        // Sets only shrink, from unreached to smaller, so this settles.
        loop {
            let mut changed = false;
            for i in 0..stages.len() {
                let Some(mut through) = at[i].clone() else { continue };
                through.extend(approver(&stages[i]));
                for (j, passed) in edges(i) {
                    let mut out = through.clone();
                    if passed { out.extend(reviewer(&stages[i])); }
                    let next = meet(at[j].clone(), Some(out));
                    if at[j] != next { at[j] = next; changed = true; }
                }
            }
            if !changed { break }
        }
        at.into_iter().zip(stages).map(|(g, s)| g.map(|mut g| { g.extend(approver(s)); g })).collect()
    }
}
pub enum Write<'a> {
    PublicationGrant { enabled: bool, revision: u64 },
    Rollback { expected_pin: &'a str, reason: &'a str, revision: u64 },
    Publish { expected_pin: Option<&'a str>, definition: &'a Definition, adapter: Option<&'a AdapterConfiguration>, notifications: Option<&'a NotificationPolicy>, revision: u64 },
    Apply(&'a Input),
    Refuse(Refusal),
}
impl Store {

    /// Latest grant is read again at command consumption; revocation also stops receipt replay.
    pub fn orchestrator_publishes_workflow(&self, workspace: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT json_extract(receipt,'$.change.enabled') FROM workflow_wire_receipts
            WHERE workspace=?1 AND json_extract(receipt,'$.change.kind')='grant' ORDER BY rowid DESC LIMIT 1",
            [workspace],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false))
    }
    pub fn workflow_publication_history(&self, workspace: &str) -> Result<Vec<Receipt>> {
        history(&self.conn(), workspace)
    }
    pub fn workflow_configuration(&self, workspace: &str) -> Result<Option<Configuration>> {
        let pin: Option<String> = self.conn().query_row(
            "SELECT pin FROM workflow_bundle_selections WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?;
        match pin { Some(pin) => self.workflow_bundle(&pin), None => Ok(None) }
    }
    /// The tracker-only template's bundle, stored if it is not yet, and its pin. It needs no publication:
    /// it is built in, runs nothing, and a ticket is tracked under it before any definition is chosen.
    pub fn workflow_ticket_bundle(&self, definition: &Definition) -> Result<String> {
        if !definition.is_ticket() { return Err(WorkflowError::IdentityConflict); }
        let pin = notification_configuration_pin(definition, None, None);
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO workflow_definitions(pin,definition) VALUES(?1,?2) ON CONFLICT(pin) DO NOTHING", params![definition.pin(), serde_json::to_string(definition)?])?;
        tx.execute("INSERT INTO workflow_bundles(pin,definition_pin,adapter) VALUES(?1,?2,?3) ON CONFLICT(pin) DO NOTHING", params![pin, definition.pin(), serde_json::to_string(&None::<AdapterConfiguration>)?])?;
        tx.commit()?;
        Ok(pin)
    }
    pub fn workflow_bundle(&self, pin: &str) -> Result<Option<Configuration>> {
        let raw: Option<(String, String)> = self.conn().query_row(
            "SELECT d.definition,b.adapter FROM workflow_bundles b JOIN workflow_definitions d ON d.pin=b.definition_pin WHERE b.pin=?1",
            [pin], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        raw.map(|(definition,adapter)| {
            let definition = serde_json::from_str(&definition)?;
            let adapter: Option<AdapterConfiguration> = serde_json::from_str(&adapter)?;
            let notifications: Option<String> = self.conn().query_row("SELECT policy FROM workflow_notification_policies WHERE bundle_pin=?1", [pin], |r| r.get(0)).optional()?;
            let notifications: Option<NotificationPolicy> = notifications.map(|s| serde_json::from_str(&s)).transpose()?;
            if notification_configuration_pin(&definition, adapter.as_ref(), notifications.as_ref()) != pin { return Err(WorkflowError::IdentityConflict); }
            Ok(Configuration { pin:pin.into(),definition,adapter,notifications })
        }).transpose()
    }
    pub fn workflow_definition(&self, pin: &str) -> Result<Option<Definition>> {
        let raw: Option<String> = self.conn().query_row("SELECT definition FROM workflow_definitions WHERE pin=?1", [pin], |r| r.get(0)).optional()?;
        raw.map(|r| serde_json::from_str(&r).map_err(Into::into)).transpose()
    }
    /// Called before any current-state validation; identity includes authenticated principal.
    pub fn workflow_wire_receipt(&self, workspace: &str, id: &str, payload: &str) -> Result<Option<Receipt>> {
        cached(&self.conn(), workspace, id, payload)
    }
    pub fn workflow_wire_write(&self, workspace: &str, id: &str, payload: &str, write: Write<'_>) -> Result<Receipt> {
        self.workflow_wire_write_by(workspace, id, payload, None, write)
    }
    /// `workflow_wire_write`, the receipt naming `actor`, who sent it (ov-1001). A retry answered from the
    /// receipt already written keeps the first sender's name: the receipt is immutable.
    pub fn workflow_wire_write_by(&self, workspace: &str, id: &str, payload: &str, actor: Option<&WorkflowActor>, write: Write<'_>) -> Result<Receipt> {
        if payload.len() > 65536 { return Err(WorkflowError::Oversized); }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !crate::engine_cutover::engine_allowed(&tx, self.engine_incarnation)? { return Err(WorkflowError::Authority); }
        if let Some(receipt) = cached(&tx, workspace, id, payload)? { return Ok(receipt); }
        let mut moved = Vec::new();
        let mut change = None;
        let mut receipt = match write {
            Write::PublicationGrant { enabled, revision } => {
                change = Some(serde_json::json!({"kind":"grant","enabled":enabled,"at":crate::tasks::now_millis()}));
                Receipt { change:None, id:id.into(),result:Ok(revision),effects:vec![],actor:None }
            }
            Write::Rollback { expected_pin, reason, revision } => {
                let current = selected(&tx, workspace)?;
                let previous = history(&tx, workspace)?.into_iter().find_map(|r| r.change.filter(|c| c["kind"] == "publication"));
                let result = match previous.as_ref().and_then(|c| c["previous"].as_str()) {
                    Some(pin) if current.as_deref() == Some(expected_pin) => {
                        tx.execute("UPDATE workflow_bundle_selections SET pin=?1 WHERE workspace=?2",params![pin,workspace])?;
                        change = Some(publication_change(&tx, workspace, current.as_deref(), pin, reason, actor.is_some())?);
                        Ok(revision)
                    }
                    _ => Err(Refusal::Stale),
                };
                Receipt { change:None,id:id.into(),result,effects:vec![],actor:None }
            }
            Write::Apply(input) => apply_in(&tx, input, &serde_json::to_string(input)?, actor.map(WorkflowActor::board_actor).as_ref(), &mut moved)?,
            Write::Refuse(reason) => Receipt { change:None, id: id.into(), result: Err(reason), effects: vec![], actor: None },
            Write::Publish { notifications, expected_pin, definition, adapter, revision } => {
                let current: Option<String> = tx.query_row("SELECT pin FROM workflow_bundle_selections WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?;
                let result = if current.as_deref() != expected_pin { Err(Refusal::Stale) }
                    else { definition.validate().map(|()| revision) };
                if result.is_ok() {
                    let pin = notification_configuration_pin(definition, adapter, notifications);
                    let definition_pin = definition.pin();
                    tx.execute("INSERT INTO workflow_definitions(pin,definition) VALUES(?1,?2) ON CONFLICT(pin) DO NOTHING", params![definition_pin, serde_json::to_string(definition)?])?;
                    tx.execute("INSERT INTO workflow_bundles(pin,definition_pin,adapter) VALUES(?1,?2,?3) ON CONFLICT(pin) DO NOTHING", params![pin,definition_pin,serde_json::to_string(&adapter)?])?;
                    if let Some(policy) = notifications {
                        tx.execute("INSERT INTO workflow_notification_policies(bundle_pin,policy) VALUES(?1,?2) ON CONFLICT DO NOTHING", params![pin,serde_json::to_string(policy)?])?;
                    }
                    let reason = serde_json::from_str::<serde_json::Value>(payload).ok()
                        .and_then(|v| v.get(1)?.get("reason")?.as_str().map(str::to_owned))
                        .unwrap_or_else(|| "Owner published workflow".into());
                    change = Some(publication_change(&tx, workspace, current.as_deref(), &pin, &reason, actor.is_some())?);
                    tx.execute("INSERT INTO workflow_bundle_selections(workspace,pin) VALUES(?1,?2) ON CONFLICT(workspace) DO UPDATE SET pin=excluded.pin", params![workspace,pin])?;
                }
                Receipt { change:None, id: id.into(), result, effects: vec![], actor: None }
            }
        };
        receipt.actor = actor.cloned();
        receipt.change = change;
        tx.execute("INSERT INTO workflow_wire_receipts(workspace,id,payload,receipt) VALUES(?1,?2,?3,?4)", params![workspace,id,payload,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        drop(conn);
        self.announce_board_moves(&moved);
        Ok(receipt)
    }
}

fn selected(conn: &rusqlite::Connection, workspace: &str) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT pin FROM workflow_bundle_selections WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?)
}
fn history(conn: &rusqlite::Connection, workspace: &str) -> Result<Vec<Receipt>> {
    let mut stmt = conn.prepare("SELECT receipt FROM workflow_wire_receipts WHERE workspace=?1 AND json_type(receipt,'$.change')='object' ORDER BY rowid DESC")?;
    let raw = stmt.query_map([workspace], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter().map(|s| serde_json::from_str(&s).map_err(Into::into)).collect()
}
fn publication_change(tx: &Transaction<'_>, workspace: &str, previous: Option<&str>, pin: &str, reason: &str, notify: bool) -> Result<serde_json::Value> {
    // Bundle pins retain every adapter and notification setting. The readable diff contains definitions,
    // plus adapter/policy change flags, so a board notice never copies environment secrets.
    let definition = |pin: &str| -> Result<serde_json::Value> {
        let raw: String = tx.query_row("SELECT definition FROM workflow_definitions d JOIN workflow_bundles b ON b.definition_pin=d.pin WHERE b.pin=?1", [pin], |r| r.get(0))?;
        Ok(serde_json::from_str(&raw)?)
    };
    let component = |pin: &str, sql: &str| -> Result<Option<String>> {
        Ok(tx.query_row(sql, [pin], |r| r.get(0)).optional()?)
    };
    let diff = serde_json::json!({"definition":{"before":previous.map(&definition).transpose()?,"after":definition(pin)?},
        "adapter_changed":previous.map(|p|component(p,"SELECT adapter FROM workflow_bundles WHERE pin=?1")).transpose()?.flatten()!=component(pin,"SELECT adapter FROM workflow_bundles WHERE pin=?1")?,
        "notifications_changed":previous.map(|p|component(p,"SELECT policy FROM workflow_notification_policies WHERE bundle_pin=?1")).transpose()?.flatten()!=component(pin,"SELECT policy FROM workflow_notification_policies WHERE bundle_pin=?1")?});
    let at = crate::tasks::now_millis();
    let mut change = serde_json::json!({"kind":"publication","previous":previous,"pin":pin,"reason":reason,"at":at,"diff":diff});
    // Authenticated publications notify through a review card in the same transaction. Internal
    // fixture/bootstrap publications without a caller do not manufacture owner-facing board work.
    if notify && let Ok(ws) = uuid::Uuid::parse_str(workspace) {
        let row: Option<(Vec<u8>, String)> = tx.query_row("SELECT repository_id,task_prefix FROM workspaces WHERE id=?1", [ws.as_bytes().as_slice()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((repo, prefix)) = row {
            let id = uuid::Uuid::now_v7();
            let key = crate::tasks::next_key_under(tx, &prefix)?;
            let title = "Workflow published — review change";
            tx.execute("INSERT INTO tasks(id,repository_id,workspace_id,key,title,status,status_since,created_at,resource_version,intent) VALUES(?1,?2,?3,?4,?5,'in_review',?6,?6,1,?7)",
                params![id.as_bytes().as_slice(),repo,ws.as_bytes().as_slice(),key,title,at,format!("{reason}\nPrevious: {}\nPublished: {pin}\nRestore with: farcooler workflow rollback {workspace}",previous.unwrap_or("none"))])?;
            crate::tasks::insert_note(tx,id,crate::models::NoteKind::Created,crate::models::Actor::Manager,reason,&serde_json::json!({"workflow_publication_notice":true,"change":change}),None)
                .map_err(|e| WorkflowError::Board(e.to_string()))?;
            change["notice_task"] = serde_json::json!(id);
        }
    }
    Ok(change)
}
fn cached(conn: &rusqlite::Connection, workspace: &str, id: &str, payload: &str) -> Result<Option<Receipt>> {
    let previous: Option<(String,String)> = conn.query_row("SELECT payload,receipt FROM workflow_wire_receipts WHERE workspace=?1 AND id=?2", params![workspace,id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    previous.map(|(original,receipt)| {
        if original != payload { return Err(WorkflowError::IdentityConflict); }
        Ok(serde_json::from_str(&receipt)?)
    }).transpose()
}
pub(crate) fn migration_publication(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE workflow_definitions(pin TEXT PRIMARY KEY NOT NULL, definition TEXT NOT NULL);
        CREATE TABLE workflow_configurations(workspace TEXT PRIMARY KEY NOT NULL, pin TEXT NOT NULL REFERENCES workflow_definitions(pin));
        CREATE TABLE workflow_wire_receipts(workspace TEXT NOT NULL,id TEXT NOT NULL,payload TEXT NOT NULL,receipt TEXT NOT NULL,PRIMARY KEY(workspace,id));
        CREATE TRIGGER workflow_definitions_immutable_update BEFORE UPDATE ON workflow_definitions BEGIN SELECT RAISE(ABORT, 'immutable definition'); END;
        CREATE TRIGGER workflow_definitions_immutable_delete BEFORE DELETE ON workflow_definitions BEGIN SELECT RAISE(ABORT, 'immutable definition'); END;
        CREATE TRIGGER workflow_wire_receipts_immutable_update BEFORE UPDATE ON workflow_wire_receipts BEGIN SELECT RAISE(ABORT, 'immutable wire receipt'); END;
        CREATE TRIGGER workflow_wire_receipts_immutable_delete BEFORE DELETE ON workflow_wire_receipts BEGIN SELECT RAISE(ABORT, 'immutable wire receipt'); END;
        ")
}

pub(crate) fn migration_bundles(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch("CREATE TABLE workflow_bundles(pin TEXT PRIMARY KEY NOT NULL, definition_pin TEXT NOT NULL REFERENCES workflow_definitions(pin), adapter TEXT NOT NULL);
        CREATE TABLE workflow_bundle_selections(workspace TEXT PRIMARY KEY NOT NULL, pin TEXT NOT NULL REFERENCES workflow_bundles(pin));
        INSERT INTO workflow_bundles SELECT pin,pin,'null' FROM workflow_definitions;
        INSERT INTO workflow_bundle_selections SELECT workspace,pin FROM workflow_configurations;
        CREATE TRIGGER workflow_bundles_no_update BEFORE UPDATE ON workflow_bundles BEGIN SELECT RAISE(ABORT,'immutable bundle'); END;
        CREATE TRIGGER workflow_bundles_no_delete BEFORE DELETE ON workflow_bundles BEGIN SELECT RAISE(ABORT,'immutable bundle'); END;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_publication_rolls_back_definition_config_and_receipt_together() {
        let store=Store::open_in_memory().unwrap();
        store.conn().execute_batch("CREATE TRIGGER fail_publication BEFORE INSERT ON workflow_wire_receipts BEGIN SELECT RAISE(ABORT,'fixture crash'); END;").unwrap();
        let definition=Definition::tracker("owner");
        assert!(store.workflow_wire_write("ws","p","payload",Write::Publish { notifications:None, adapter:None, expected_pin:None,definition:&definition,revision:0 }).is_err());
        assert!(store.workflow_configuration("ws").unwrap().is_none());
        assert!(store.workflow_definition(&definition.pin()).unwrap().is_none());
        assert!(store.workflow_bundle(&definition.pin()).unwrap().is_none());
        store.conn().execute_batch("DROP TRIGGER fail_publication").unwrap();
        let receipt=store.workflow_wire_write("ws","p","payload",Write::Publish { notifications:None, adapter:None, expected_pin:None,definition:&definition,revision:0 }).unwrap();
        assert!(receipt.result.is_ok());
        assert!(store.workflow_bundle(&definition.pin()).unwrap().is_some());
        assert!(store.conn().execute("UPDATE workflow_bundles SET adapter='{}'",[]).is_err());
        assert!(store.conn().execute("UPDATE workflow_definitions SET definition='{}'",[]).is_err());
        assert!(store.conn().execute("DELETE FROM workflow_wire_receipts",[]).is_err());
        let refusal=store.workflow_wire_write("ws","p2","payload2",Write::Publish { notifications:None, adapter:None, expected_pin:None,definition:&definition,revision:0 }).unwrap();
        assert_eq!(refusal.result,Err(Refusal::Stale));
    }
    #[test]
    fn workflow_publication_legacy_none_payload_and_prebundle_receipt_survive_migration() {
        use farcooler_protocol::workflow_commands::{Action, Mutation};
        // The c401 producer had no adapter field. Keep that producer shape
        // independent of today's enum, rather than merely replaying new JSON.
        #[derive(Serialize)]
        #[serde(tag = "action")]
        enum OriginalAction {
            Publish { mutation: Mutation, expected_pin: Option<String>, definition: Definition },
        }
        let definition = Definition::tracker("owner"); let pin = definition.pin();
        let mutation = Mutation { id:"original".into(),revision:0,epoch:1,sequence:1 };
        let old = serde_json::to_string(&("owner",OriginalAction::Publish { mutation:mutation.clone(),
            expected_pin:None,definition:definition.clone() })).unwrap();
        let current = serde_json::to_string(&("owner",Action::Publish { reason:None, notifications:None, mutation,
            expected_pin:None,definition:definition.clone(),adapter:None })).unwrap();
        assert_eq!(old,current,"None must preserve original producer bytes, not add adapter:null");
        assert!(serde_json::from_str::<serde_json::Value>(&old).unwrap()[1].get("adapter").is_none());
        let receipt = Receipt { change:None, id:"wire:owner:original".into(),result:Ok(0),effects:vec![],actor:None };
        let raw_receipt = serde_json::to_string(&receipt).unwrap();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap(); migration_publication(&tx).unwrap();
        tx.execute("INSERT INTO workflow_definitions(pin,definition) VALUES(?1,?2)",
            params![pin,serde_json::to_string(&definition).unwrap()]).unwrap();
        tx.execute("INSERT INTO workflow_configurations(workspace,pin) VALUES('old',?1)",[&pin]).unwrap();
        tx.execute("INSERT INTO workflow_wire_receipts(workspace,id,payload,receipt) VALUES('old',?1,?2,?3)",
            params![receipt.id,old,raw_receipt]).unwrap();
        tx.commit().unwrap();
        let tx = conn.transaction().unwrap(); migration_bundles(&tx).unwrap(); tx.commit().unwrap();
        let bundle: (String,String,String) = conn.query_row(
            "SELECT b.pin,b.definition_pin,b.adapter FROM workflow_bundles b JOIN workflow_bundle_selections s ON s.pin=b.pin WHERE s.workspace='old'",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(bundle,(pin.clone(),pin,"null".into()));
        assert_eq!(cached(&conn,"old",&receipt.id,&current).unwrap(),Some(receipt.clone()));
        assert!(matches!(cached(&conn,"old",&receipt.id,&format!("{current} ")),Err(WorkflowError::IdentityConflict)));
        let after: (String,String) = conn.query_row("SELECT payload,receipt FROM workflow_wire_receipts WHERE workspace='old'",[],
            |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(after,(old,raw_receipt),"migration/replay must not rewrite original receipt bytes");
    }

}

#[cfg(test)]
mod ov1179_tests {
    use super::*;
    #[test]
    fn ov1179_every_publication_keeps_previous_diff_reason_and_actor() {
        let store = Store::open_in_memory().unwrap();
        let actor = WorkflowActor { person: "owner".into(), device: None, pane: None, orchestrator: false };
        let first = Definition::tracker("owner");
        let second = Definition::tracker("orchestrator");
        for (id, definition, expected) in [("first", &first, None), ("second", &second, Some(first.pin()))] {
            let receipt = store.workflow_wire_write_by("ws", id, id, Some(&actor), Write::Publish {
                expected_pin: expected.as_deref(), definition, adapter: None, notifications: None, revision: 0,
            }).unwrap();
            assert!(receipt.result.is_ok());
            let value = serde_json::to_value(receipt).unwrap();
            assert!(value["change"]["at"].is_i64(), "publication has no timestamp");
            assert_eq!(value["change"]["previous"], serde_json::json!(expected));
            assert!(value["change"]["diff"].is_object(), "publication has no diff");
            assert!(value["change"]["reason"].as_str().is_some_and(|s| !s.is_empty()));
            assert_eq!(value["actor"]["person"], "owner");
        }
    }
}

#[cfg(test)]
mod ov1179_rollback_tests {
    use super::*;
    #[test]
    fn ov1179_grant_revoke_rollback_and_receipts_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let actor = WorkflowActor { person:"owner".into(),device:Some("phone".into()),pane:None,orchestrator:false };
        let first = Definition::tracker("owner");
        let second = Definition::tracker("orchestrator");
        {
            let store = Store::open(&path).unwrap();
            assert!(!store.orchestrator_publishes_workflow("ws").unwrap());
            store.workflow_wire_write_by("ws","grant","g",Some(&actor),Write::PublicationGrant {enabled:true,revision:0}).unwrap();
            assert!(store.orchestrator_publishes_workflow("ws").unwrap());
            for (id,definition,expected) in [("a",&first,None),("b",&second,Some(first.pin()))] {
                assert!(store.workflow_wire_write_by("ws",id,id,Some(&actor),Write::Publish { expected_pin:expected.as_deref(),definition,adapter:None,notifications:None,revision:0 }).unwrap().result.is_ok());
            }
        }
        let store = Store::open(&path).unwrap();
        assert!(store.orchestrator_publishes_workflow("ws").unwrap());
        // A stale rollback cannot restore over a newer selection.
        let stale=store.workflow_wire_write_by("ws","stale","stale",Some(&actor),Write::Rollback {expected_pin:"old",reason:"Revert",revision:0}).unwrap();
        assert!(stale.result.is_err());
        let second_pin=second.pin();
        let rollback = || Write::Rollback {expected_pin:&second_pin,reason:"Revert a bad change",revision:0};
        let result=store.workflow_wire_write_by("ws","rollback","r",Some(&actor),rollback()).unwrap();
        assert!(result.result.is_ok());
        assert_eq!(store.workflow_configuration("ws").unwrap().unwrap().definition,first);
        assert_eq!(store.workflow_wire_write_by("ws","rollback","r",Some(&actor),rollback()).unwrap(),result);
        assert_eq!(result.change.as_ref().unwrap()["reason"],"Revert a bad change");
        let history=store.workflow_publication_history("ws").unwrap();
        assert_eq!(history.len(),4,"replays and refusals add no audit event");
        assert_eq!(history.last().unwrap().actor.as_ref(),Some(&actor));
        store.workflow_wire_write_by("ws","revoke","off",Some(&actor),Write::PublicationGrant {enabled:false,revision:0}).unwrap();
        assert!(!store.orchestrator_publishes_workflow("ws").unwrap());
        assert!(store.workflow_bundle(&second.pin()).unwrap().is_some(),"rollback retains the replaced bundle");
    }
    /// ov-1201: a step's environment may be dropped or kept as it was, never added to, on a ticket step or a
    /// group's; no adapter is the same as none, and any other adapter is a change.
    #[test]
    fn ov1201_runs_the_same_only_with_the_same_commands_and_environments() {
        let mut before = Definition::tracker("owner");
        before.stages[0].operation.environment.insert("KEEP".into(), "1".into());
        let previous = Configuration { pin: "p".into(), definition: before.clone(), adapter: None, notifications: None };
        assert!(runs_the_same(Some(&previous), &before, None));
        let mut dropped = before.clone(); dropped.stages[0].operation.environment.clear();
        assert!(runs_the_same(Some(&previous), &dropped, None));
        let mut added = before.clone(); added.stages[0].operation.environment.insert("GH_TOKEN".into(), "t".into());
        assert!(!runs_the_same(Some(&previous), &added, None));
        let mut moved = before.clone(); moved.stages[0].id = "elsewhere".into();
        assert!(!runs_the_same(Some(&previous), &moved, None), "another step's environment");
        let mut grouped = before.clone();
        grouped.groups.insert("g".into(), farcooler_workflow::GroupDef { limit: None, exclusive: "main".into(), steps: before.stages.clone() });
        assert!(!runs_the_same(Some(&previous), &grouped, None), "a group step's environment is its own");
        assert!(!runs_the_same(None, &before, None), "nothing to keep");
        assert!(runs_the_same(None, &Definition::tracker("orchestrator"), None));
    }
    /// ov-1201 (review a2472ebc): with the adapter unchanged, every step that runs something stays behind the
    /// deciders it was behind: its own approver, and a person's review whose `passed` route is the way to it, a
    /// group's steps through the join that queues into the group.
    #[test]
    fn ov1201_runs_the_same_only_behind_the_same_deciders() {
        use farcooler_workflow::{GroupDef, OperationSpec, Progression, Reviewer, Stage, StageKind, Target};
        let step = |id: &str, kind: StageKind, argument: &str| Stage { id: id.into(), kind, progression: Progression::Automatic,
            max_attempts: 1, operation: OperationSpec { adapter_version: "v1".into(), arguments: vec![argument.into()], environment: Default::default() },
            routes: Default::default(), limit: 0 };
        let route = |mut s: Stage, label: &str, to: &str| { s.routes.insert(label.into(), Target::Step(to.into())); s };
        let approved = |mut s: Stage, by: &str| { s.progression = Progression::AwaitApproval { principal: by.into() }; s };
        let review = step("review", StageKind::Review(Reviewer::Person("owner".into())), "person:owner");
        let definition = |stages: Vec<Stage>| { let mut d = Definition::tracker("owner"); d.schema = 2; d.stages = stages; d };
        let build = step("build", StageKind::Build, "");
        let deploy = step("deploy", StageKind::Check, "deploy");
        let lint = step("lint", StageKind::Check, "lint");
        let configuration = |definition: Definition| Configuration { pin: "p".into(), definition, adapter: None, notifications: None };

        // The deploy waits on the owner's approval.
        let previous = configuration(definition(vec![build.clone(), approved(deploy.clone(), "owner")]));
        assert!(runs_the_same(Some(&previous), &previous.definition, None));
        assert!(runs_the_same(Some(&previous), &definition(vec![approved(build.clone(), "orchestrator"), approved(deploy.clone(), "owner")]), None),
            "another decider in front");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), deploy.clone()]), None), "approval dropped");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), approved(deploy.clone(), "orchestrator")]), None),
            "approval reassigned");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), approved(deploy.clone(), "owner"), lint.clone()]), None),
            "a command no step ran");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), step("deploy-first", StageKind::Check, "deploy"),
            approved(deploy.clone(), "owner")]), None), "the same command, unapproved, on another step");
        assert!(runs_the_same(Some(&previous), &definition(vec![build.clone(), approved(deploy.clone(), "owner"),
            step("deploy-again", StageKind::Check, "deploy")]), None), "again, after the owner's approval");

        // The deploy is behind the owner's review.
        let previous = configuration(definition(vec![build.clone(), lint.clone(), review.clone(), deploy.clone()]));
        assert!(runs_the_same(Some(&previous), &previous.definition, None));
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), route(route(lint.clone(), "passed", "deploy"), "rejected", "review"),
            review.clone(), deploy.clone()]), None), "routed past the review");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(), lint.clone(), route(review.clone(), "rejected", "deploy"),
            deploy.clone()]), None), "routed through a rejection");
        assert!(runs_the_same(Some(&previous), &definition(vec![build.clone(), lint.clone(), route(review.clone(), "rejected", "build"),
            deploy.clone()]), None), "a rejection sent back to the build");
        assert!(!runs_the_same(Some(&previous), &definition(vec![build.clone(),
            step("review", StageKind::Review(Reviewer::Orchestrator("orchestrator".into())), "orchestrator:orchestrator"),
            deploy.clone()]), None), "review reassigned");

        // A group's deploy is behind the review in front of its join.
        let grouped = |ticket: Vec<Stage>| { let mut d = definition(ticket);
            d.groups.insert("g".into(), GroupDef { limit: None, exclusive: "main".into(), steps: vec![step("run", StageKind::Run, "deploy")] }); d };
        let join = step("join", StageKind::Join { group: "g".into(), adopt: false }, "g");
        let previous = configuration(grouped(vec![build.clone(), lint.clone(), review.clone(), join.clone()]));
        assert!(runs_the_same(Some(&previous), &previous.definition, None));
        assert!(!runs_the_same(Some(&previous), &grouped(vec![build.clone(), route(route(lint.clone(), "passed", "join"), "rejected", "review"),
            review.clone(), join.clone()]), None), "routed past the review to the join");
        assert!(runs_the_same(Some(&previous), &grouped(vec![build.clone(), lint.clone(), review.clone(), approved(join.clone(), "orchestrator")]), None),
            "another decider in front of the join");
        // A split restarts the group at its step, so that is a way to it too.
        let split = |gate: Stage| { let mut d = definition(vec![build.clone(), join.clone()]);
            d.groups.insert("g".into(), GroupDef { limit: None, exclusive: "main".into(),
                steps: vec![gate, review.clone(), step("run", StageKind::Run, "deploy")] }); d };
        let previous = configuration(split(step("gate", StageKind::Run, "lint")));
        let mut splitting = step("gate", StageKind::Run, "lint");
        splitting.routes.insert("rejected".into(), Target::Split { step: "run".into(), otherwise: Box::new(Target::Fail) });
        assert!(!runs_the_same(Some(&previous), &split(splitting), None), "split past the review");
        let previous = configuration(grouped(vec![build.clone(), lint.clone(), review.clone(), join.clone()]));
        let mut unjoined = grouped(vec![build.clone(), lint.clone(), review.clone()]);
        unjoined.stages.push(step("other", StageKind::Join { group: "h".into(), adopt: false }, "h"));
        assert!(runs_the_same(Some(&previous), &unjoined, None), "a group no join queues into runs nothing");
    }
}


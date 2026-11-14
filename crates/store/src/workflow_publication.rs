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
}

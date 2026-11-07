//! Publication and authenticated wire receipts share the workflow transaction.
use crate::{Store, workflow::{Receipt, Result, WorkflowError, apply_in}};
use farcooler_protocol::workflow_config::AdapterConfiguration;
use farcooler_workflow::{Definition, Input, Refusal};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration { pub pin: String, pub definition: Definition, pub adapter: Option<AdapterConfiguration> }
pub fn configuration_pin(definition: &Definition, adapter: Option<&AdapterConfiguration>) -> String {
    adapter.map_or_else(|| definition.pin(), |a| farcooler_workflow::digest(&("workflow-bundle/v1", definition, a)))
}
pub enum Write<'a> {
    Publish { expected_pin: Option<&'a str>, definition: &'a Definition, adapter: Option<&'a AdapterConfiguration>, revision: u64 },
    Apply(&'a Input),
    Refuse(Refusal),
}
impl Store {
    pub fn workflow_configuration(&self, workspace: &str) -> Result<Option<Configuration>> {
        let pin: Option<String> = self.conn().query_row(
            "SELECT pin FROM workflow_bundle_selections WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?;
        match pin { Some(pin) => self.workflow_bundle(&pin), None => Ok(None) }
    }
    pub fn workflow_bundle(&self, pin: &str) -> Result<Option<Configuration>> {
        let raw: Option<(String, String)> = self.conn().query_row(
            "SELECT d.definition,b.adapter FROM workflow_bundles b JOIN workflow_definitions d ON d.pin=b.definition_pin WHERE b.pin=?1",
            [pin], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        raw.map(|(definition,adapter)| {
            let definition = serde_json::from_str(&definition)?;
            let adapter: Option<AdapterConfiguration> = serde_json::from_str(&adapter)?;
            if configuration_pin(&definition, adapter.as_ref()) != pin { return Err(WorkflowError::IdentityConflict); }
            Ok(Configuration { pin:pin.into(),definition,adapter })
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
        if payload.len() > 65536 { return Err(WorkflowError::Oversized); }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(receipt) = cached(&tx, workspace, id, payload)? { return Ok(receipt); }
        let receipt = match write {
            Write::Apply(input) => apply_in(&tx, input, &serde_json::to_string(input)?)?,
            Write::Refuse(reason) => Receipt { id: id.into(), result: Err(reason), effects: vec![] },
            Write::Publish { expected_pin, definition, adapter, revision } => {
                let current: Option<String> = tx.query_row("SELECT pin FROM workflow_bundle_selections WHERE workspace=?1", [workspace], |r| r.get(0)).optional()?;
                let result = if current.as_deref() != expected_pin { Err(Refusal::Stale) }
                    else { definition.validate().map(|()| revision) };
                if result.is_ok() {
                    let pin = configuration_pin(definition, adapter);
                    let definition_pin = definition.pin();
                    tx.execute("INSERT INTO workflow_definitions(pin,definition) VALUES(?1,?2) ON CONFLICT(pin) DO NOTHING", params![definition_pin, serde_json::to_string(definition)?])?;
                    tx.execute("INSERT INTO workflow_bundles(pin,definition_pin,adapter) VALUES(?1,?2,?3) ON CONFLICT(pin) DO NOTHING", params![pin,definition_pin,serde_json::to_string(&adapter)?])?;
                    tx.execute("INSERT INTO workflow_bundle_selections(workspace,pin) VALUES(?1,?2) ON CONFLICT(workspace) DO UPDATE SET pin=excluded.pin", params![workspace,pin])?;
                }
                Receipt { id: id.into(), result, effects: vec![] }
            }
        };
        tx.execute("INSERT INTO workflow_wire_receipts(workspace,id,payload,receipt) VALUES(?1,?2,?3,?4)", params![workspace,id,payload,serde_json::to_string(&receipt)?])?;
        tx.commit()?;
        Ok(receipt)
    }
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
        assert!(store.workflow_wire_write("ws","p","payload",Write::Publish { adapter:None, expected_pin:None,definition:&definition,revision:0 }).is_err());
        assert!(store.workflow_configuration("ws").unwrap().is_none());
        assert!(store.workflow_definition(&definition.pin()).unwrap().is_none());
        assert!(store.workflow_bundle(&definition.pin()).unwrap().is_none());
        store.conn().execute_batch("DROP TRIGGER fail_publication").unwrap();
        let receipt=store.workflow_wire_write("ws","p","payload",Write::Publish { adapter:None, expected_pin:None,definition:&definition,revision:0 }).unwrap();
        assert!(receipt.result.is_ok());
        assert!(store.workflow_bundle(&definition.pin()).unwrap().is_some());
        assert!(store.conn().execute("UPDATE workflow_bundles SET adapter='{}'",[]).is_err());
        assert!(store.conn().execute("UPDATE workflow_definitions SET definition='{}'",[]).is_err());
        assert!(store.conn().execute("DELETE FROM workflow_wire_receipts",[]).is_err());
        let refusal=store.workflow_wire_write("ws","p2","payload2",Write::Publish { adapter:None, expected_pin:None,definition:&definition,revision:0 }).unwrap();
        assert_eq!(refusal.result,Err(Refusal::Stale));
    }
}

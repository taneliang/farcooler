//! One-time publication upgrade for existing Sol workflows (ov-1356).
use crate::{Store, workflow::{Result, WorkflowError}, workflow_publication::Write};

impl Store {
    /// Publish 6.2 Sol alongside 6.1 Sol, only on steps already offering Sol. Existing bundles and
    /// running items remain immutable; new starts must inspect the new publication pin. The durable
    /// migration receipt makes this one-shot, including after an owner rolls the publication back.
    pub fn workflow_upgrade_sol_62(&self) -> Result<()> {
        let workspaces = {
            let conn = self.conn();
            let mut query = conn.prepare("SELECT workspace FROM workflow_bundle_selections")?;
            query.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let id = "migration:ov-1356";
        let payload = r#"["runtime",{"reason":"ov-1356: offer GPT-6.2 Sol on steps offering GPT-6.1 Sol"}]"#;
        for workspace in workspaces {
            if self.workflow_wire_receipt(&workspace, id, payload)?.is_some() { continue; }
            let Some(mut configuration) = self.workflow_configuration(&workspace)? else { continue };
            let mut changed = false;
            for models in configuration.definition.models.values_mut() {
                if models.iter().any(|m| m == "codex:gpt-6.1-sol") && !models.iter().any(|m| m == "codex:gpt-6.2-sol") {
                    models.push("codex:gpt-6.2-sol".into());
                    changed = true;
                }
            }
            if !changed { continue; }
            // An explicit model-count limit can leave no room for another choice. Keep that
            // owner's publication intact rather than refusing every daemon startup.
            if let Err(refusal) = configuration.definition.validate() {
                tracing::warn!(%workspace, ?refusal, "Sol upgrade needs a manual publication within its configured limits");
                continue;
            }
            let revision = self.workflow_state(&workspace)?.map_or(0, |s| s.revision);
            let receipt = self.workflow_wire_write(&workspace, id, payload, Write::Publish {
                expected_pin: Some(&configuration.pin), definition: &configuration.definition,
                adapter: configuration.adapter.as_ref(), notifications: configuration.notifications.as_ref(), revision,
            })?;
            if receipt.result.is_err() { return Err(WorkflowError::IdentityConflict); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_publication::{Configuration, notification_configuration_pin};

    #[test]
    fn workflow_sol_upgrade_publishes_existing_main_once_and_preserves_choices_and_rollback() {
        let store = Store::open_in_memory().unwrap();
        let tracker = farcooler_workflow::Definition::tracker("owner");
        assert!(store.workflow_wire_write("non-sol", "original", "original", Write::Publish {
            expected_pin: None, definition: &tracker, adapter: None, notifications: None, revision: 0,
        }).unwrap().result.is_ok());
        let bundle: serde_json::Value = serde_json::from_str(include_str!("../../../.claude/agent/reports/ov-1071/workflow-main.json")).unwrap();
        let mut old: Configuration = serde_json::from_value(serde_json::json!({
            "pin":"", "definition":bundle["definition"], "adapter":bundle["adapter"],
            "notifications":farcooler_protocol::workflow_config::NotificationPolicy::default(),
        })).unwrap();
        for models in old.definition.models.values_mut() { models.retain(|m| m != "codex:gpt-6.2-sol"); }
        // An intentionally Claude-only step must stay Claude-only.
        old.definition.models.get_mut("gate-0").unwrap().retain(|m| m.starts_with("claude:"));
        old.pin = notification_configuration_pin(&old.definition, old.adapter.as_ref(), old.notifications.as_ref());
        let mut full = old.definition.clone();
        full.limits = Some(farcooler_workflow::scheduling::Limits {
            models: full.models.values().map(Vec::len).max().unwrap(), ..Default::default()
        });
        assert!(store.workflow_wire_write("full", "original", "original", Write::Publish {
            expected_pin: None, definition: &full, adapter: None, notifications: None, revision: 0,
        }).unwrap().result.is_ok());
        assert!(store.workflow_wire_write("main", "original", "original", Write::Publish {
            expected_pin: None, definition: &old.definition, adapter: old.adapter.as_ref(), notifications: old.notifications.as_ref(), revision: 0,
        }).unwrap().result.is_ok());
        store.conn().execute_batch("CREATE TRIGGER fail_upgrade BEFORE INSERT ON workflow_wire_receipts BEGIN SELECT RAISE(ABORT,'fixture crash'); END;").unwrap();
        assert!(store.workflow_upgrade_sol_62().is_err());
        assert_eq!(store.workflow_configuration("main").unwrap().unwrap(), old, "a failed migration is atomic");
        store.conn().execute_batch("DROP TRIGGER fail_upgrade").unwrap();
        store.workflow_upgrade_sol_62().unwrap();
        let updated = store.workflow_configuration("main").unwrap().unwrap();
        assert_eq!(store.workflow_configuration("full").unwrap().unwrap().definition, full, "an explicit model limit survives the upgrade");
        assert_eq!(store.workflow_configuration("non-sol").unwrap().unwrap().pin, tracker.pin());
        assert_eq!(store.workflow_publication_history("non-sol").unwrap().len(), 1);
        assert_ne!(updated.pin, old.pin);
        let mut expected = old.clone();
        expected.definition.models.get_mut("build").unwrap().push("codex:gpt-6.2-sol".into());
        expected.pin = updated.pin.clone();
        assert_eq!(updated, expected, "only the Sol step's choices change; defaults, adapter and notifications survive");
        assert_eq!(store.workflow_bundle(&old.pin).unwrap().unwrap(), old, "the original bundle remains pinned");
        let history = store.workflow_publication_history("main").unwrap();
        assert_eq!(history.len(), 2);
        let change = history[0].change.as_ref().unwrap();
        assert_eq!(change["previous"], old.pin);
        assert_eq!(change["pin"], updated.pin);
        assert!(change["reason"].as_str().unwrap().contains("ov-1356"));
        store.workflow_upgrade_sol_62().unwrap();
        assert_eq!(store.workflow_publication_history("main").unwrap(), history);
        assert!(store.workflow_wire_write("main", "rollback", "rollback", Write::Rollback {
            expected_pin: &updated.pin, reason: "Keep previous choices", revision: 0,
        }).unwrap().result.is_ok());
        store.workflow_upgrade_sol_62().unwrap();
        assert_eq!(store.workflow_configuration("main").unwrap().unwrap(), old, "a later rollback is respected");
    }
}

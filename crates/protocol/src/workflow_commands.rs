//! Closed command vocabulary, shared by the real CLI producer and daemon parser.
pub use farcooler_workflow::{Definition, valid_rework_reason};
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 60 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub id: String,
    pub revision: u64,
    pub epoch: u64,
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Action {
    Get,
    Publish { mutation: Mutation, expected_pin: Option<String>, definition: Definition, #[serde(default, skip_serializing_if = "Option::is_none")] adapter: Option<Box<crate::workflow_config::AdapterConfiguration>>, #[serde(default, skip_serializing_if = "Option::is_none")] notifications: Option<crate::workflow_config::NotificationPolicy> },
    Enqueue { mutation: Mutation, item: String, pin: String },
    Approve { mutation: Mutation, item: String, run: String, pin: String, candidate: Option<String> },
    Review { mutation: Mutation, item: String, run: String, pin: String, evidence: farcooler_workflow::Evidence, passed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")] reason: Option<String> },
    Cancel { mutation: Mutation, item: String, run: String, pin: String },
    /// Run a failed item's step again. A controller's command, like cancel.
    Retry { mutation: Mutation, item: String, run: String, pin: String },
}
impl Action {
    pub fn method(&self) -> &'static str {
        match self {
            Self::Get => "workflow.get",
            Self::Publish { .. } => "workspace.set_workflow",
            Self::Enqueue { .. } => "workflow.enqueue",
            Self::Approve { .. } => "workflow.approve",
            Self::Review { .. } => "workflow.review",
            Self::Cancel { .. } => "workflow.cancel",
            Self::Retry { .. } => "workflow.retry",
        }
    }
}

/// Reject nested fields that the pure definition's older parser might otherwise drop.
pub fn parse(raw: &str) -> Result<Action, &'static str> {
    if raw.len() > MAX_BYTES { return Err("oversized workflow command"); }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| "invalid JSON")?;
    let action: Action = serde_json::from_value(value.clone()).map_err(|_| "invalid action")?;
    if serde_json::to_value(&action).map_err(|_| "invalid encoding")? != value { return Err("unknown workflow fields"); }
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_reasoned_wire_preserves_legacy_and_refuses_unknown_fields() {
        let legacy = r#"{"action":"Review","mutation":{"id":"same","revision":5,"epoch":1,"sequence":2},"item":"a","run":"run","pin":"pin","evidence":{"candidate":"c","manifest":"m","receipt":"r","policy_pin":"p"},"passed":false}"#;
        assert_eq!(serde_json::to_string(&parse(legacy).unwrap()).unwrap(),legacy);
        let mut value:serde_json::Value=serde_json::from_str(legacy).unwrap();
        value["reason"]=serde_json::json!("  修复 ");
        let parsed=parse(&value.to_string()).unwrap();
        assert!(matches!(parsed,Action::Review{reason:Some(ref r),..} if r=="  修复 "));
        value["reason"]=serde_json::json!({"grant":"Controller"});assert!(parse(&value.to_string()).is_err());
        value["reason"]=serde_json::Value::Null;assert!(parse(&value.to_string()).is_err());
        value.as_object_mut().unwrap().remove("reason");value["authority"]=serde_json::json!("owner");assert!(parse(&value.to_string()).is_err());
    }
    #[test]
    fn workflow_wire_carries_retry_and_routed_definitions_exactly() {
        let retry = Action::Retry { mutation:Mutation { id:"r".into(),revision:3,epoch:1,sequence:2 },item:"a".into(),run:"run".into(),pin:"pin".into() };
        let raw = serde_json::to_string(&retry).unwrap();
        assert_eq!(parse(&raw).unwrap(),retry);
        assert_eq!(retry.method(),"workflow.retry");
        let mut definition = Definition::coding();
        definition.schema = 2; definition.max_reworks = 0; definition.rework_target = None;
        definition.stages[2].routes.insert("rejected".into(), farcooler_workflow::Target::Step("stage-1".into()));
        definition.stages[2].limit = 2;
        let publish = Action::Publish { notifications:None, adapter:None, mutation:Mutation { id:"p".into(),revision:0,epoch:1,sequence:1 },expected_pin:None,definition };
        assert_eq!(parse(&serde_json::to_string(&publish).unwrap()).unwrap(),publish);
    }
    #[test]
    fn workflow_wire_parser_rejects_authority_result_and_unknown_nested_fields() {
        let action = Action::Publish { notifications:None, adapter:None, mutation: Mutation { id:"publish".into(),revision:0,epoch:1,sequence:1 }, expected_pin:None,definition:Definition::tracker("owner") };
        let raw = serde_json::to_string(&action).unwrap();
        assert_eq!(parse(&raw).unwrap(),action);
        let mut value:serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["grant"] = serde_json::json!("Controller");
        assert!(parse(&value.to_string()).is_err());
        value.as_object_mut().unwrap().remove("grant");
        value["definition"]["stages"][0]["operation"]["result"] = serde_json::json!("Passed");
        assert!(parse(&value.to_string()).is_err());
        for raw in [r#"{"action":"Resolve"}"#,r#"{"action":"Observe"}"#,r#"{"action":"Get","principal":"owner"}"#] { assert!(parse(raw).is_err()); }
        assert!(parse(&"x".repeat(MAX_BYTES+1)).is_err());
    }
}

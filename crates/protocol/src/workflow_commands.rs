//! Closed command vocabulary, shared by the real CLI producer and daemon parser.
use farcooler_workflow::Definition;
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
    Publish { mutation: Mutation, expected_pin: Option<String>, definition: Definition },
    Enqueue { mutation: Mutation, item: String, pin: String },
    Approve { mutation: Mutation, item: String, run: String, pin: String, candidate: Option<String> },
    Review { mutation: Mutation, item: String, run: String, pin: String, evidence: farcooler_workflow::Evidence, passed: bool },
    Cancel { mutation: Mutation, item: String, run: String, pin: String },
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
    fn workflow_wire_parser_rejects_authority_result_and_unknown_nested_fields() {
        let action = Action::Publish { mutation: Mutation { id:"publish".into(),revision:0,epoch:1,sequence:1 }, expected_pin:None,definition:Definition::tracker("owner") };
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

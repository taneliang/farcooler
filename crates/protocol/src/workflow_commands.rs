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
    /// Queue a ticket waiting at its join step into the open group. The orchestrator decides.
    Queue { mutation: Mutation, item: String, run: String, pin: String },
    /// Pull a queued ticket out of a group that has not left yet.
    Leave { mutation: Mutation, item: String, run: String, pin: String },
    /// A person's word on a push whose outcome was lost, believed only after the remote is read:
    /// `landed` says the pushed commit is on the target. Person-only; it pushes nothing.
    Settle { mutation: Mutation, item: String, run: String, pin: String, landed: bool },
    /// Bring a ticket under the engine with no run: the tracker-only template, in its first column.
    Track { mutation: Mutation, item: String, #[serde(default, skip_serializing_if = "Option::is_none")] column: Option<String> },
    /// Move a tracker-only ticket to a column (`backlog`, `todo`, `doing`, `review`, `decision`), or close it (`done`, `cancelled`).
    Move { mutation: Mutation, item: String, run: String, pin: String, to: String },
    /// Start a run on a tracker-only ticket under the workspace's published definition `start_pin`. `models`
    /// picks a build step's model, by step id, from the ones the definition lists; a step left out runs its default (ov-966).
    ///
    /// `worktree` names the worktree and branch the run makes and works in, which the app lists, from main as it
    /// is at Start (ov-967). Left out, the run works in a private tree.
    Start { mutation: Mutation, item: String, run: String, pin: String, start_pin: String,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")] models: std::collections::BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] worktree: Option<NamedTree> },
}
/// A worktree the orchestrator names for a run: what the app calls it, and its new branch (ov-967).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedTree { pub name: String, pub branch: String }
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
            Self::Queue { .. } => "workflow.queue",
            Self::Leave { .. } => "workflow.leave",
            Self::Settle { .. } => "workflow.settle",
            Self::Track { .. } => "workflow.track",
            Self::Move { .. } => "workflow.move",
            Self::Start { .. } => "workflow.start",
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
        let settle = r#"{"action":"Settle","mutation":{"id":"s","revision":3,"epoch":1,"sequence":2},"item":"a","run":"run","pin":"pin","landed":true}"#;
        assert_eq!(serde_json::to_string(&parse(settle).unwrap()).unwrap(),settle);
        assert_eq!(parse(settle).unwrap().method(),"workflow.settle");
        assert!(parse(&settle.replace(r#","landed":true"#,"")).is_err(),"a settlement says which way");
        for action in ["Queue","Leave"] {
            let raw = format!(r#"{{"action":"{action}","mutation":{{"id":"q","revision":3,"epoch":1,"sequence":2}},"item":"a","run":"run","pin":"pin"}}"#);
            let parsed = parse(&raw).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(),raw);
            assert_eq!(parsed.method(),format!("workflow.{}",action.to_lowercase()));
        }
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

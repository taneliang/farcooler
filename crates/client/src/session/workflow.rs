//! Authenticated workflow driver. Renderers forward opaque actions and retain prepared bytes.
use super::*;
use crate::workflow_presentation::{self as cards, Action, Capabilities, Intent, Presentation};
use farcooler_protocol::{capability, v1::WorkflowCommand, workflow_commands::{self as wire, Mutation}};
use farcooler_workflow::{digest, Command, State};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowContext { pub capabilities: Capabilities, pub binding: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub schema: u32,
    pub state: State,
    pub configuration: serde_json::Value,
    pub notices: serde_json::Value,
    pub context: WorkflowContext,
    pub presentation: Presentation,
}
/// Identity is observation plus ordering, never authority. Persist for the life of the intent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCommandIdentity { pub id: String, pub sequence: u64, pub revision: u64, pub epoch: u64 }
/// `json` is sent unchanged, including after replacing the Session on reconnect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWorkflowAction { pub schema: u32, pub workspace: Uuid, pub json: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PreparedWorkflowResult {
    Inspection { intent: Intent },
    Mutation { prepared: PreparedWorkflowAction },
}
fn invalid(message: impl Into<String>) -> SessionError { SessionError::Protocol(message.into()) }
fn stale() -> SessionError {
    SessionError::Refused { code: farcooler_protocol::v1::ErrorCode::ResourceConflict as i32,
        retryable: false, message: "Workflow action is stale; inspect a fresh snapshot".into(), what: "workflow_stale".into() }
}

fn project(state: &State, context: &WorkflowContext) -> Result<Presentation, SessionError> {
    let mut view = cards::project(state, &context.capabilities).map_err(|e| invalid(format!("workflow {e:?}")))?;
    if context.binding.is_empty() { return Err(invalid("missing workflow admission binding")); }
    for card in &mut view.runs { for action in &mut card.actions {
        action.guard = digest(&("workflow-session/v1", &action.guard, &context.binding));
    }}
    Ok(view)
}
fn prepare(snapshot: &WorkflowSnapshot, workspace: Uuid, item: &str, selected: Action,
    identity: WorkflowCommandIdentity) -> Result<PreparedWorkflowResult, SessionError>
{
    if snapshot.state.workspace != workspace.to_string() || identity.revision != snapshot.state.revision
        || identity.epoch != snapshot.state.epoch { return Err(stale()); }
    let view = project(&snapshot.state, &snapshot.context)?;
    let card = view.runs.iter().find(|c| c.item == item).ok_or_else(stale)?;
    if !card.actions.contains(&selected) { return Err(stale()); }
    let pure = cards::project(&snapshot.state, &snapshot.context.capabilities).map_err(|_| stale())?;
    let original = pure.runs.iter().find(|c| c.item == item).and_then(|c|
        c.actions.iter().find(|a| a.kind == selected.kind && a.label == selected.label)).ok_or_else(stale)?;
    let intent = cards::action(&snapshot.state, &snapshot.context.capabilities, item, original).map_err(|_| stale())?;
    let Intent::Command(command) = intent else { return Ok(PreparedWorkflowResult::Inspection { intent }); };
    if identity.id.is_empty() || identity.id.len() > 100 { return Err(invalid("invalid workflow command identity")); }
    let mutation = Mutation { id: identity.id, sequence: identity.sequence, revision: identity.revision, epoch: identity.epoch };
    // The single finite Intent -> wire mapping for every transport binding.
    let action = match command {
        Command::Approve { item, run, pin, candidate } => wire::Action::Approve { mutation, item, run, pin, candidate },
        Command::Review { item, run, evidence, passed } => wire::Action::Review {
            mutation, item, run, pin: card.definition_pin.clone(), evidence, passed },
        Command::Cancel { item } => wire::Action::Cancel { mutation, item, run: card.run.clone(), pin: card.definition_pin.clone() },
        _ => return Err(invalid("unsupported workflow intent")),
    };
    let json = serde_json::to_string(&action).map_err(|e| invalid(e.to_string()))?;
    wire::parse(&json).map_err(invalid)?;
    Ok(PreparedWorkflowResult::Mutation { prepared: PreparedWorkflowAction { schema: 1, workspace, json } })
}
impl Session {
    async fn workflow_wire(&self, workspace: Uuid, json: String) -> Result<serde_json::Value, SessionError> {
        let action = wire::parse(&json).map_err(invalid)?;
        let value = self.value_requiring(action.method(), Some(workspace),
            Some(request::Payload::WorkflowCommand(WorkflowCommand { schema: 1, json })),
            vec![capability::WORKFLOW_COMMANDS.into()]).await?;
        match value {
            result::Value::WorkflowReply(reply) if reply.schema == 1 => serde_json::from_str(&reply.json).map_err(|e| invalid(e.to_string())),
            _ => Err(invalid("invalid workflow reply")),
        }
    }
    /// Fetch authenticated state, admission context and schema2 cards.
    pub async fn workflow_snapshot(&self, workspace: Uuid) -> Result<WorkflowSnapshot, SessionError> {
        let raw = self.workflow_wire(workspace, serde_json::to_string(&wire::Action::Get).expect("closed action")).await?;
        let context: WorkflowContext = serde_json::from_value(raw["context"].clone()).map_err(|e| invalid(e.to_string()))?;
        let state: State = if raw["state"].is_null() { State::empty(workspace.to_string(), context.capabilities.epoch) }
            else { serde_json::from_value(raw["state"].clone()).map_err(|e| invalid(e.to_string()))? };
        if state.workspace != workspace.to_string() { return Err(invalid("wrong workflow workspace")); }
        let presentation = project(&state, &context)?;
        Ok(WorkflowSnapshot { schema: 2, state, context, presentation, configuration: raw["configuration"].clone(), notices: raw["notices"].clone() })
    }
    /// Revalidate the exact displayed selection without advancing its observation.
    pub async fn workflow_prepare_action(&self, workspace: Uuid, item: &str, selected: Action,
        identity: WorkflowCommandIdentity) -> Result<PreparedWorkflowResult, SessionError>
    {
        prepare(&self.workflow_snapshot(workspace).await?, workspace, item, selected, identity)
    }
    /// Send retained canonical mutation bytes; reconnect retries must reuse this envelope.
    pub async fn workflow_submit_action(&self, prepared: &PreparedWorkflowAction) -> Result<serde_json::Value, SessionError> {
        if prepared.schema != 1 { return Err(invalid("invalid prepared workflow schema")); }
        match wire::parse(&prepared.json).map_err(invalid)? {
            wire::Action::Approve { .. } | wire::Action::Review { .. } | wire::Action::Cancel { .. } => {},
            _ => return Err(invalid("invalid prepared workflow action")),
        }
        self.workflow_wire(prepared.workspace, prepared.json.clone()).await
    }
    /// Shared JSON route for C/JNI/WASM. No caller-supplied snapshot or capability context.
    pub async fn workflow_call(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SessionError> {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Snapshot { workspace: Uuid }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Prepare {
            workspace: Uuid, item: String, action: Action, identity: WorkflowCommandIdentity,
        }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Submit { prepared: PreparedWorkflowAction }
        let value = match method {
            "workflow.snapshot" => { let a: Snapshot = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                serde_json::to_value(self.workflow_snapshot(a.workspace).await?) },
            "workflow.prepare_action" => { let a: Prepare = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                serde_json::to_value(self.workflow_prepare_action(a.workspace, &a.item, a.action, a.identity).await?) },
            "workflow.submit_action" => { let a: Submit = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                return self.workflow_submit_action(&a.prepared).await; },
            _ => return Err(invalid("unknown workflow Session method")),
        };
        value.map_err(|e| invalid(e.to_string()))
    }
}
#[cfg(test)]
mod tests;

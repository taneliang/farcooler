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
    pub host: String,
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
// An intent carries a workflow command, which grew with Start's model picks (ov-966). The result is made
// once per command and matched by value, so boxing it would only cost the callers.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PreparedWorkflowResult {
    Inspection { intent: Intent },
    Validation { error: cards::InputError },
    Mutation { prepared: PreparedWorkflowAction },
}
fn invalid(message: impl Into<String>) -> SessionError { SessionError::Protocol(message.into()) }
impl PreparedWorkflowAction {
    /// Validate the retained envelope without changing its identity or JSON bytes.
    pub fn validate(&self) -> Result<wire::Action, SessionError> {
        if self.schema != 1 { return Err(invalid("unsupported prepared workflow schema")); }
        let action = wire::parse(&self.json).map_err(invalid)?;
        match action {
            wire::Action::Approve { .. } | wire::Action::Review { .. } | wire::Action::Cancel { .. } => Ok(action),
            _ => Err(invalid("invalid prepared workflow action")),
        }
    }
}
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
fn selection(snapshot: &WorkflowSnapshot, workspace: Uuid, item: &str, selected: Action,
    revision: u64, epoch: u64, reason: Option<&str>) -> Result<(Intent, cards::RunCard), SessionError>
{
    if snapshot.state.workspace != workspace.to_string() || revision != snapshot.state.revision
        || epoch != snapshot.state.epoch { return Err(stale()); }
    let view = project(&snapshot.state, &snapshot.context)?;
    let card = view.runs.iter().find(|c| c.item == item).ok_or_else(stale)?;
    if !card.actions.contains(&selected) { return Err(stale()); }
    let pure = cards::project(&snapshot.state, &snapshot.context.capabilities).map_err(|_| stale())?;
    let original = pure.runs.iter().find(|c| c.item == item).and_then(|c|
        c.actions.iter().find(|a| a.kind == selected.kind && a.label == selected.label)).ok_or_else(stale)?;
    let intent = cards::action_with_reason(&snapshot.state, &snapshot.context.capabilities, item, original, reason).map_err(|_| stale())?;
    Ok((intent, card.clone()))
}
fn mutation(workspace: Uuid, command: Command, card: cards::RunCard,
    identity: WorkflowCommandIdentity) -> Result<PreparedWorkflowResult, SessionError>
{
    if identity.id.is_empty() || identity.id.len() > 100 { return Err(invalid("invalid workflow command identity")); }
    let mutation = Mutation { id: identity.id, sequence: identity.sequence, revision: identity.revision, epoch: identity.epoch };
    // The single finite Intent -> wire mapping for every transport binding.
    let action = match command {
        Command::Approve { item, run, pin, candidate } => wire::Action::Approve { mutation, item, run, pin, candidate },
        Command::Review { item, run, evidence, passed, reason } => wire::Action::Review {
            mutation, item, run, pin: card.definition_pin.clone(), evidence, passed, reason },
        Command::Cancel { item } => wire::Action::Cancel { mutation, item, run: card.run.clone(), pin: card.definition_pin.clone() },
        _ => return Err(invalid("unsupported workflow intent")),
    };
    let json = serde_json::to_string(&action).map_err(|e| invalid(e.to_string()))?;
    wire::parse(&json).map_err(invalid)?;
    Ok(PreparedWorkflowResult::Mutation { prepared: PreparedWorkflowAction { schema: 1, workspace, json } })
}
fn prepare(snapshot: &WorkflowSnapshot, workspace: Uuid, item: &str, selected: Action,
    identity: WorkflowCommandIdentity) -> Result<PreparedWorkflowResult, SessionError>
{
    let (intent, card) = selection(snapshot, workspace, item, selected, identity.revision, identity.epoch, None)?;
    match intent {
        Intent::Command(command) => mutation(workspace, command, card, identity),
        intent => Ok(PreparedWorkflowResult::Inspection { intent }),
    }
}
// Only the authenticated server context supplies the caller. This matches
// daemon workflow_runtime/wire_commands.rs's `wire:{principal}` source key;
// socket tests assert the allocated source against actual server consumption.
fn prepare_selection(snapshot: &WorkflowSnapshot, workspace: Uuid, item: &str, selected: Action,
    revision: u64, epoch: u64) -> Result<PreparedWorkflowResult, SessionError>
{
    prepare_selection_with_reason(snapshot, workspace, item, selected, revision, epoch, None)
}
fn prepare_selection_with_reason(snapshot: &WorkflowSnapshot, workspace: Uuid, item: &str, selected: Action,
    revision: u64, epoch: u64, reason: Option<&str>) -> Result<PreparedWorkflowResult, SessionError>
{
    // Validate observation first, even for malformed text: never bless an old form.
    let view = project(&snapshot.state, &snapshot.context)?;
    if snapshot.state.workspace != workspace.to_string() || revision != snapshot.state.revision || epoch != snapshot.state.epoch
        || !view.runs.iter().any(|r| r.item == item && r.actions.contains(&selected)) { return Err(stale()); }
    if let Err(error) = cards::validate_input(&selected.kind, reason) {
        return Ok(PreparedWorkflowResult::Validation { error });
    }
    let (intent, card) = selection(snapshot, workspace, item, selected, revision, epoch, reason)?;
    let Intent::Command(command) = intent else { return Ok(PreparedWorkflowResult::Inspection { intent }); };
    let principal = snapshot.context.capabilities.decision_principal.as_deref()
        .filter(|p| matches!(*p, "owner" | "orchestrator")).ok_or_else(|| invalid("missing workflow caller source"))?;
    let source = format!("wire:{principal}");
    let sequence = snapshot.state.source_sequences.get(&source).copied().unwrap_or(0)
        .checked_add(1).ok_or_else(|| invalid("workflow source sequence exhausted"))?;
    mutation(workspace, command, card, WorkflowCommandIdentity {
        id: Uuid::now_v7().to_string(), sequence, revision, epoch,
    })
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
        let host = self.host().await?;
        let host = Uuid::from_slice(&host.id).map_err(|_| invalid("invalid workflow host identity"))?.to_string();
        Ok(WorkflowSnapshot { schema: 2, host, state, context, presentation, configuration: raw["configuration"].clone(), notices: raw["notices"].clone() })
    }
    /// Revalidate the exact displayed selection without advancing its observation.
    pub async fn workflow_prepare_action(&self, workspace: Uuid, item: &str, selected: Action,
        identity: WorkflowCommandIdentity) -> Result<PreparedWorkflowResult, SessionError>
    {
        prepare(&self.workflow_snapshot(workspace).await?, workspace, item, selected, identity)
    }
    /// Prepare the exact displayed selection with shared sequence/identity allocation.
    pub async fn workflow_prepare_selection(&self, workspace: Uuid, item: &str, selected: Action,
        revision: u64, epoch: u64) -> Result<PreparedWorkflowResult, SessionError>
    {
        prepare_selection(&self.workflow_snapshot(workspace).await?, workspace, item, selected, revision, epoch)
    }
    /// Send retained canonical mutation bytes; reconnect retries must reuse this envelope.
    pub async fn workflow_submit_action(&self, prepared: &PreparedWorkflowAction) -> Result<serde_json::Value, SessionError> {
        prepared.validate()?;
        self.workflow_wire(prepared.workspace, prepared.json.clone()).await
    }
    /// Shared JSON route for C/JNI/WASM. No caller-supplied snapshot or capability context.
    pub async fn workflow_call(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SessionError> {
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Snapshot { workspace: Uuid }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Prepare {
            workspace: Uuid, item: String, action: Action, identity: WorkflowCommandIdentity,
        }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Selection {
            workspace: Uuid, item: String, action: Action, revision: String, epoch: String,
            #[serde(default)] reason: Option<String>,
        }
        #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Submit { prepared: PreparedWorkflowAction }
        let value = match method {
            "workflow.snapshot" => { let a: Snapshot = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                serde_json::to_value(self.workflow_snapshot(a.workspace).await?) },
            "workflow.prepare_action" => { let a: Prepare = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                serde_json::to_value(self.workflow_prepare_action(a.workspace, &a.item, a.action, a.identity).await?) },
            "workflow.prepare_selection" => { let a: Selection = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                let decimal = |s: &str| -> Result<u64, SessionError> {
                    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) { return Err(invalid("invalid workflow observation")); }
                    s.parse().map_err(|_| invalid("invalid workflow observation"))
                };
                serde_json::to_value(prepare_selection_with_reason(&self.workflow_snapshot(a.workspace).await?, a.workspace, &a.item, a.action,
                    decimal(&a.revision)?, decimal(&a.epoch)?, a.reason.as_deref())?) },
            "workflow.submit_action" => { let a: Submit = serde_json::from_value(args).map_err(|e| invalid(e.to_string()))?;
                return self.workflow_submit_action(&a.prepared).await; },
            _ => return Err(invalid("unknown workflow Session method")),
        };
        value.map_err(|e| invalid(e.to_string()))
    }
}
#[cfg(test)]
mod tests;

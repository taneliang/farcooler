//! The Engine view's reads and a person's actions over one session (`engine_page`, ov-1486).
use farcooler_protocol::capability;
use farcooler_protocol::v1::{ErrorCode, TaskNoteKind};
use farcooler_protocol::workflow_commands::Action;
use uuid::Uuid;

use super::*;
use crate::engine_page::{self as page, Draft, Outcome, Page, Selection};

fn protocol(message: &str) -> SessionError { SessionError::Protocol(message.into()) }

impl Session {
    /// The runner's bounded `workflow.get`: one reply, one control frame, whatever the board holds (ov-1434).
    async fn engine_state(&self, workspace: Uuid) -> Result<serde_json::Value, SessionError> {
        self.engine_command(workspace, &Action::Get).await
    }

    async fn engine_command(&self, workspace: Uuid, action: &Action) -> Result<serde_json::Value, SessionError> {
        self.workflow_wire(workspace, serde_json::to_string(action).map_err(|e| protocol(&e.to_string()))?).await
    }

    /// The Engine view: stations, landings, the person's actions, and the drafts waiting on them.
    pub async fn engine_page(&self, workspace: Uuid) -> Result<Page, SessionError> {
        let raw = self.engine_state(workspace).await?;
        // Titles and CI come from the plan, and the titles it lacks from the board. Without either, the view
        // still stands, with keys for titles.
        let plan = match self.plan(workspace).await {
            Ok(plan) => Some(plan),
            Err(error) if error.is_disconnect() => return Err(error),
            Err(_) => None,
        };
        let plan = self.engine_titles(workspace, &raw, plan).await?;
        let (drafts, problem) = match self.pending_drafts(workspace).await {
            Ok(drafts) => (drafts, None),
            Err(error) if error.is_disconnect() => return Err(error),
            Err(SessionError::Refused { code, .. }) if code == ErrorCode::ScopeDenied as i32 =>
                (Vec::new(), Some("This runner doesn’t let you see workflow drafts here.".to_string())),
            Err(_) => (Vec::new(), Some("Far Cooler couldn’t read the drafts waiting for approval.".to_string())),
        };
        page::page(workspace, &raw, plan.as_ref(), drafts, problem)
    }

    async fn engine_titles(&self, workspace: Uuid, raw: &serde_json::Value, plan: Option<farcooler_protocol::v1::Plan>)
        -> Result<Option<farcooler_protocol::v1::Plan>, SessionError>
    {
        let held = plan.clone().unwrap_or_default();
        if !page::lacks_titles(raw, &held) { return Ok(plan); }
        let request = farcooler_protocol::v1::TaskListRequest {
            workspace_id: Some(bytes::Bytes::copy_from_slice(workspace.as_bytes())), ..Default::default() };
        match self.task_pages(workspace, request, vec![capability::WORKSTREAMS.into()]).await {
            Ok(list) => Ok(Some(page::with_titles(held, &list.items))),
            Err(error) if error.is_disconnect() => Err(error),
            Err(_) => Ok(plan),
        }
    }

    async fn drafts_reply(&self, workspace: Uuid, draft: Option<&str>) -> Result<serde_json::Value, SessionError> {
        self.engine_command(workspace, &Action::Drafts { draft: draft.map(str::to_owned) }).await
    }

    async fn pending_drafts(&self, workspace: Uuid) -> Result<Vec<Draft>, SessionError> {
        let reply = self.drafts_reply(workspace, None).await?;
        let rows = reply["drafts"].as_array().ok_or_else(|| protocol("Invalid workflow drafts"))?;
        let mut drafts = Vec::new();
        for row in rows.iter().filter(|row| row["state"] == "pending") {
            let (id, card) = draft_row(row)?;
            drafts.push(page::draft(&id, &self.task_record(card).await?));
        }
        Ok(drafts)
    }

    /// Send the command a press means, if the row is still what the person saw. The runner checks who is
    /// asking, and the run itself, again.
    pub async fn engine_act(&self, workspace: Uuid, selection: Selection) -> Result<Outcome, SessionError> {
        if let Some(refused) = page::missing_reason(&selection) { return Ok(refused); }
        // The engine takes a command against one revision of the whole board. A busy board moves between the
        // read and the write, so a stale answer is read again, and sent only if this row hasn't changed.
        let mut tries = 0;
        loop {
            tries += 1;
            let raw = self.engine_state(workspace).await?;
            let Some(command) = page::command(workspace, &raw, &selection)? else {
                return Ok(Outcome::no(page::CHANGED, None));
            };
            let reply = self.engine_command(workspace, &command).await?;
            if tries < 3 && reply.pointer("/receipt/result/Err").and_then(|e| e.as_str()) == Some("Stale") { continue; }
            return Ok(page::receipt_outcome(selection.kind, &reply));
        }
    }

    /// Answer the question on a draft's card, as the person at this client (`task.note`). The runner decides
    /// whose answer it is and whether it may decide, and publishes only on an approval it accepts.
    pub async fn engine_decide(&self, workspace: Uuid, draft: &str, note: &str, approve: bool)
        -> Result<Outcome, SessionError>
    {
        let reply = self.drafts_reply(workspace, Some(draft)).await?;
        if reply["draft"]["state"] != "pending" { return Ok(Outcome::no(page::DRAFT_GONE, None)); }
        let (id, card) = draft_row(&reply["draft"])?;
        let Some(answer) = page::decision(&id, note, approve, &self.task_record(card).await?) else {
            return Ok(Outcome::no(page::CHANGED, None));
        };
        self.task_note(task_note_append(card, TaskNoteKind::Answer, &answer)).await?;
        let after = self.drafts_reply(workspace, Some(draft)).await?;
        Ok(page::decision_outcome(approve, after["draft"]["state"].as_str().unwrap_or_default(),
            &self.task_record(card).await?))
    }

    /// The `engine.*` routes of [`Session::workflow_call`].
    pub(super) async fn engine_call(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SessionError> {
        #[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Read { workspace: Uuid }
        #[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Act { workspace: Uuid, selection: Selection }
        #[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Decide { workspace: Uuid, draft: String, note: String, approve: bool }
        let bad = |e: serde_json::Error| protocol(&e.to_string());
        if !self.can(capability::WORKFLOW_COMMANDS) { return Err(protocol("This runner has no workflow engine")); }
        match method {
            "engine.read" => { let a: Read = serde_json::from_value(args).map_err(bad)?;
                serde_json::to_value(self.engine_page(a.workspace).await?) }
            "engine.act" => { let a: Act = serde_json::from_value(args).map_err(bad)?;
                serde_json::to_value(self.engine_act(a.workspace, a.selection).await?) }
            "engine.decide" => { let a: Decide = serde_json::from_value(args).map_err(bad)?;
                serde_json::to_value(self.engine_decide(a.workspace, &a.draft, &a.note, a.approve).await?) }
            _ => return Err(protocol("unknown workflow Session method")),
        }.map_err(bad)
    }
}

fn draft_row(row: &serde_json::Value) -> Result<(String, Uuid), SessionError> {
    let id = row["id"].as_str().ok_or_else(|| protocol("Invalid workflow draft"))?;
    let card = row["card"].as_str().and_then(|c| Uuid::parse_str(c).ok()).ok_or_else(|| protocol("Invalid workflow draft"))?;
    Ok((id.to_owned(), card))
}

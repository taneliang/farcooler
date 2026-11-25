//! Browser board intents use the existing scoped task, read-state and Engine calls.
use super::{Session, SessionError, board_reads::MarkRead, request, require, result, wrong};
use crate::web_board::{self, Draft, Intent};
use farcooler_protocol::{capability, v1::{self as pb, ErrorCode}, workflow_commands::{Action, Mutation}};
use farcooler_workflow::{State, digest};
use serde_json::{Value, json};
use uuid::Uuid;

impl Session {
    pub async fn web_board(&self, workspace: Uuid, repository: Uuid) -> Result<Value, SessionError> {
        require(self.capabilities(), capability::WORKSTREAMS, "task.list")?;
        let board = self.tasks(repository, Some(workspace)).await?;
        Ok(web_board::board(&board, self.granted_scope()))
    }

    pub async fn web_ticket(&self, workspace: Uuid, id: Uuid) -> Result<Value, SessionError> {
        let (detail, terminals, trees) = tokio::try_join!(
            self.task_record(id), self.terminals(), self.worktrees(),
        )?;
        let task = on_board(&detail, workspace)?;
        let start = if web_board::can_write(self.granted_scope()) && self.can(capability::WORKFLOW_COMMANDS) {
            self.ticket_start(workspace, &task.key).await.ok().flatten()
        } else {
            None
        };
        let value = crate::tasks_json::task_json(task, super::now_millis());
        let destinations = web_board::destinations(task, &terminals, &trees);
        let mut ticket = web_board::ticket(&value, self.granted_scope(), self.can(capability::BOARD_READS),
            start.as_ref().map(|s| s.guard.as_str()), destinations);
        ticket["review"] = crate::web_review::links(self.granted_scope(), destinations.1.is_some(),
            self.can(capability::WORKTREE_FILES), self.can(capability::CHANGES));
        Ok(ticket)
    }

    pub async fn web_board_intent(&self, workspace: Uuid, intent: Intent) -> Result<Value, SessionError> {
        if !web_board::can_write(self.granted_scope()) {
            return Err(refused(ErrorCode::ScopeDenied, ""));
        }
        match intent {
            Intent::Create { draft } => {
                require(self.capabilities(), capability::TASKS, "task.create")?;
                let title = checked_title(&draft.title)?;
                let payload = request::Payload::TaskCreate(pb::TaskCreate {
                    workspace_id: Some(id_bytes(workspace)), title, intent: draft.intent,
                    acceptance: acceptance(&draft.acceptance, &[]),
                    constraints: lines(&draft.constraints), labels: lines(&draft.labels),
                    actor: "user".into(), ..Default::default()
                });
                let reply = self.value_requiring("task.create", Some(workspace), Some(payload),
                    vec![capability::WORKSTREAMS.into()]).await?;
                ticket_id(reply)
            }
            Intent::Edit { task, version, draft } => {
                let detail = self.task_record(task).await?;
                let current = on_board(&detail, workspace)?;
                if version.parse::<u64>().ok() != Some(current.resource_version) {
                    return Err(refused(ErrorCode::ResourceConflict, "ticket_version"));
                }
                let title = checked_title(&draft.title)?;
                let payload = request::Payload::TaskUpdate(update(current, draft, title));
                let response = self.value("task.update", Some(task), Some(payload)).await.map_err(|e| {
                    if matches!(&e, SessionError::Refused { code, what, .. } if *code == ErrorCode::ResourceConflict as i32 && what.is_empty()) {
                        refused(ErrorCode::ResourceConflict, "ticket_version")
                    } else { e }
                })?;
                ticket_id(response)
            }
            Intent::Mark { task, through } => {
                let raise = match task {
                    Some(task) => MarkRead { opened: vec![(task, through)], floor_ms: None },
                    None => MarkRead { opened: vec![], floor_ms: Some(through) },
                };
                self.mark_read(workspace, raise).await?;
                Ok(Value::Null)
            }
            Intent::Start { task, guard } => {
                let detail = self.task_record(task).await?;
                let key = &on_board(&detail, workspace)?.key;
                let start = self.ticket_start(workspace, key).await?.filter(|s| s.guard == guard)
                    .ok_or_else(|| refused(ErrorCode::ResourceConflict, "ticket_start"))?;
                let reply = self.workflow_wire(workspace, serde_json::to_string(&start.action)
                    .map_err(|e| SessionError::Protocol(e.to_string()))?).await?;
                if reply.pointer("/receipt/result/Ok").is_none() {
                    return Err(refused(ErrorCode::ResourceConflict, "ticket_start"));
                }
                Ok(Value::Null)
            }
        }
    }

    async fn ticket_start(&self, workspace: Uuid, key: &str) -> Result<Option<Start>, SessionError> {
        let raw = self.workflow_wire(workspace, serde_json::to_string(&Action::Get).expect("closed action")).await?;
        Ok(start(&raw, workspace, key))
    }
}

fn id_bytes(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

fn refused(code: ErrorCode, what: &str) -> SessionError {
    SessionError::Refused { code: code as i32, retryable: false, message: what.into(), what: what.into() }
}

fn ticket_id(reply: result::Value) -> Result<Value, SessionError> {
    match reply {
        result::Value::Task(task) => Uuid::from_slice(&task.id).map(|id| json!(id))
            .map_err(|e| SessionError::Protocol(e.to_string())),
        other => Err(wrong("task", &other)),
    }
}

pub(super) fn on_board(detail: &pb::TaskDetail, workspace: Uuid) -> Result<&pb::Task, SessionError> {
    detail.task.as_ref().filter(|task| task.workspace_id.as_ref() == workspace.as_bytes())
        .ok_or_else(|| refused(ErrorCode::NotFound, ""))
}

fn checked_title(title: &str) -> Result<String, SessionError> {
    let title = title.trim();
    if title.is_empty() { return Err(refused(ErrorCode::InvalidArgument, "ticket_title")); }
    Ok(title.into())
}

fn lines(text: &str) -> Vec<String> {
    text.lines().filter(|line| !line.trim().is_empty()).map(str::to_owned).collect()
}

// A textarea normalizes newlines. Keep complete existing entries, including
// multiline criteria, when a person inserts or removes a different entry.
fn textarea(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn revised_lines(text: &str, current: &[&str]) -> Vec<String> {
    let normalized: Vec<_> = current.iter().map(|entry| textarea(entry)).collect();
    if normalized.join("\n") == text {
        return current.iter().map(|entry| (*entry).to_owned()).collect();
    }
    let mut remaining = text;
    let mut revised = Vec::new();
    while !remaining.is_empty() {
        let existing = normalized.iter().enumerate().filter(|(_, entry)| !entry.is_empty())
            .filter(|(_, entry)| remaining.strip_prefix(entry.as_str())
                .is_some_and(|tail| tail.is_empty() || tail.starts_with('\n')))
            .max_by_key(|(_, entry)| entry.len());
        if let Some((i, entry)) = existing {
            revised.push(current[i].to_owned());
            remaining = &remaining[entry.len()..];
            remaining = remaining.strip_prefix('\n').unwrap_or(remaining);
        } else {
            let (line, tail) = remaining.split_once('\n').unwrap_or((remaining, ""));
            if !line.trim().is_empty() { revised.push(line.to_owned()); }
            remaining = tail;
        }
    }
    revised
}

fn acceptance(text: &str, current: &[pb::TaskAcceptanceItem]) -> Vec<pb::TaskAcceptanceItem> {
    let original: Vec<_> = current.iter().map(|a| a.text.as_str()).collect();
    let mut remaining: Vec<_> = current.iter().collect();
    revised_lines(text, &original).into_iter().map(|text| {
        if let Some(i) = remaining.iter().position(|a| a.text == text) {
            remaining.remove(i).clone()
        } else {
            pb::TaskAcceptanceItem { text, ..Default::default() }
        }
    }).collect()
}

fn update(current: &pb::Task, draft: Draft, title: String) -> pb::TaskUpdate {
    pb::TaskUpdate {
        task_id: current.id.clone(), expected_version: current.resource_version,
        title,
        intent: if textarea(&current.intent) == draft.intent { current.intent.clone() } else { draft.intent },
        acceptance: acceptance(&draft.acceptance, &current.acceptance),
        constraints: revised_lines(&draft.constraints, &current.constraints.iter().map(String::as_str).collect::<Vec<_>>()),
        labels: revised_lines(&draft.labels, &current.labels.iter().map(String::as_str).collect::<Vec<_>>()),
        worktree_id: current.worktree_id.clone(), actor: "user".into(),
    }
}

struct Start {
    guard: String,
    action: Action,
}

fn start(raw: &Value, workspace: Uuid, key: &str) -> Option<Start> {
    let context = &raw["context"];
    if context["capabilities"]["controller"] != true { return None; }
    let binding = context["binding"].as_str().filter(|s| !s.is_empty())?;
    let principal = context.pointer("/capabilities/decision_principal")?.as_str()?;
    let state: State = serde_json::from_value(raw["state"].clone()).ok()?;
    if state.workspace != workspace.to_string() { return None; }
    let start_pin = raw.pointer("/configuration/pin")?.as_str()?;
    let (run, pin) = match state.items.get(key) {
        Some(item) => item.can_start().then(|| (item.run.clone(), item.pin.clone()))?,
        None => state.archive.get(key).filter(|item| item.can_start())?.start_identity(key),
    };
    let guard = digest(&("web-ticket-start/v1", workspace, key, &run, &pin, start_pin, binding, state.epoch));
    let source = format!("wire:{principal}");
    let sequence = state.source_sequences.get(&source).copied().unwrap_or(0).checked_add(1)?;
    let mutation = Mutation { id: Uuid::now_v7().to_string(), sequence, revision: state.revision, epoch: state.epoch };
    let action = Action::Start {
        mutation, item: key.into(), run, pin, start_pin: start_pin.into(),
        models: Default::default(), worktree: None, with: vec![],
    };
    Some(Start { guard, action })
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_workflow::{Archived, Phase};

    #[test]
    fn edit_preserves_untouched_multiline_fields_and_criterion_ids() {
        let item = pb::TaskAcceptanceItem {
            id: id_bytes(Uuid::now_v7()), text: "First line\n  more context".into(), met: true,
        };
        let task = pb::Task {
            id: id_bytes(Uuid::now_v7()), resource_version: u64::MAX,
            acceptance: vec![item.clone()], constraints: vec!["one\ntwo".into()],
            labels: vec!["label".into()], worktree_id: Some(id_bytes(Uuid::now_v7())),
            ..Default::default()
        };
        let raw = crate::tasks_json::task_json(&task, 0);
        let fields = web_board::form(Some(&raw))["fields"].as_array().unwrap().iter()
            .map(|f| (f["name"].as_str().unwrap().to_string(), f["value"].clone()))
            .collect::<serde_json::Map<_, _>>();
        let draft = serde_json::from_value(Value::Object(fields)).unwrap();
        let revised = update(&task, draft, "New title".into());
        assert_eq!(revised.title, "New title");
        assert_eq!(revised.expected_version, u64::MAX);
        assert_eq!(revised.constraints, task.constraints);
        assert_eq!(revised.labels, task.labels);
        assert_eq!(revised.acceptance, std::slice::from_ref(&item));
        let extra = acceptance("First line\n  more context\nNew criterion", std::slice::from_ref(&item));
        assert_eq!(extra[0], item);
        assert_eq!(extra[1].text, "New criterion");
        assert!(!extra[1].met);
        assert_eq!(revised.worktree_id, task.worktree_id);
    }

    #[test]
    fn start_requires_current_ticket_identity_configuration_and_controller() {
        let ws = Uuid::now_v7();
        let mut state = State::empty(ws.to_string(), 7);
        state.source_sequences.insert("wire:owner".into(), 40);
        state.archive.insert("ov-1".into(), Archived {
            phase: Phase::Cancelled, start_load: None, closed: None, candidate: None, group: None,
            last_run: None, history: vec![],
        });
        let mut raw = json!({
            "state": state, "configuration":{"pin":"published"},
            "context":{"binding":"admission", "capabilities":{"controller":true,"decision_principal":"owner"}}
        });
        let prepared = start(&raw, ws, "ov-1").unwrap();
        let Action::Start { mutation, item, run, pin, start_pin, .. } = &prepared.action else { panic!("expected Start"); };
        assert_eq!((mutation.sequence, mutation.revision, mutation.epoch), (41, 0, 7));
        assert!(state.matches_start(item, run, pin));
        assert_eq!(start_pin, "published");
        raw["state"]["revision"] = json!(18);
        assert_eq!(start(&raw, ws, "ov-1").unwrap().guard, prepared.guard);
        raw["configuration"]["pin"] = "repinned".into();
        assert_ne!(start(&raw, ws, "ov-1").unwrap().guard, prepared.guard);
        raw["context"]["capabilities"]["controller"] = false.into();
        assert!(start(&raw, ws, "ov-1").is_none());
    }
}

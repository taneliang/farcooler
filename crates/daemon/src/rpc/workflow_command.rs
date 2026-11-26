//! Dispatch workflow commands and choose the reply shape requested by the client.
use farcooler_core::{DomainError, Result};
use farcooler_protocol::{
    accepts,
    v1::{Request, WorkflowReply, request, result},
    workflow_commands::{self, Action, BUNDLE_VIEW, MAX_BYTES},
};

use super::Rpc;
use uuid::Uuid;
use crate::workflow_runtime::{Error, GetView, encode_plan_get};

fn domain_error(error: Error) -> DomainError {
    match error {
        Error::Domain(refusal) => refusal,
        _ => DomainError::NotFound,
    }
}

impl Rpc {
    pub(super) async fn workflow_command(&self, req: Request) -> Result<result::Value> {
        let workspace = Self::target(&req)?;
        let svc = &self.service;
        svc.store.get_workspace(workspace)?;
        self.engine_caller(workspace, &req.method)?;
        let Some(request::Payload::WorkflowCommand(payload)) = req.payload else {
            return Err(DomainError::NotFound);
        };
        if payload.schema != 1 || payload.json.len() > MAX_BYTES { return Err(DomainError::NotFound); }
        let action = workflow_commands::parse(&payload.json).map_err(|_| DomainError::NotFound)?;
        if action.method() != req.method { return Err(DomainError::NotFound); }
        let handle = self.workflow.as_ref().ok_or(DomainError::CapabilityUnsupported { needed: "workflow execution service" })?;
        let admission = self.admit(workspace).await?;
        // A Maintainer can publish an approved draft through workflow_drafts.
        // Direct publication belongs to the owner or the workspace's authorized orchestrator.
        if matches!(action, Action::Publish { .. } | Action::PreviewPublication { .. })
            && !admission.is_owner() && admission.orchestrator().is_none()
        {
            return Err(DomainError::ScopeDenied { needed: "owner" });
        }
        if let Action::PreviewPublication { publication } = action {
            return Self::workflow_publication_preview(handle, admission, *publication).await;
        }
        admission.permits(&action)?;
        let wants = |view: &str| req.accepts.iter().any(|entry| entry == view);
        // Reads bypass the effect inbox. Shipped apps use the unmarked, single-response schema.
        let json = match action {
            Action::Propose { .. } | Action::Drafts { .. } =>
                return crate::workflow_drafts::handle(svc, &self.watcher, workspace, &action),
            Action::Logs { item } =>
                handle.read_logs(admission, item).await.map_err(domain_error)?.to_string(),
            Action::Get if wants(BUNDLE_VIEW) => handle.read_bundle(admission).await.map_err(|error| match error {
                Error::Domain(refusal) => refusal,
                _ => DomainError::OperationFailed,
            })?,
            Action::History { .. } | Action::ArchiveItem { .. } =>
                handle.read_history(admission, action).await.map_err(domain_error)?.to_string(),
            Action::Get | Action::GetPage { .. } => {
                let page = match &action {
                    Action::GetPage { offset, digest } => Some((*offset, digest.as_str())),
                    _ if wants(accepts::WORKFLOW_PAGES) => Some((0, "")),
                    _ => None,
                };
                let full = wants(accepts::WORKFLOW_FULL);
                let summary = wants("workflow_summary_v1") && !full;
                let plan = !full && page.is_none() && wants(accepts::WORKFLOW_PLAN);
                let view = if plan { GetView::Plan } else if summary { GetView::Summary } else { GetView::Full };
                let mut read = handle.read_get(admission, view).await.map_err(domain_error)?;
                if plan {
                    let cursor = req.accepts.iter().find_map(|value| value.strip_prefix(accepts::WORKFLOW_PLAN_PAGE));
                    return Ok(result::Value::WorkflowReply(WorkflowReply { schema: 1, json: encode_plan_get(read.value, cursor)? }));
                }
                let published_only = wants("workflow_repin_v1");
                if published_only { read.keep_publication(); }
                if !summary {
                    let include_suggestion = !published_only && (page.is_none() || full);
                    read.value = crate::workflow_starter::add_to_get(svc, workspace, read.value, include_suggestion);
                }
                match page {
                    Some((offset, digest)) => read.into_paged_json(offset, digest),
                    None => read.into_json(summary || full),
                }.map_err(|_| DomainError::OperationFailed)?
            }
            _ => {
                let value = handle.wire_command(admission, action).await.map_err(domain_error)?;
                self.announce_workflow_publication(&value);
                value.to_string()
            }
        };
        Ok(result::Value::WorkflowReply(WorkflowReply { schema: 1, json }))
    }

    /// Every successful publication path announces the receipt's owner-facing review card.
    pub(crate) fn announce_workflow_publication(&self, value: &serde_json::Value) {
        if let Some(id) = value
            .pointer("/receipt/change/notice_task")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            && let Ok(task) = self.service.store.get_task(id)
        {
            self.watcher.announce_task_changed(
                &task,
                None,
                farcooler_store::models::Actor::Manager,
            );
            self.watcher.task_event(
                &task,
                crate::watch::task_notice::TaskEvent::Created,
                farcooler_store::models::Actor::Manager,
            );
            self.watcher.announce_needs_you();
        }
    }
}

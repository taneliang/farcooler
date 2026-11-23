//! Workflow command admission, execution and reply shaping.
use farcooler_protocol::workflow_commands::{Action, BUNDLE_VIEW, MAX_BYTES};

use super::*;
use crate::workflow_runtime::{GetView, encode_legacy_get, encode_plan_get};

impl Rpc {
    pub(super) async fn workflow_command(&self, req: Request) -> Result<result::Value> {
        let svc = &self.service;
        let workspace = Self::target(&req)?;
        svc.store.get_workspace(workspace)?;
        self.engine_caller(workspace, &req.method)?;
        let Some(request::Payload::WorkflowCommand(p)) = req.payload else {
            return Err(DomainError::NotFound);
        };
        if p.schema != 1 || p.json.len() > MAX_BYTES {
            return Err(DomainError::NotFound);
        }
        let action: Action = farcooler_protocol::workflow_commands::parse(&p.json)
            .map_err(|_| DomainError::NotFound)?;
        if action.method() != req.method {
            return Err(DomainError::NotFound);
        }
        let handle = self
            .workflow
            .as_ref()
            .ok_or(DomainError::CapabilityUnsupported {
                needed: "workflow execution service",
            })?;
        let admission = self.admit(workspace).await?;
        // A Maintainer may publish a draft they approved through workflow_drafts. The direct
        // command is still the owner's or, with a grant, this workspace's orchestrator's.
        if matches!(action, Action::Publish { .. } | Action::PreviewPublication { .. })
            && !admission.is_owner()
            && admission.orchestrator().is_none()
        {
            return Err(DomainError::ScopeDenied { needed: "owner" });
        }
        if let Action::PreviewPublication { publication } = action {
            return Self::workflow_publication_preview(handle, admission, *publication).await;
        }
        admission.permits(&action)?;
        if matches!(action, Action::Propose { .. } | Action::Drafts { .. }) {
            return crate::workflow_drafts::handle(svc, &self.watcher, workspace, &action);
        } // filed and asked of a person here, never the engine (ov-971)
        // Every read bypasses the effect inbox and its per-message git/tree cleanup.
        let plan_view = req
            .accepts
            .iter()
            .any(|v| v == farcooler_protocol::accepts::WORKFLOW_PLAN);
        let summary = req.accepts.iter().any(|v| v == "workflow_summary_v1");
        let full = req.accepts.iter().any(|v| v == "workflow_full_v1");
        if let Action::Logs { item } = action {
            let value = handle
                .read_logs(admission, item)
                .await
                .map_err(|e| match e {
                    crate::workflow_runtime::Error::Domain(said) => said,
                    _ => DomainError::NotFound,
                })?;
            return Ok(result::Value::WorkflowReply(
                farcooler_protocol::v1::WorkflowReply {
                    schema: 1,
                    json: value.to_string(),
                },
            ));
        }
        let action_is_get = matches!(action, Action::Get);
        if action_is_get && req.accepts.iter().any(|v| v == BUNDLE_VIEW) {
            let json = handle.read_bundle(admission).await.map_err(|error| match error {
                crate::workflow_runtime::Error::Domain(refusal) => refusal,
                _ => DomainError::OperationFailed,
            })?;
            return Ok(result::Value::WorkflowReply(
                farcooler_protocol::v1::WorkflowReply { schema: 1, json },
            ));
        }
        let value = if action_is_get {
            let view = if plan_view {
                GetView::Plan
            } else if summary && !full {
                GetView::Summary
            } else {
                GetView::Full
            };
            handle.read_get(admission, view).await
        } else {
            handle.wire_command(admission, action).await
        }
        .map_err(|e| match e {
            crate::workflow_runtime::Error::Domain(said) => said,
            _ => DomainError::NotFound,
        })?;
        if action_is_get && plan_view {
            return Ok(result::Value::WorkflowReply(
                farcooler_protocol::v1::WorkflowReply {
                    schema: 1,
                    json: encode_plan_get(value),
                },
            ));
        }
        let mut value = value;
        // `workspace re-pin` reads the published workflow and nothing else (ov-1373): the state, notices and
        // lists beside it grow without bound, and the reply may not pass the 1 MiB frame cap.
        if action_is_get && req.accepts.iter().any(|v| v == "workflow_repin_v1") {
            if let Some(o) = value.as_object_mut() {
                o.retain(|k, _| matches!(k.as_str(), "schema" | "configuration" | "context"));
            }
        }
        self.announce_workflow_publication(&value);
        if summary && !full {
            return Ok(result::Value::WorkflowReply(
                farcooler_protocol::v1::WorkflowReply {
                    schema: 1,
                    json: value.to_string(),
                },
            ));
        }
        let value = crate::workflow_starter::add_to_get(svc, workspace, value).await;
        let json = if action_is_get && req.accepts.is_empty() {
            encode_legacy_get(value)
        } else {
            value.to_string()
        };
        Ok(result::Value::WorkflowReply(
            farcooler_protocol::v1::WorkflowReply { schema: 1, json },
        ))
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

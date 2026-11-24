use super::*;

impl Session {
    pub fn board_order_label(&self) -> Option<&'static str> {
        self.can(farcooler_protocol::capability::BOARD_ORDER).then_some("Backlog and To Do")
    }

    pub async fn board_order_view(&self, workspace: Uuid) -> Result<Option<serde_json::Value>, SessionError> {
        use farcooler_protocol::{capability, v1::{TaskListRequest, TaskStatus}};
        if self.board_order_label().is_none() { return Ok(None); }
        let mut tasks = Vec::new();
        for status in [TaskStatus::Backlog, TaskStatus::Todo] {
            let request = TaskListRequest { workspace_id: Some(bytes::Bytes::copy_from_slice(workspace.as_bytes())),
                status: status as i32, ..Default::default() };
            let page = self.task_pages(workspace, request, vec![capability::WORKSTREAMS.into()]).await?;
            tasks.extend(page.items);
        }
        Ok(Some(crate::task_board_view::order_view(&tasks, self.granted_scope() != "read")))
    }

    pub async fn reorder_task(&self, task: Uuid, anchor: Uuid, after: bool) -> Result<(), SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_ORDER, "task.reorder")?;
        let request = task_reorder_request(task, anchor, after, "user".into());
        match self.value(&request.method, Some(task), request.payload).await? {
            result::Value::Empty(_) => Ok(()),
            other => Err(wrong("empty", &other)),
        }
    }
}

pub fn task_reorder_request(task: Uuid, anchor: Uuid, after: bool, actor: String) -> farcooler_protocol::v1::Request {
    use farcooler_protocol::v1::{Request, TaskReorder, task_reorder::Placement};
    let anchor = bytes::Bytes::copy_from_slice(anchor.as_bytes());
    Request {
        target_resource_id: Some(bytes::Bytes::copy_from_slice(task.as_bytes())),
        payload: Some(request::Payload::TaskReorder(TaskReorder {
            task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
            placement: Some(if after { Placement::After(anchor) } else { Placement::Before(anchor) }),
            actor,
        })),
        ..farcooler_transport::request("task.reorder")
    }
}

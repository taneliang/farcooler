use super::*;

impl Session {
    pub(super) async fn task_pages(
        &self, target: Uuid, mut list: farcooler_protocol::v1::TaskListRequest, required: Vec<String>,
    ) -> Result<farcooler_protocol::v1::TaskList, SessionError> {
        // A page at a time (ov-1298): a long-lived board no longer fits one frame. A runner too old to page
        // answers with every task and no next page. A page whose cursor went (its task deleted or moved
        // between reads) is refused, and the read starts again, once.
        let mut whole = farcooler_protocol::v1::TaskList::default();
        let mut restarted = false;
        loop {
            list.page_after = Some(whole.next_page_after.take().unwrap_or_default());
            let payload = request::Payload::TaskList(list.clone());
            let page = match self.value_requiring("task.list", Some(target), Some(payload), required.clone()).await {
                Ok(result::Value::TaskList(page)) => page,
                Ok(other) => return Err(wrong("task_list", &other)),
                Err(_) if !restarted && list.page_after.as_ref().is_some_and(|a| !a.is_empty()) => {
                    restarted = true;
                    whole = Default::default();
                    continue;
                }
                Err(e) => return Err(e),
            };
            if whole.reads.is_none() { whole.reads = page.reads; }
            whole.items.extend(page.items);
            match page.next_page_after.filter(|next| !next.is_empty() && Some(next) != list.page_after.as_ref()) {
                Some(next) => whole.next_page_after = Some(next),
                None => break,
            }
        }
        Ok(whole)
    }

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

//! The browser board, over the existing scoped task and read-state calls.
use super::{Session, SessionError, board_reads::MarkRead, require};
use crate::web_board::{self, Intent};
use farcooler_protocol::{capability, v1::{self as pb, ErrorCode}};
use serde_json::Value;
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
        let value = crate::tasks_json::task_json(task, super::now_millis());
        let destinations = web_board::destinations(task, &terminals, &trees);
        let mut ticket = web_board::ticket(&value, self.granted_scope(), self.can(capability::BOARD_READS),
            destinations);
        ticket["review"] = crate::web_review::links(self.granted_scope(), destinations.1.is_some(),
            self.can(capability::WORKTREE_FILES), self.can(capability::CHANGES));
        Ok(ticket)
    }

    pub async fn web_board_intent(&self, workspace: Uuid, intent: Intent) -> Result<(), SessionError> {
        if !web_board::can_write(self.granted_scope()) {
            return Err(refused(ErrorCode::ScopeDenied));
        }
        let Intent::Mark { task, through } = intent;
        let raise = match task {
            Some(task) => MarkRead { opened: vec![(task, through)], floor_ms: None },
            None => MarkRead { opened: vec![], floor_ms: Some(through) },
        };
        self.mark_read(workspace, raise).await?;
        Ok(())
    }
}

fn refused(code: ErrorCode) -> SessionError {
    SessionError::Refused { code: code as i32, retryable: false, message: String::new(), what: String::new() }
}

pub(super) fn on_board(detail: &pb::TaskDetail, workspace: Uuid) -> Result<&pb::Task, SessionError> {
    detail.task.as_ref().filter(|task| task.workspace_id.as_ref() == workspace.as_bytes())
        .ok_or_else(|| refused(ErrorCode::NotFound))
}

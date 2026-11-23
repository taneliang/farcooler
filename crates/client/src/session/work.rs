//! Work commands use the same scoped Session as every other runner call.
use super::{Session, SessionError, request, result, wrong};
use crate::work_view::{self, Intent, Inventory};
use farcooler_protocol::v1::{ErrorCode, Terminal, TerminalRename, TerminalState};
use uuid::Uuid;

impl Session {
    pub async fn work_inventory(&self) -> Result<Inventory, SessionError> {
        let workspace_read = async {
            if self.can(farcooler_protocol::capability::WORKSTREAMS) {
                self.workspaces().await
            } else {
                Ok(Vec::new())
            }
        };
        let (workspaces, repositories, worktrees, terminals) = tokio::try_join!(
            workspace_read, self.repositories(), self.worktrees(), self.terminals()
        )?;
        Ok(work_view::inventory(self.granted_scope(), workspaces, repositories, worktrees, terminals))
    }

    pub async fn work_intent(&self, intent: Intent) -> Result<Option<Uuid>, SessionError> {
        let id = match intent {
            Intent::Create { repository, workspace, name, branch } => {
                let tree = self.create_worktree_in(repository, name.trim(), branch.trim(), "HEAD", "", false, workspace).await?;
                Some(work_id(&tree.id)?)
            }
            Intent::Start { worktree, preset } => {
                let terminal = self.create_terminal(worktree, preset.label(), preset.command(), false).await?;
                Some(work_id(&terminal.id)?)
            }
            Intent::Rename { terminal, name } => {
                let payload = request::Payload::TerminalRename(TerminalRename { name: name.trim().into() });
                self.value("terminal.rename", Some(terminal), Some(payload)).await?;
                None
            }
            Intent::Close { terminal } => {
                if let Some(listed) = self.work_terminal(terminal).await? {
                    match TerminalState::try_from(listed.state) {
                        Ok(TerminalState::Running | TerminalState::Starting) => self.stop_terminal(terminal).await?,
                        Ok(TerminalState::Exited | TerminalState::Error | TerminalState::Lost) => {}
                        _ => return Err(work_state_error(ErrorCode::OperationFailed, "terminal_state_unknown")),
                    }
                    self.remove_terminal(terminal).await?;
                }
                None
            }
            Intent::Restart { terminal } => {
                let listed = self.work_terminal(terminal).await?
                    .ok_or_else(|| work_state_error(ErrorCode::NotFound, ""))?;
                match TerminalState::try_from(listed.state) {
                    Ok(TerminalState::Running | TerminalState::Starting) => Some(terminal),
                    Ok(TerminalState::Lost) => {
                        match self.value("terminal.restart", Some(terminal), None).await? {
                            // A runner can replace the pane while recovering it.
                            result::Value::Terminal(t) => Some(work_id(&t.id)?),
                            other => return Err(wrong("terminal", &other)),
                        }
                    }
                    _ => return Err(work_state_error(ErrorCode::OperationFailed, "terminal_state_changed")),
                }
            }
        };
        Ok(id)
    }

    async fn work_terminal(&self, terminal: Uuid) -> Result<Option<Terminal>, SessionError> {
        // A close may be retried after the stop succeeded. Look again so the
        // retry can remove the stopped record without stopping a second time.
        Ok(self.terminals().await?.into_iter().find(|t| t.id.as_ref() == terminal.as_bytes()))
    }
}

fn work_id(bytes: &[u8]) -> Result<Uuid, SessionError> {
    Uuid::from_slice(bytes).map_err(|e| SessionError::Protocol(e.to_string()))
}

fn work_state_error(code: ErrorCode, what: &str) -> SessionError {
    SessionError::Refused { code: code as i32, retryable: false, message: what.into(), what: what.into() }
}

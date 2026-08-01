//! One ACP session: the conversation, the capability answers, and the events.

use std::path::Path;

use crate::acp::conn::{AcpConnection, AcpError, Incoming};
use crate::acp::normalize::update_to_events;
use crate::acp::wire::Rpc;
use crate::event::{AgentEvent, AgentGapReason, Diff, PermissionOption, ToolStatus};
use crate::fs_guard::confine;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Acp(#[from] AcpError),
    #[error("refused: the path is outside the workspace worktree")]
    Refused,
    #[error("the agent did not accept the session")]
    Rejected,
}

/// Perform a confined write and describe it as a diff.
pub fn handle_fs_write(
    worktree: &Path,
    requested: &str,
    contents: &str,
) -> Result<AgentEvent, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    let old_text = std::fs::read_to_string(&path).ok();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, contents).map_err(|_| SessionError::Refused)?;
    Ok(AgentEvent::ToolUpdate {
        id: path.display().to_string(),
        status: ToolStatus::Completed,
        content: None,
        diff: Some(Diff {
            path: path.display().to_string(),
            old_text,
            new_text: contents.to_string(),
        }),
    })
}

/// Read a confined file for the agent.
pub fn handle_fs_read(worktree: &Path, requested: &str) -> Result<String, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    std::fs::read_to_string(&path).map_err(|_| SessionError::Refused)
}

/// A `session/request_permission` as the event that blocks a fleet row.
pub fn permission_event(request_id: &str, params: &serde_json::Value) -> AgentEvent {
    let options = params["options"]
        .as_array()
        .map(|opts| {
            opts.iter()
                .map(|o| PermissionOption {
                    id: o["optionId"].as_str().unwrap_or_default().to_string(),
                    name: o["name"].as_str().unwrap_or_default().to_string(),
                    kind: o["kind"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    AgentEvent::Permission {
        id: request_id.to_string(),
        tool_call: params["toolCall"]["toolCallId"].as_str().unwrap_or_default().to_string(),
        options,
    }
}

/// What a reconnect emits when the agent cannot replay its own history.
pub fn load_unsupported_event() -> AgentEvent {
    AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
}

pub struct AgentSession {
    conn: AcpConnection,
    pub session_id: String,
    pub available_modes: Vec<String>,
    pub available_commands: Vec<String>,
}

impl AgentSession {
    /// Initialize, then either load an existing session or create a new one.
    ///
    /// `resume` carries the session id from SQLite. Its absence means this is a
    /// terminal that has never been in agent pane mode.
    pub async fn start(
        mut conn: AcpConnection,
        resume: Option<String>,
    ) -> Result<(Self, Vec<AgentEvent>), SessionError> {
        let init = conn
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true } }
                }),
            )
            .await?;
        let can_load = init["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);

        let mut prelude = Vec::new();
        let cwd = conn.worktree.display().to_string();

        let session_id = match resume {
            Some(id) if can_load => {
                conn.request(
                    "session/load",
                    serde_json::json!({ "sessionId": id, "cwd": cwd, "mcpServers": [] }),
                )
                .await?;
                id
            }
            Some(id) => {
                // Honest rather than convenient: the conversation continues, but
                // the history before this point cannot be shown.
                prelude.push(load_unsupported_event());
                conn.request(
                    "session/new",
                    serde_json::json!({ "cwd": cwd, "mcpServers": [] }),
                )
                .await?["sessionId"]
                    .as_str()
                    .unwrap_or(&id)
                    .to_string()
            }
            None => conn
                .request("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": [] }))
                .await?["sessionId"]
                .as_str()
                .ok_or(SessionError::Rejected)?
                .to_string(),
        };

        let available_modes: Vec<String> = init["agentCapabilities"]["availableModes"]
            .as_array()
            .map(|m| m.iter().filter_map(|v| v["id"].as_str().map(String::from)).collect())
            .unwrap_or_default();

        prelude.insert(
            0,
            AgentEvent::SessionStarted {
                session_id: session_id.clone(),
                agent_mode: init["agentCapabilities"]["currentModeId"].as_str().map(String::from),
                available_modes: available_modes.clone(),
                available_commands: Vec::new(),
            },
        );

        Ok((
            Self { conn, session_id, available_modes, available_commands: Vec::new() },
            prelude,
        ))
    }

    pub async fn prompt(&mut self, text: &str) -> Result<(), SessionError> {
        self.conn
            .notify(
                "session/prompt",
                serde_json::json!({
                    "sessionId": self.session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn answer(
        &mut self,
        request_id: serde_json::Value,
        option_id: &str,
    ) -> Result<(), SessionError> {
        self.conn
            .respond(
                request_id,
                serde_json::json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_mode(&mut self, agent_mode: &str) -> Result<(), SessionError> {
        self.conn
            .notify(
                "session/set_mode",
                serde_json::json!({ "sessionId": self.session_id, "modeId": agent_mode }),
            )
            .await?;
        Ok(())
    }

    pub async fn cancel(&mut self) -> Result<(), SessionError> {
        self.conn
            .notify("session/cancel", serde_json::json!({ "sessionId": self.session_id }))
            .await?;
        Ok(())
    }

    /// Read one frame and turn it into events, answering capabilities inline.
    pub async fn pump(&mut self) -> Result<Vec<AgentEvent>, SessionError> {
        let Some(Incoming { id, method, params }) = self.conn.next_incoming().await? else {
            return Ok(Vec::new());
        };
        let worktree = self.conn.worktree.clone();
        match (method.as_str(), id) {
            ("fs/read_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                match handle_fs_read(&worktree, path) {
                    Ok(content) => {
                        self.conn.respond(id, serde_json::json!({ "content": content })).await?
                    }
                    // Answered rather than left hanging: an unanswered request
                    // stalls the agent forever, and a refusal it can see is
                    // better than a turn that never ends.
                    Err(_) => self.conn.respond(id, serde_json::json!({ "content": "" })).await?,
                }
                Ok(Vec::new())
            }
            ("fs/write_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                let content = params["content"].as_str().unwrap_or_default();
                let outcome = handle_fs_write(&worktree, path, content);
                self.conn.respond(id, serde_json::json!({})).await?;
                match outcome {
                    Ok(event) => Ok(vec![event]),
                    Err(_) => Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]),
                }
            }
            ("session/request_permission", Some(id)) => {
                // The id is kept as JSON text so the client can hand back
                // exactly what the adapter sent; inventing a new id here would
                // make the answer unroutable.
                let request_id = serde_json::to_string(&id).unwrap_or_default();
                Ok(vec![permission_event(&request_id, &params)])
            }
            ("session/update", _) => {
                let rpc = Rpc { method: Some(method), params: Some(params), id: None, result: None };
                Ok(rpc
                    .session_notification()
                    .map(|n| update_to_events(&n.update))
                    .unwrap_or_else(|| vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]))
            }
            _ => Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason};

    #[test]
    fn an_fs_write_becomes_a_diff_carrying_what_was_there_before() {
        // Tier 2's whole justification: the diff is a protocol fact, not a
        // reconstruction from a vendor's private tool schema.
        let dir = std::env::temp_dir().join(format!("overnight-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old\n").unwrap();

        let event = handle_fs_write(&dir, file.to_str().unwrap(), "new\n").expect("allowed");
        let AgentEvent::ToolUpdate { diff: Some(d), .. } = event else { panic!("expected a diff") };
        assert_eq!(d.old_text.as_deref(), Some("old\n"));
        assert_eq!(d.new_text, "new\n");
    }

    #[test]
    fn a_write_outside_the_worktree_is_refused_and_says_so() {
        let dir = std::env::temp_dir().join(format!("overnight-sess2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(handle_fs_write(&dir, "/etc/passwd", "x").is_err());
    }

    #[test]
    fn a_permission_request_becomes_a_blocking_event() {
        let params = serde_json::json!({
            "toolCall": { "toolCallId": "t1", "title": "Run ls" },
            "options": [
                { "optionId": "allow", "name": "Yes", "kind": "allow_once" },
                { "optionId": "reject", "name": "No", "kind": "reject_once" }
            ]
        });
        let event = permission_event("req-1", &params);
        let AgentEvent::Permission { id, options, .. } = event else { panic!("expected permission") };
        assert_eq!(id, "req-1");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "allow");
    }

    #[test]
    fn a_reconnect_without_session_load_produces_a_visible_gap() {
        assert_eq!(
            load_unsupported_event(),
            AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
        );
    }
}

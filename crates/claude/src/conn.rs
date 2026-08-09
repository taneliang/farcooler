//! One `claude` process in stream-json mode, and the conversation with it.
//!
//! Line-delimited JSON both ways. Unlike ACP and codex this is NOT JSON-RPC:
//! ordinary traffic is typed frames with no ids at all, and only the control
//! channel correlates by `request_id`. So there is no generic `request` here —
//! a caller either sends a frame or sends a control request and waits for the
//! matching `control_response`.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum ClaudeError {
    #[error("could not start claude")]
    Spawn,
    #[error("claude closed its connection")]
    Closed,
    #[error("claude refused: {0}")]
    Refused(String),
}

/// A frame from the CLI.
#[derive(Debug)]
pub enum Incoming {
    /// An ordinary typed frame: assistant, user, result, system…
    Frame(serde_json::Value),
    /// The CLI is asking us something and is blocked until we answer —
    /// `can_use_tool` above all. Dropping these hangs the turn.
    Control { request_id: String, request: serde_json::Value },
}

fn classify(value: serde_json::Value) -> Option<Incoming> {
    match value["type"].as_str() {
        Some("control_request") => Some(Incoming::Control {
            request_id: value["request_id"].as_str().unwrap_or_default().to_string(),
            request: value["request"].clone(),
        }),
        Some(_) => Some(Incoming::Frame(value)),
        None => None,
    }
}

/// The write half, plus the process handle.
pub struct ClaudeWriter {
    stdin: ChildStdin,
    next_control_id: u64,
    pub worktree: PathBuf,
    _child: Child,
}

impl ClaudeWriter {
    async fn write(&mut self, value: serde_json::Value) -> Result<(), ClaudeError> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| ClaudeError::Closed)?;
        self.stdin.flush().await.map_err(|_| ClaudeError::Closed)
    }

    /// Send a prompt as a user message.
    pub async fn send_user(&mut self, text: &str) -> Result<(), ClaudeError> {
        self.write(serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
            "parent_tool_use_id": null,
        }))
        .await
    }

    /// Send a control request without waiting for its answer.
    ///
    /// Not waited on for the same reason `turn/start` is not: the turn cannot
    /// finish while nobody is answering the CLI's own requests, so blocking
    /// here would deadlock.
    ///
    /// `fields` are merged into the request beside its subtype, because the
    /// setters carry their value there — `set_model` wants `{ model }`, not a
    /// subtype with the value glued onto it. An earlier version built
    /// `"set_model:opus"` as the subtype, which the CLI does not recognize and
    /// ignores in silence: the picker moved and the model did not.
    pub async fn control(
        &mut self,
        subtype: &str,
        fields: serde_json::Value,
    ) -> Result<String, ClaudeError> {
        let id = format!("fc-{}", self.next_control_id);
        self.next_control_id += 1;
        let mut request = serde_json::json!({ "subtype": subtype });
        if let (Some(request), Some(fields)) = (request.as_object_mut(), fields.as_object()) {
            for (key, value) in fields {
                request.insert(key.clone(), value.clone());
            }
        }
        self.write(serde_json::json!({
            "type": "control_request",
            "request_id": id,
            "request": request,
        }))
        .await?;
        Ok(id)
    }

    /// Answer a `can_use_tool` request.
    ///
    /// `behavior`, not `decision`: the shape is the SDK's `PermissionResult`,
    /// and an `allow` must carry `updatedInput` or the CLI rejects it — a
    /// documented contract the SDK's own changelog records getting wrong.
    pub async fn answer_permission(
        &mut self,
        request_id: &str,
        allow: bool,
        input: serde_json::Value,
    ) -> Result<(), ClaudeError> {
        let response = if allow {
            serde_json::json!({ "behavior": "allow", "updatedInput": input })
        } else {
            serde_json::json!({ "behavior": "deny", "message": "Denied by the user" })
        };
        self.write(serde_json::json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id, "response": response },
        }))
        .await
    }
}

/// A connection during startup, before the read half is split off.
pub struct ClaudeConnection {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
    pub worktree: PathBuf,
}

impl ClaudeConnection {
    pub async fn spawn(
        program: &std::path::Path,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
        worktree: PathBuf,
    ) -> Result<Self, ClaudeError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(&worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Nested inside another Claude Code session the CLI neither answers nor
        // exits — the documented cause of `Status::AdapterSilent`.
        for var in crate::handshake::NESTING_VARS {
            command.env_remove(var);
        }

        let mut child = command.spawn().map_err(|_| ClaudeError::Spawn)?;
        let stdin = child.stdin.take().ok_or(ClaudeError::Spawn)?;
        let stdout = child.stdout.take().ok_or(ClaudeError::Spawn)?;
        Ok(ClaudeConnection { stdin, stdout: BufReader::new(stdout), child, worktree })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), ClaudeError> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| ClaudeError::Closed)?;
        self.stdin.flush().await.map_err(|_| ClaudeError::Closed)
    }

    /// Initialize, and read until the CLI answers.
    ///
    /// Waits for the `control_response` to our own request and NOTHING else.
    ///
    /// It waited for the `system: init` frame first, which deadlocks: measured
    /// against 2.1.226, `init` does not arrive until a prompt is sent, so a
    /// session that waits for it before allowing a prompt waits forever. In 25
    /// seconds with no prompt the CLI sends the user's hook frames and the
    /// control response, and no `init` at all.
    ///
    /// The control response carries what a session needs anyway — commands,
    /// agents, models, account — and the session id comes from `--session-id`,
    /// which the caller passes because Far Cooler assigned it at launch.
    ///
    /// Frames seen on the way are returned: the user's own hooks fire first,
    /// and dropping them unread would be a silent choice rather than a
    /// deliberate one.
    pub async fn initialize(
        &mut self,
    ) -> Result<(serde_json::Value, Vec<serde_json::Value>), ClaudeError> {
        self.write(serde_json::json!({
            "type": "control_request",
            "request_id": "fc-init",
            "request": { "subtype": "initialize" },
        }))
        .await?;

        let mut seen = Vec::new();
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).await.map_err(|_| ClaudeError::Closed)?;
            if read == 0 {
                return Err(ClaudeError::Closed);
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            if value["type"] == "control_response" {
                // An error here is the CLI refusing to start at all, worth
                // reporting in its own words rather than as a timeout later.
                if value["response"]["subtype"] == "error" {
                    let message = value["response"]["error"]
                        .as_str()
                        .unwrap_or("claude refused to initialize");
                    return Err(ClaudeError::Refused(message.to_string()));
                }
                return Ok((value["response"]["response"].clone(), seen));
            }
            seen.push(value);
        }
    }

    /// Split into a writer and a receiver, with reading moved into a task.
    pub fn split(self) -> (ClaudeWriter, mpsc::UnboundedReceiver<Incoming>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut stdout = self.stdout;
        tokio::spawn(async move {
            loop {
                let mut line = String::new();
                match stdout.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                    continue;
                };
                if let Some(incoming) = classify(value)
                    && tx.send(incoming).is_err()
                {
                    return;
                }
            }
        });
        (
            ClaudeWriter {
                stdin: self.stdin,
                next_control_id: 1,
                worktree: self.worktree,
                _child: self.child,
            },
            rx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_request_is_told_apart_from_an_ordinary_frame() {
        // Dropping a control request hangs the turn: the CLI is blocked until
        // it is answered.
        let control = serde_json::json!({
            "type": "control_request", "request_id": "7",
            "request": { "subtype": "can_use_tool" }
        });
        assert!(matches!(
            classify(control),
            Some(Incoming::Control { request_id, .. }) if request_id == "7"
        ));

        let assistant = serde_json::json!({ "type": "assistant" });
        assert!(matches!(classify(assistant), Some(Incoming::Frame(_))));
    }

    #[test]
    fn a_frame_with_no_type_is_dropped_rather_than_guessed_at() {
        assert!(classify(serde_json::json!({ "hello": 1 })).is_none());
    }
}

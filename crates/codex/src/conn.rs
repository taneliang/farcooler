//! One `codex app-server` process, and the JSON-RPC conversation with it.
//!
//! Line-delimited JSON, both directions, and bidirectional in the way that
//! matters: the server sends US requests — the approval prompts — and a client
//! that only pumps its own responses deadlocks the agent mid-turn. Hence
//! `Incoming::Request`.
//!
//! Structurally the same as `farcooler_acp::conn` and deliberately not shared
//! with it. Two differences are load-bearing: codex puts no `jsonrpc` member on
//! the wire, and its ids are plain integers rather than arbitrary JSON. A
//! common implementation would have to be configurable in exactly the places
//! that are easiest to get wrong.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("could not start codex app-server")]
    Spawn,
    #[error("codex app-server closed its connection")]
    Closed,
    /// The server answered with a JSON-RPC error. Carries its own words.
    #[error("codex app-server refused: {0}")]
    Refused(String),
}

/// A frame from the server that the caller has to deal with.
#[derive(Debug)]
pub enum Incoming {
    /// The server is asking us something and is blocked until we answer.
    Request { id: serde_json::Value, method: String, params: serde_json::Value },
    /// One-way. Every `item/*`, `turn/*` and `thread/*` update is one of these.
    Notification { method: String, params: serde_json::Value },
    /// The answer to a request we sent and deliberately did not wait on.
    Response { id: serde_json::Value, result: serde_json::Value },
    /// The same, when the answer is a JSON-RPC error rather than a result.
    ///
    /// `turn/start` is the one that matters. It reports its end by the
    /// `turn/completed` notification, and a `turn/start` that FAILS never
    /// sends one — so this frame is the only announcement that the turn is
    /// over. Dropped, the pane says Working for a server that has stopped.
    Failure { id: serde_json::Value, message: String },
}

/// What a JSON-RPC `error` object actually says, as one sentence.
///
/// `data.details` folded in for the same reason `request` has always folded
/// it: the codex line puts "Internal error" in `message` and the explanation
/// underneath. Shared with `request` so a refusal reads the same whether it
/// answered a request this connection blocked on or one it did not.
fn error_detail(error: &serde_json::Value) -> String {
    let message =
        error.get("message").and_then(|m| m.as_str()).unwrap_or("codex app-server refused");
    match error.get("data").and_then(|d| d.get("details")).and_then(|d| d.as_str()) {
        Some(details) if !details.is_empty() => format!("{message}: {details}"),
        _ => message.to_string(),
    }
}

fn classify(value: serde_json::Value) -> Option<Incoming> {
    // Errors first, and by the `error` member rather than by shape: an error
    // reply carries an id and no result, which the match below cannot tell
    // apart from a frame with nothing in it.
    //
    // Without this arm an error answering `turn/start` was dropped here, so
    // `pending_turn` stayed set and no `turn/completed` was ever coming —
    // activity stayed Working forever for a server that had already refused.
    // The field comment on `pending_turn` claimed this case was covered ("so a
    // `turn/start` that fails outright is not mistaken for a running turn");
    // it described an intent the code did not carry out.
    if let Some(error) = value.get("error") {
        let id = value.get("id")?.clone();
        return Some(Incoming::Failure { id, message: error_detail(error) });
    }
    let method = value.get("method").and_then(|m| m.as_str()).map(str::to_string);
    let id = value.get("id").cloned();
    let result = value.get("result").cloned();
    match (method, id, result) {
        (None, Some(id), Some(result)) => Some(Incoming::Response { id, result }),
        (Some(method), Some(id), None) => Some(Incoming::Request {
            id,
            method,
            params: value.get("params").cloned().unwrap_or(serde_json::Value::Null),
        }),
        (Some(method), None, _) => Some(Incoming::Notification {
            method,
            params: value.get("params").cloned().unwrap_or(serde_json::Value::Null),
        }),
        _ => None,
    }
}

/// The write half, plus the process handle.
pub struct CodexWriter {
    stdin: ChildStdin,
    next_id: u64,
    pub worktree: PathBuf,
    /// Kept so the process is not reaped out from under the reader task.
    _child: Child,
}

impl CodexWriter {
    async fn write(&mut self, value: serde_json::Value) -> Result<(), CodexError> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| CodexError::Closed)?;
        self.stdin.flush().await.map_err(|_| CodexError::Closed)
    }

    /// No `jsonrpc` member — see the module doc.
    pub async fn notify(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), CodexError> {
        self.write(serde_json::json!({ "method": method, "params": params })).await
    }

    pub async fn respond(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), CodexError> {
        self.write(serde_json::json!({ "id": id, "result": result })).await
    }

    /// Send a request and return its id without waiting for the answer.
    ///
    /// `turn/start` goes out this way: waiting on it would deadlock, because
    /// the turn cannot finish while nobody is answering the server's approval
    /// requests.
    pub async fn request_no_wait(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<u64, CodexError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(serde_json::json!({ "id": id, "method": method, "params": params })).await?;
        Ok(id)
    }
}

/// A connection during startup, before the read half is split off.
pub struct CodexConnection {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    child: Child,
    next_id: u64,
    pub worktree: PathBuf,
    /// Notifications seen while waiting for a response, kept in order.
    ///
    /// `thread/start` streams `thread/started` and the MCP status updates while
    /// its own response is still in flight. Dropping them would lose the first
    /// frames of every session.
    pending: Vec<(String, serde_json::Value)>,
}

impl CodexConnection {
    pub async fn spawn(
        program: &std::path::Path,
        args: &[String],
        env: &std::collections::BTreeMap<String, String>,
        worktree: PathBuf,
    ) -> Result<Self, CodexError> {
        let mut child = Command::new(program)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .current_dir(&worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| CodexError::Spawn)?;
        let stdin = child.stdin.take().ok_or(CodexError::Spawn)?;
        let stdout = child.stdout.take().ok_or(CodexError::Spawn)?;
        Ok(CodexConnection {
            stdin,
            stdout: BufReader::new(stdout),
            child,
            next_id: 1,
            worktree,
            pending: Vec::new(),
        })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), CodexError> {
        self.stdin
            .write_all(format!("{value}\n").as_bytes())
            .await
            .map_err(|_| CodexError::Closed)?;
        self.stdin.flush().await.map_err(|_| CodexError::Closed)
    }

    pub async fn notify(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), CodexError> {
        self.write(serde_json::json!({ "method": method, "params": params })).await
    }

    /// Send a request and read until its answer arrives.
    ///
    /// Notifications seen on the way are kept rather than dropped — see
    /// `pending`. Only used during startup, where nothing races this read.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CodexError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(serde_json::json!({ "id": id, "method": method, "params": params })).await?;

        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).await.map_err(|_| CodexError::Closed)?;
            if read == 0 {
                return Err(CodexError::Closed);
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            if value.get("id").and_then(|i| i.as_u64()) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(CodexError::Refused(error_detail(error)));
                }
                return Ok(value.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
            if let Some(method) = value.get("method").and_then(|m| m.as_str())
                && value.get("id").is_none()
            {
                self.pending.push((
                    method.to_string(),
                    value.get("params").cloned().unwrap_or(serde_json::Value::Null),
                ));
            }
        }
    }

    /// The notifications seen while waiting on startup requests, in order.
    pub fn take_pending(&mut self) -> Vec<(String, serde_json::Value)> {
        std::mem::take(&mut self.pending)
    }

    /// Split into a writer and a receiver, with reading moved into a task.
    ///
    /// The same reasoning as `AcpConnection::split`: `read_line` is not
    /// cancellation safe, so it cannot appear in a `select!`. An mpsc receiver
    /// can, and dropping one mid-await leaves the message in the channel.
    pub fn split(self) -> (CodexWriter, mpsc::UnboundedReceiver<Incoming>) {
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
            CodexWriter {
                stdin: self.stdin,
                next_id: self.next_id,
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
    fn a_notification_is_told_apart_from_a_response_by_shape_not_by_id() {
        // An earlier ACP version filtered on id alone and therefore kept only
        // the agent's questions: every `item/*` update carries no id, so the
        // whole transcript went in the bin.
        let notification = serde_json::json!({ "method": "turn/started", "params": {} });
        assert!(matches!(classify(notification), Some(Incoming::Notification { .. })));

        let response = serde_json::json!({ "id": 3, "result": {} });
        assert!(matches!(classify(response), Some(Incoming::Response { .. })));

        let request =
            serde_json::json!({ "id": 4, "method": "item/commandExecution/requestApproval" });
        assert!(matches!(classify(request), Some(Incoming::Request { .. })));
    }

    #[test]
    fn a_frame_that_is_neither_is_dropped_rather_than_guessed_at() {
        // An error with NO id answers nothing, so there is no turn it could
        // end and nobody to route it to. It used to be dropped for a weaker
        // reason — every error frame was — which is what left a refused
        // `turn/start` reporting itself as work in progress.
        assert!(classify(serde_json::json!({ "error": { "code": -32601 } })).is_none());
    }

    #[test]
    fn an_error_answering_a_turn_is_surfaced_rather_than_dropped() {
        // The `Working` forever bug on the native path. `turn/start` announces
        // its end with `turn/completed`, and a `turn/start` that fails sends
        // no such notification — this frame is the only word that the turn is
        // over. "unauthorized" is in the schema's own `codexErrorInfo` enum,
        // so an agent asking a human to sign in reaches here for real.
        let frame = serde_json::json!({
            "id": 3,
            "error": { "code": -32000, "message": "unauthorized", "data": { "details": "run `codex login`" } }
        });
        let Some(Incoming::Failure { id, message }) = classify(frame) else {
            panic!("an error answering a request must be surfaced")
        };
        assert_eq!(id, serde_json::json!(3));
        assert_eq!(message, "unauthorized: run `codex login`");
    }
}

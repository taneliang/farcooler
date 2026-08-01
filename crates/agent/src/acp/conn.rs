//! One ACP adapter process, and the JSON-RPC conversation with it.
//!
//! Line-delimited JSON, both directions. The connection is bidirectional in a
//! way LSP clients often are not: the adapter sends US requests — `fs/*` and
//! `session/request_permission` — and a client that only pumps its own
//! responses deadlocks the agent mid-turn. Hence `next_incoming`.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::acp::wire::Rpc;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("could not start the ACP adapter")]
    Spawn,
    #[error("the ACP adapter closed its connection")]
    Closed,
    #[error("malformed frame from the ACP adapter")]
    Malformed,
}

/// A frame the ADAPTER sent us that is not a response to our request.
///
/// `id` is `Some` for a request we must answer (`fs/*`,
/// `session/request_permission`) and `None` for a notification
/// (`session/update`). Both have to reach the caller: answering only requests
/// deadlocks nothing but loses the entire transcript, and reading only
/// notifications hangs the agent on its first question.
#[derive(Debug)]
pub struct Incoming {
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: serde_json::Value,
}

/// One frame, as it goes on the wire.
pub fn encode_frame(value: &serde_json::Value) -> String {
    format!("{value}\n")
}

pub struct AcpConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    pending_incoming: Vec<Incoming>,
    pub worktree: PathBuf,
}

impl AcpConnection {
    pub async fn spawn(
        program: &str,
        args: &[String],
        worktree: impl Into<PathBuf>,
    ) -> Result<Self, AcpError> {
        let worktree = worktree.into();
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherited so the adapter's own diagnostics land in the pane the
            // shim is running in, where a user can actually read them.
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| AcpError::Spawn)?;
        let stdin = child.stdin.take().ok_or(AcpError::Spawn)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(AcpError::Spawn)?);
        Ok(Self { child, stdin, stdout, next_id: 1, pending_incoming: Vec::new(), worktree })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), AcpError> {
        self.stdin
            .write_all(encode_frame(&value).as_bytes())
            .await
            .map_err(|_| AcpError::Closed)?;
        self.stdin.flush().await.map_err(|_| AcpError::Closed)
    }

    pub async fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), AcpError> {
        self.write(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }

    pub async fn respond(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), AcpError> {
        self.write(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })).await
    }

    /// Send a request and read until its response arrives.
    ///
    /// Frames that are not the answer are not discarded: notifications and
    /// adapter-initiated requests are queued for `next_incoming`, because
    /// dropping a `session/request_permission` here would hang the agent on a
    /// question nobody is ever going to answer.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await?;

        loop {
            let frame = self.read_frame().await?;
            let rpc: Rpc = serde_json::from_str(&frame).map_err(|_| AcpError::Malformed)?;
            if rpc.id.as_ref().and_then(|v| v.as_u64()) == Some(id) && rpc.result.is_some() {
                return Ok(rpc.result.unwrap_or(serde_json::Value::Null));
            }
            self.queue(rpc);
        }
    }

    /// The next frame the adapter initiated, if one has arrived.
    pub async fn next_incoming(&mut self) -> Result<Option<Incoming>, AcpError> {
        if !self.pending_incoming.is_empty() {
            return Ok(Some(self.pending_incoming.remove(0)));
        }
        let frame = self.read_frame().await?;
        let rpc: Rpc = serde_json::from_str(&frame).map_err(|_| AcpError::Malformed)?;
        self.queue(rpc);
        if self.pending_incoming.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.pending_incoming.remove(0)))
        }
    }

    /// Queue anything the adapter initiated.
    ///
    /// Keyed on `method`, not on `id`: a `session/update` notification carries
    /// no id, so filtering on id here would keep only the agent's questions and
    /// throw away everything it actually said.
    fn queue(&mut self, rpc: Rpc) {
        let Some(method) = rpc.method.clone() else { return };
        if rpc.result.is_some() {
            return;
        }
        self.pending_incoming.push(Incoming {
            id: rpc.id.clone(),
            method,
            params: rpc.params.unwrap_or(serde_json::Value::Null),
        });
    }

    async fn read_frame(&mut self) -> Result<String, AcpError> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).await.map_err(|_| AcpError::Closed)?;
        if n == 0 { Err(AcpError::Closed) } else { Ok(line) }
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake adapter: reads one line, writes one JSON-RPC result.
    fn fake_adapter() -> (String, Vec<String>) {
        (
            "/bin/sh".to_string(),
            vec![
                "-c".to_string(),
                r#"read line; printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}\n'"#
                    .to_string(),
            ],
        )
    }

    #[tokio::test]
    async fn a_request_is_matched_to_its_response_by_id() {
        let (program, args) = fake_adapter();
        let mut conn = AcpConnection::spawn(&program, &args, std::env::temp_dir())
            .await
            .expect("spawn fake adapter");
        let result = conn
            .request("initialize", serde_json::json!({ "protocolVersion": 1 }))
            .await
            .expect("a result comes back");
        assert_eq!(result["protocolVersion"], 1);
    }

    #[tokio::test]
    async fn frames_are_newline_delimited_json() {
        // ACP is line-delimited JSON-RPC on stdio. A framing mistake here shows
        // up as a hang rather than an error, so it is asserted directly.
        let line = encode_frame(&serde_json::json!({ "jsonrpc": "2.0", "method": "x" }));
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[tokio::test]
    async fn a_notification_is_queued_even_though_it_carries_no_id() {
        // The bug this asserts against threw away the entire transcript while
        // keeping the agent's questions: `session/update` carries no id, so a
        // queue that filtered on id kept only `fs/*` and permission requests.
        let (program, args) = fake_adapter();
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");
        conn.queue(Rpc {
            method: Some("session/update".into()),
            params: Some(serde_json::json!({ "sessionId": "s" })),
            id: None,
            result: None,
        });
        assert_eq!(conn.pending_incoming.len(), 1);
        assert!(conn.pending_incoming[0].id.is_none());
    }

    #[tokio::test]
    async fn a_response_to_our_own_request_is_not_queued_as_incoming() {
        // Otherwise every answer we asked for would come back a second time as
        // a request we are expected to answer, and the agent would wait forever
        // for a reply to something it never sent.
        let (program, args) = fake_adapter();
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");
        conn.queue(Rpc {
            method: None,
            params: None,
            id: Some(serde_json::json!(1)),
            result: Some(serde_json::json!({})),
        });
        assert!(conn.pending_incoming.is_empty());
    }
}

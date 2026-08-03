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
use tokio::sync::mpsc;

use crate::acp::wire::Rpc;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("could not start the ACP adapter")]
    Spawn,
    #[error("the ACP adapter closed its connection")]
    Closed,
    #[error("malformed frame from the ACP adapter")]
    Malformed,
    /// The adapter answered with a JSON-RPC error.
    ///
    /// Carries the adapter's own message, because the caller usually cannot
    /// say anything more useful than the agent already did.
    #[error("the ACP adapter refused: {0}")]
    Refused(String),
}

/// A frame from the adapter that the caller has to deal with.
///
/// All three kinds have to reach the caller, and leaving any one out breaks
/// something specific:
///
/// - dropping `Request` hangs the agent on its first permission prompt;
/// - dropping `Notification` keeps the agent's questions and discards
///   everything it actually said;
/// - dropping `Response` loses `stopReason`, which is the ONLY place a turn's
///   end is reported. Without it activity never returns to idle, `Done` never
///   happens, and no notification is ever sent — which is the entire reason
///   this feature exists.
#[derive(Debug)]
pub enum Incoming {
    /// The adapter is asking us something and is blocked until we answer.
    Request { id: serde_json::Value, method: String, params: serde_json::Value },
    /// One-way. `session/update` is all of these.
    Notification { method: String, params: serde_json::Value },
    /// The answer to a request we sent and deliberately did not wait for.
    Response { id: serde_json::Value, result: serde_json::Value },
}

/// One frame, as it goes on the wire.
pub fn encode_frame(value: &serde_json::Value) -> String {
    format!("{value}\n")
}

/// Sort a decoded frame by shape rather than by id. An earlier version
/// filtered on id alone and therefore kept only the agent's questions:
/// `session/update` carries no id, so the whole transcript went in the bin.
///
/// Shared by `AcpConnection::queue` (pre-split, used only during startup) and
/// the reader task `split` spawns, so the two can never classify a frame
/// differently.
fn classify_incoming(rpc: Rpc) -> Option<Incoming> {
    match (rpc.method, rpc.id, rpc.result) {
        // A response to something we sent and did not block on — in
        // practice `session/prompt`, whose result carries `stopReason`.
        (None, Some(id), Some(result)) => Some(Incoming::Response { id, result }),
        (Some(method), Some(id), None) => {
            Some(Incoming::Request { id, method, params: rpc.params.unwrap_or(serde_json::Value::Null) })
        }
        (Some(method), None, _) => {
            Some(Incoming::Notification { method, params: rpc.params.unwrap_or(serde_json::Value::Null) })
        }
        // An error frame, or something with no method and no result. There is
        // nothing to act on and nothing to report.
        _ => None,
    }
}

/// The write half of a split `AcpConnection`, plus the process handle.
///
/// Writing to stdin is not the unsafe half of this protocol — only
/// `read_line` on stdout is — so `notify`/`respond`/`request_no_wait` stay
/// simple `&mut self` methods here exactly as they were on `AcpConnection`.
pub struct AcpWriter {
    stdin: ChildStdin,
    next_id: u64,
    pub worktree: PathBuf,
    /// Kept so the adapter process is not reaped out from under the reader
    /// task `split` spawned. That task still owns stdout and expects frames
    /// for as long as a turn is in flight; dropping `Child` here would kill
    /// the subprocess and turn a live turn into a spurious `Closed`.
    _child: Child,
}

impl AcpWriter {
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

    /// Send a request without waiting for its answer. See the identical
    /// method on `AcpConnection` for why `session/prompt` has to go this way.
    pub async fn request_no_wait(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<u64, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await?;
        Ok(id)
    }
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
            if rpc.id.as_ref().and_then(|v| v.as_u64()) == Some(id) {
                if let Some(error) = rpc.error {
                    // Without this arm the loop waits for a `result` that is
                    // never coming and the whole session hangs — a refusal
                    // presenting as a hang, which is the hardest kind of
                    // failure to attribute. Seen for real: `session/load`
                    // answering "Session not found" for an id whose transcript
                    // does not exist yet.
                    let message =
                        error["message"].as_str().unwrap_or("the adapter refused").to_string();
                    // `message` is not always where the substance is. Probed
                    // directly against `@agentclientprotocol/codex-acp` on
                    // 2026-08-03 by sending `session/load` for an unknown id:
                    // it answers `message: "Internal error"` — which says
                    // nothing — and puts the actual explanation at
                    // `data.details`: `"no rollout found for thread id
                    // 00000000-0000-7000-8000-000000000000"`. Appended rather
                    // than substituted, so an adapter that puts something
                    // useful in both fields loses neither.
                    let detail = match error["data"]["details"].as_str() {
                        Some(details) if !details.is_empty() => format!("{message}: {details}"),
                        _ => message,
                    };
                    return Err(AcpError::Refused(detail));
                }
                if rpc.result.is_some() {
                    return Ok(rpc.result.unwrap_or(serde_json::Value::Null));
                }
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

    /// Queue a frame that is not the response `request` is waiting for.
    fn queue(&mut self, rpc: Rpc) {
        if let Some(incoming) = classify_incoming(rpc) {
            self.pending_incoming.push(incoming);
        }
    }

    /// Send a request without waiting for its answer.
    ///
    /// `session/prompt` has to go this way. Blocking on it would mean not
    /// pumping while the turn runs, and the turn cannot finish because the
    /// agent is waiting on an `fs/write_text_file` or a permission prompt that
    /// nobody is reading — a deadlock that looks exactly like a hung agent.
    pub async fn request_no_wait(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<u64, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await?;
        Ok(id)
    }

    /// Split into a writer and a task-fed stream of incoming frames.
    ///
    /// Used after startup. `BufReader::read_line` is documented by tokio as
    /// NOT cancellation safe: a `select!` branch that races it against
    /// anything else drops whatever was partially read the moment the other
    /// branch wins. `AgentSession::pump` used to do exactly that, and worse —
    /// `handle_fs_write` performed the write and only then awaited `respond`,
    /// so a cancellation between the two left the file on disk with the agent
    /// never answered, hanging forever on its own request.
    ///
    /// The fix is to give `read_line` a task that owns it exclusively and is
    /// never cancelled, and to hand callers an `mpsc::UnboundedReceiver`
    /// instead — `recv` IS cancellation safe, so it is the only thing safe to
    /// put in a `select!` branch. Do not fold this back into a `select!` over
    /// `read_line`; that is precisely the bug this method exists to close.
    /// Take the `session/update` notifications already queued, as events.
    ///
    /// A `session/load` replays the whole conversation as notifications while
    /// the load request is still in flight, so by the time it answers, the
    /// history is sitting in this queue. Taking it here lets a resumed session
    /// arrive as one batch that ends where the history ends.
    ///
    /// Requests are left exactly where they are. A `session/request_permission`
    /// dropped here would hang the agent on a question nobody will ever be
    /// asked.
    pub fn take_pending_updates(&mut self) -> Vec<crate::event::AgentEvent> {
        let mut events = Vec::new();
        let mut kept = Vec::new();
        for incoming in std::mem::take(&mut self.pending_incoming) {
            let Incoming::Notification { method, params } = &incoming else {
                kept.push(incoming);
                continue;
            };
            if method != "session/update" {
                kept.push(incoming);
                continue;
            }
            let rpc = Rpc {
                method: Some(method.clone()),
                params: Some(params.clone()),
                id: None,
                result: None,
                error: None,
            };
            match rpc.session_notification() {
                Some(n) => events.extend(crate::acp::normalize::update_to_events(&n.update)),
                None => kept.push(incoming),
            }
        }
        self.pending_incoming = kept;
        events
    }

    pub fn split(self) -> (AcpWriter, mpsc::UnboundedReceiver<Incoming>) {
        let Self { child, stdin, mut stdout, next_id, pending_incoming, worktree } = self;
        let (tx, rx) = mpsc::unbounded_channel();

        // Frames `request()` already read past while waiting for its own
        // response (a permission prompt seen during startup, say) must reach
        // the caller before anything the reader task reads next, or they
        // would be delivered out of order.
        for incoming in pending_incoming {
            if tx.send(incoming).is_err() {
                break;
            }
        }

        tokio::spawn(async move {
            loop {
                let mut line = String::new();
                let read = stdout.read_line(&mut line).await;
                match read {
                    // EOF or a broken pipe: the adapter is gone. Dropping `tx`
                    // here — not sending a value — is the signal; it is what
                    // makes the receiver's next `recv()` resolve instead of
                    // hanging on a channel that will never produce again.
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(rpc) = serde_json::from_str::<Rpc>(&line) else {
                    // A malformed frame breaks the JSON-RPC framing itself —
                    // there is no safe way to resync mid-stream — so this is
                    // treated the same as a closed pipe rather than skipped.
                    break;
                };
                match classify_incoming(rpc) {
                    Some(incoming) => {
                        if tx.send(incoming).is_err() {
                            break; // nobody is listening anymore
                        }
                    }
                    // An error frame, or something with no method and no
                    // result: nothing to act on, so read the next line rather
                    // than surfacing an empty batch to the caller.
                    None => continue,
                }
            }
        });

        (AcpWriter { stdin, next_id, worktree, _child: child }, rx)
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
            error: None,
        });
        assert_eq!(conn.pending_incoming.len(), 1);
        assert!(matches!(conn.pending_incoming[0], Incoming::Notification { .. }));
    }

    #[tokio::test]
    async fn a_response_we_did_not_block_on_is_surfaced_not_dropped() {
        // `session/prompt` is sent without waiting, and its response is the
        // ONLY place `stopReason` appears. Dropping it here means a turn never
        // reports its end, activity never returns to idle, Done never happens
        // and no notification is ever sent.
        let (program, args) = fake_adapter();
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");
        conn.queue(Rpc {
            method: None,
            params: None,
            id: Some(serde_json::json!(7)),
            result: Some(serde_json::json!({ "stopReason": "end_turn" })),
            error: None,
        });
        let Some(Incoming::Response { id, result }) = conn.pending_incoming.pop() else {
            panic!("a response must be surfaced")
        };
        assert_eq!(id, serde_json::json!(7));
        assert_eq!(result["stopReason"], "end_turn");
    }

    #[tokio::test]
    async fn a_response_is_never_mistaken_for_a_request_we_must_answer() {
        // Answering a response would send the agent a reply to something it
        // never asked, and leave whatever it IS waiting on unanswered.
        let (program, args) = fake_adapter();
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");
        conn.queue(Rpc {
            method: None,
            params: None,
            id: Some(serde_json::json!(1)),
            result: Some(serde_json::json!({})),
            error: None,
        });
        assert!(!conn.pending_incoming.iter().any(|i| matches!(i, Incoming::Request { .. })));
    }

    #[tokio::test]
    async fn an_error_reply_is_returned_rather_than_waited_on_forever() {
        // The bug this pins presented as a hang, not as an error: `request`
        // looped for a `result` that an error reply never carries, so a
        // refusal the adapter stated plainly looked like a wedged agent with a
        // blank pane. Seen for real when `session/load` answered "Session not
        // found" for an id whose transcript did not exist yet.
        let program = "/bin/sh".to_string();
        let args = vec![
            "-c".to_string(),
            r#"read line; printf '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Session not found"}}\n'"#
                .to_string(),
        ];
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            conn.request("session/load", serde_json::json!({})),
        )
        .await
        .expect("must not hang");

        match outcome {
            Err(AcpError::Refused(message)) => assert!(message.contains("Session not found")),
            other => panic!("expected the adapter's own refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_error_with_data_details_folds_the_details_in() {
        // The exact bytes `@agentclientprotocol/codex-acp` answered when
        // probed directly on 2026-08-03: `session/load` for an unknown id.
        // `message` alone says nothing ("Internal error"); the explanation is
        // in `data.details`. A caller reading only `message` — which is what
        // this connection did before this test existed — would show the user
        // "Internal error" and throw away the one useful sentence.
        let program = "/bin/sh".to_string();
        let args = vec![
            "-c".to_string(),
            r#"read line; printf '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Internal error","data":{"details":"no rollout found for thread id 00000000-0000-7000-8000-000000000000"}}}\n'"#
                .to_string(),
        ];
        let mut conn =
            AcpConnection::spawn(&program, &args, std::env::temp_dir()).await.expect("spawn");

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            conn.request("session/load", serde_json::json!({})),
        )
        .await
        .expect("must not hang");

        match outcome {
            Err(AcpError::Refused(message)) => {
                assert!(message.contains("Internal error"), "{message}");
                assert!(
                    message.contains("no rollout found for thread id"),
                    "{message}"
                );
            }
            other => panic!("expected the adapter's own refusal, got {other:?}"),
        }
    }
}

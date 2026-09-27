//! A worktree made from a phone is claimed for its repository's Main, which
//! takes a `workspace.list` first. When that list can't be read, the create
//! still goes ahead, unclaimed: the claim is a convenience, and the worktree
//! is what was asked for.
//!
//! The peer here is not a daemon. It says hello as a runner with
//! `workstreams`, refuses `workspace.list`, and answers `worktree.create`
//! with a worktree, keeping what it was sent.

use std::sync::{Arc, Mutex};

use farcooler_client::session::Session;
use farcooler_protocol::v1::{
    self as pb, ServerHello, WireEnvelope, request, result, response, wire_envelope,
};
use farcooler_transport::codec::{FrameReader, FrameWriter};

fn envelope(body: wire_envelope::Body) -> WireEnvelope {
    WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(body),
    }
}

/// A runner with workstreams whose workspace list can't be read. Returns the
/// `worktree.create` payloads it is sent.
async fn a_runner_that_cannot_list(socket: &std::path::Path) -> Arc<Mutex<Vec<pb::WorktreeCreate>>> {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    let created = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&created);
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = envelope(wire_envelope::Body::ServerHello(ServerHello {
            selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            daemon_version: "a runner whose list fails".into(),
            max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
            max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
            capabilities: vec![
                farcooler_protocol::capability::WORKTREES.to_string(),
                farcooler_protocol::capability::TERMINALS.to_string(),
                farcooler_protocol::capability::WORKSTREAMS.to_string(),
            ],
            ..Default::default()
        }));
        if writer.write_frame(&hello).await.is_err() {
            return;
        }
        while let Ok(Some(frame)) = reader.read_frame().await {
            let Some(wire_envelope::Body::Request(req)) = frame.body else { continue };
            let outcome = match (req.method.as_str(), req.payload) {
                ("worktree.create", Some(request::Payload::WorktreeCreate(p))) => {
                    kept.lock().unwrap().push(p.clone());
                    response::Outcome::Result(pb::Result {
                        value: Some(result::Value::Worktree(pb::Worktree {
                            id: bytes::Bytes::copy_from_slice(&[1; 16]),
                            task_name: p.task_name,
                            branch: p.branch,
                            ..Default::default()
                        })),
                    })
                }
                _ => response::Outcome::Error(pb::Error {
                    code: pb::ErrorCode::OperationFailed as i32,
                    retryable: true,
                    message: "operation failed".into(),
                    ..Default::default()
                }),
            };
            let reply = envelope(wire_envelope::Body::Response(pb::Response {
                request_id: req.request_id,
                outcome: Some(outcome),
            }));
            if writer.write_frame(&reply).await.is_err() {
                return;
            }
        }
    });
    created
}

#[tokio::test]
async fn a_worktree_is_made_unclaimed_when_the_workspace_list_fails() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let created = a_runner_that_cannot_list(&socket).await;
    let mut session = Session::connect_local(&socket).await.expect("connect");

    let made = session
        .create_worktree(uuid::Uuid::now_v7(), "phone task", "feat/phone", "HEAD", "", false)
        .await
        .expect("the create goes ahead");
    assert_eq!(made.task_name, "phone task");
    let sent = created.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one create reached the runner");
    assert_eq!(sent[0].workspace_id, None, "claimed for a workspace nobody could list");
}

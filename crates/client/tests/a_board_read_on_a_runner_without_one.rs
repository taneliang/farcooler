//! A board read on a runner that keeps no board is refused HERE, by
//! `Session::tasks` and `Session::task` themselves, before a request is sent.
//!
//! Through the two reads rather than the helper they call, because the helper
//! being right says nothing about whether the reads still call it: deleting
//! the `require(...)` line from either would leave every other test green,
//! since a real daemon refuses with the same code. So the peer here is not a
//! daemon. It answers the handshake as a runner from before the board — the
//! two floor capabilities and nothing else — and then only counts what it is
//! sent. Refused with the runner's own code and nothing on the wire is the
//! claim; a request arriving is the failure.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use farcooler_client::session::{Session, SessionError};
use farcooler_protocol::v1::{ServerHello, WireEnvelope, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};

/// A peer that says hello as an old runner and counts every request after.
async fn an_old_runner(socket: &std::path::Path) -> Arc<AtomicUsize> {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        // The client's hello, answered with the capabilities a runner from
        // before the board advertised.
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let reply = WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(wire_envelope::Body::ServerHello(ServerHello {
                selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                daemon_version: "an old runner".into(),
                max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
                max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
                capabilities: vec![
                    farcooler_protocol::capability::WORKSPACES.to_string(),
                    farcooler_protocol::capability::TERMINALS.to_string(),
                ],
                ..Default::default()
            })),
        };
        if writer.write_frame(&reply).await.is_err() {
            return;
        }
        // Counted, never answered: a read that reached here has already done
        // the wrong thing, and an answer would only let it look right.
        while let Ok(Some(frame)) = reader.read_frame().await {
            if matches!(frame.body, Some(wire_envelope::Body::Request(_))) {
                counted.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    requests
}

fn refused_as_unsupported(result: Result<serde_json::Value, SessionError>, what: &str) {
    match result {
        Err(SessionError::Refused { code, retryable, .. }) => {
            assert_eq!(
                code,
                farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32,
                "{what} was refused with the wrong code"
            );
            assert!(!retryable, "{what}: asking again will not grow the runner a board");
        }
        other => panic!("{what} on a runner without a board: {other:?}"),
    }
}

#[tokio::test]
async fn task_list_and_task_get_are_refused_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("old.sock");
    let requests = an_old_runner(&socket).await;

    let mut session = Session::connect_local(&socket).await.expect("connect to the old runner");
    assert!(!session.can(farcooler_protocol::capability::TASKS));

    // Bounded: the peer never answers, so a read that sent its request waits
    // forever. Two seconds is a refusal that never needed the wire, many
    // times over.
    let bound = std::time::Duration::from_secs(2);
    let list = tokio::time::timeout(bound, session.tasks(uuid::Uuid::now_v7()))
        .await
        .expect("task.list went to the runner and waited for an answer");
    refused_as_unsupported(list, "task.list");
    let get = tokio::time::timeout(bound, session.task(uuid::Uuid::now_v7()))
        .await
        .expect("task.get went to the runner and waited for an answer");
    refused_as_unsupported(get, "task.get");

    // Give a request that was sent anyway the time to arrive and be counted.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(requests.load(Ordering::SeqCst), 0, "a board read reached a runner without a board");
}

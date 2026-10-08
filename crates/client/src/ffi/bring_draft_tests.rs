//! `terminal.bring_draft` from an app (ov-369), the whole way: `dispatch` with
//! the JSON a Mac or a phone passes, a `Session`, and a runner that answers
//! as the daemon does and records what reached it. The apps read `text` and
//! `cleared` off what this answers.

use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::{self as pb, response, result, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};
use serde_json::json;

use super::dispatch;
use crate::session::{Session, SessionError};

/// A runner offering `capabilities` whose box holds `box_text`: a read
/// answers it, a clear empties it.
async fn a_runner(socket: &std::path::Path, capabilities: Vec<String>, box_text: &str) -> Arc<Mutex<Vec<pb::Request>>> {
    let envelope = |body| pb::WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(body),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let mut held = box_text.to_string();
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = envelope(wire_envelope::Body::ServerHello(pb::ServerHello {
            selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            daemon_version: "bring".into(),
            max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
            max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
            capabilities,
            ..Default::default()
        }));
        if writer.write_frame(&hello).await.is_err() {
            return;
        }
        while let Ok(Some(frame)) = reader.read_frame().await {
            let Some(wire_envelope::Body::Request(req)) = frame.body else { continue };
            let Some(pb::request::Payload::BringDraft(p)) = req.payload.as_ref() else { continue };
            let brought = if p.expected.is_empty() {
                pb::BroughtDraft { text: held.clone(), cleared: false }
            } else {
                held.clear();
                pb::BroughtDraft { text: p.expected.clone(), cleared: true }
            };
            recorded.lock().unwrap().push(req.clone());
            let reply = envelope(wire_envelope::Body::Response(pb::Response {
                request_id: req.request_id,
                outcome: Some(response::Outcome::Result(pb::Result { value: Some(result::Value::BroughtDraft(brought)) })),
            }));
            if writer.write_frame(&reply).await.is_err() {
                return;
            }
        }
    });
    seen
}

/// A read, then a clear naming exactly what the read answered: each answered
/// as the apps read it, and sent targeted at the terminal.
#[tokio::test]
async fn a_read_then_a_clear_reach_the_runner_and_answer_text_and_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let all = farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect();
    let seen = a_runner(&socket, all, "fix the login\nthen the tests").await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let terminal = uuid::Uuid::now_v7();

    let read = dispatch(&session, "terminal.bring_draft", &json!({ "terminal": terminal.to_string() })).await.expect("read");
    assert_eq!(read, json!({ "text": "fix the login\nthen the tests", "cleared": false }));
    let clear = json!({ "terminal": terminal.to_string(), "expected": "fix the login\nthen the tests" });
    let cleared = dispatch(&session, "terminal.bring_draft", &clear).await.expect("cleared");
    assert_eq!(cleared, json!({ "text": "fix the login\nthen the tests", "cleared": true }));

    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    for (req, expected) in seen.iter().zip(["", "fix the login\nthen the tests"]) {
        let Some(pb::request::Payload::BringDraft(p)) = &req.payload else { panic!("{req:?}") };
        assert_eq!(p.expected, expected);
        assert_eq!(p.terminal_id.as_ref(), terminal.as_bytes());
        assert_eq!(req.target_resource_id.as_deref(), Some(terminal.as_bytes().as_slice()), "kept in order with the pane's input");
    }
}

/// A runner from before Bring Here: refused here, nothing sent.
#[tokio::test]
async fn a_runner_without_bring_draft_is_never_asked() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let older = farcooler_protocol::capability::ALL
        .iter()
        .filter(|c| **c != farcooler_protocol::capability::BRING_DRAFT)
        .map(|c| c.to_string())
        .collect();
    let seen = a_runner(&socket, older, "a draft").await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let asked = dispatch(&session, "terminal.bring_draft", &json!({ "terminal": uuid::Uuid::now_v7().to_string() })).await;
    let unsupported = farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32;
    assert!(matches!(asked, Err(SessionError::Refused { code, .. }) if code == unsupported), "{asked:?}");
    assert!(seen.lock().unwrap().is_empty());
}

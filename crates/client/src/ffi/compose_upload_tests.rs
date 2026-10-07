//! `terminal.compose`'s images from an app (ov-393), the whole way: `dispatch`
//! with the JSON a Mac or a phone passes, a `Session`, and a runner that
//! records what reached it. A runner with `compose_upload` is sent each image
//! in `terminal.paste_file` chunks with `stage`, then a compose naming them;
//! one without gets them inside the compose, 900 KB together at most.

use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, response, result, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};
use serde_json::json;

use super::dispatch;
use crate::session::{Session, SessionError};

/// A runner that advertises `capabilities`, takes every paste chunk and
/// answers a compose Sent, and records each request.
async fn a_recording_runner(socket: &std::path::Path, capabilities: Vec<String>) -> Arc<Mutex<Vec<pb::Request>>> {
    let envelope = |body| pb::WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(body),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorded = seen.clone();
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = envelope(wire_envelope::Body::ServerHello(pb::ServerHello {
            selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            daemon_version: "records".into(),
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
            let value = match req.payload.as_ref() {
                Some(pb::request::Payload::TerminalFilePut(p)) => {
                    let stored = p.offset + p.chunk.len() as u64;
                    let path = (stored == p.total_size).then(|| "/staged".to_string());
                    result::Value::TerminalFilePut(pb::TerminalFilePutResult { stored, path })
                }
                _ => result::Value::TerminalTold(pb::TerminalTold { queued: false }),
            };
            recorded.lock().unwrap().push(req.clone());
            let reply = envelope(wire_envelope::Body::Response(pb::Response {
                request_id: req.request_id,
                outcome: Some(response::Outcome::Result(pb::Result { value: Some(value) })),
            }));
            if writer.write_frame(&reply).await.is_err() {
                return;
            }
        }
    });
    seen
}

fn png(len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.resize(len, 0x5a);
    b
}

fn args(terminal: uuid::Uuid, image: &[u8]) -> serde_json::Value {
    json!({
        "terminal": terminal.to_string(),
        "text": "two\nlines",
        "images": [{ "mime": "image/png", "base64": farcooler_core::base64::encode(image) }],
    })
}

/// The compose a runner was sent: its blocks past the text.
fn images_of(compose: &pb::Request) -> Vec<Content> {
    let Some(pb::request::Payload::AgentPrompt(p)) = &compose.payload else { panic!("{compose:?}") };
    p.blocks.iter().skip(1).filter_map(|b| b.content.clone()).collect()
}

/// With `compose_upload`: a 10 MB image goes in 128 KB chunks, each staged
/// and naming the capability, all under one transfer id; then the compose
/// names that id, and the capability too.
#[tokio::test]
async fn a_runner_that_takes_uploads_is_sent_the_image_in_chunks_first() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let all = farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect();
    let seen = a_recording_runner(&socket, all).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let terminal = uuid::Uuid::now_v7();
    let image = png(10 * 1024 * 1024);

    let answer = dispatch(&session, "terminal.compose", &args(terminal, &image)).await.expect("sent");
    assert_eq!(answer, json!({ "queued": false }));

    let seen = seen.lock().unwrap().clone();
    let (chunks, rest): (Vec<_>, Vec<_>) = seen.iter().partition(|r| r.method == "terminal.paste_file");
    assert_eq!(chunks.len(), image.len().div_ceil(farcooler_protocol::PASTE_CHUNK_BYTES), "{}", chunks.len());
    let mut ids = std::collections::HashSet::new();
    for chunk in &chunks {
        let Some(pb::request::Payload::TerminalFilePut(p)) = &chunk.payload else { panic!() };
        assert!(p.stage, "kept for the compose, typed nowhere");
        assert_eq!(chunk.required_capabilities, ["compose_upload"]);
        ids.insert(p.transfer_id.clone());
    }
    assert_eq!(ids.len(), 1, "one transfer");
    let compose = rest.iter().find(|r| r.method == "terminal.compose").expect("a compose");
    assert_eq!(compose.required_capabilities, ["compose_upload"]);
    assert_eq!(images_of(compose), [Content::StagedImage(ids.into_iter().next().unwrap())]);
}

/// Without it: a small image rides inside the compose as it always did, and
/// a 10 MB one is refused here, `images_too_large`, with nothing sent.
#[tokio::test]
async fn a_runner_without_uploads_keeps_the_old_cap() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let older = farcooler_protocol::capability::ALL
        .iter()
        .filter(|c| **c != farcooler_protocol::capability::COMPOSE_UPLOAD)
        .map(|c| c.to_string())
        .collect();
    let seen = a_recording_runner(&socket, older).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let terminal = uuid::Uuid::now_v7();

    match dispatch(&session, "terminal.compose", &args(terminal, &png(10 * 1024 * 1024))).await {
        Err(SessionError::Refused { what, .. }) => assert_eq!(what, "images_too_large"),
        other => panic!("{other:?}"),
    }
    assert!(seen.lock().unwrap().iter().all(|r| r.method != "terminal.paste_file" && r.method != "terminal.compose"));

    let small = png(500 * 1024);
    dispatch(&session, "terminal.compose", &args(terminal, &small)).await.expect("sent");
    let seen = seen.lock().unwrap().clone();
    assert!(seen.iter().all(|r| r.method != "terminal.paste_file"), "nothing uploaded");
    let compose = seen.iter().find(|r| r.method == "terminal.compose").expect("a compose");
    assert!(compose.required_capabilities.is_empty());
    let images = images_of(compose);
    let [Content::Image(carried)] = images.as_slice() else { panic!("{compose:?}") };
    assert_eq!(carried.data.as_ref(), small.as_slice());
}

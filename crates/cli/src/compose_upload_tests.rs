//! `terminal compose --image` against a runner that records what reached it
//! (ov-393): with `compose_upload`, each image staged in chunks and the
//! compose naming it and the capability; without, carried, 900 KB at most.

use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, response, result, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};

use super::request_for;

/// A runner advertising `capabilities` that takes every paste chunk, and
/// records each request.
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
        let (mut reader, mut writer) = (FrameReader::new(read), FrameWriter::new(write));
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

fn blocks(image: usize) -> Vec<pb::AgentPromptBlock> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.resize(image, 0x5a);
    vec![
        pb::AgentPromptBlock { content: Some(Content::Text("two\nlines".into())) },
        pb::AgentPromptBlock { content: Some(Content::Image(pb::ImageBlock { mime_type: "image/png".into(), data: png.into() })) },
    ]
}

/// A socket path short enough for `bind`, in a directory of its own.
fn socket() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fcc-{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    (dir.join("r.sock"), dir)
}

#[tokio::test]
async fn a_runner_that_takes_uploads_gets_the_image_staged_first() {
    let (path, dir) = socket();
    let all: Vec<String> = farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect();
    let seen = a_recording_runner(&path, all.clone()).await;
    let client = farcooler_transport::Client::connect(&path, "test", "0").await.expect("connected");
    let id = uuid::Uuid::now_v7();
    let ask = request_for(&client, id, blocks(10 * 1024 * 1024), &all).await.expect("a request");
    assert_eq!(ask.required_capabilities, ["agent_compose", "compose", "compose_upload"]);
    let Some(pb::request::Payload::AgentPrompt(p)) = &ask.payload else { panic!() };
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), (10 * 1024 * 1024usize).div_ceil(farcooler_protocol::PASTE_CHUNK_BYTES));
    let mut ids = std::collections::HashSet::new();
    for chunk in &seen {
        let Some(pb::request::Payload::TerminalFilePut(put)) = &chunk.payload else { panic!("{chunk:?}") };
        assert!(put.stage && chunk.required_capabilities == ["compose_upload"], "{:?}", chunk.required_capabilities);
        ids.insert(put.transfer_id.clone());
    }
    assert_eq!(ids.len(), 1);
    assert_eq!(p.blocks[1].content, Some(Content::StagedImage(ids.into_iter().next().unwrap())));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn a_runner_without_uploads_keeps_the_old_cap() {
    let (path, dir) = socket();
    let older: Vec<String> = farcooler_protocol::capability::ALL
        .iter()
        .filter(|c| **c != farcooler_protocol::capability::COMPOSE_UPLOAD)
        .map(|c| c.to_string())
        .collect();
    let seen = a_recording_runner(&path, older.clone()).await;
    let client = farcooler_transport::Client::connect(&path, "test", "0").await.expect("connected");
    let id = uuid::Uuid::now_v7();
    let refused = request_for(&client, id, blocks(10 * 1024 * 1024), &older).await.unwrap_err();
    assert!(refused.to_string().contains("too large to send together"), "{refused}");
    let ask = request_for(&client, id, blocks(500 * 1024), &older).await.expect("a request");
    assert_eq!(ask.required_capabilities, ["agent_compose", "compose"]);
    let Some(pb::request::Payload::AgentPrompt(p)) = &ask.payload else { panic!() };
    assert!(matches!(&p.blocks[1].content, Some(Content::Image(i)) if i.data.len() == 500 * 1024));
    assert!(seen.lock().unwrap().is_empty(), "nothing uploaded");
    let _ = std::fs::remove_dir_all(dir);
}

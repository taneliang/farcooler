//! A phone's Files calls, the whole way: `dispatch` with the JSON an app
//! passes, a `Session`, and a runner that records what reached it (ov-259).

use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::{self as pb, response, result, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};
use serde_json::json;

use super::dispatch;
use crate::session::{Session, SessionError};

/// What reached the runner for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    required: Vec<String>,
    worktree_id: Vec<u8>,
    folder: String,
    path: String,
}

/// A runner that advertises `capabilities`, answers the three Files methods
/// and `host.health` with fixed values, and records each request.
async fn a_recording_runner(socket: &std::path::Path, capabilities: Vec<String>) -> Arc<Mutex<Vec<Seen>>> {
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
            let (mut worktree_id, mut folder, mut path) = (Vec::new(), String::new(), String::new());
            let value = match req.payload.as_ref() {
                Some(pb::request::Payload::WorktreeDir(d)) => {
                    (worktree_id, folder, path) = (d.worktree_id.to_vec(), d.folder.clone(), d.path.clone());
                    result::Value::WorktreeDir(pb::WorktreeDir {
                        path: d.path.clone(),
                        truncated: false,
                        entries: vec![pb::WorktreeDirEntry {
                            name: "src".into(),
                            kind: pb::WorktreeEntryKind::Directory as i32,
                            ..Default::default()
                        }],
                    })
                }
                Some(pb::request::Payload::WorktreeFile(f)) => {
                    (worktree_id, folder, path) = (f.worktree_id.to_vec(), f.folder.clone(), f.path.clone());
                    result::Value::WorktreeFile(pb::WorktreeFile {
                        path: f.path.clone(),
                        state: pb::WorktreeFileState::Text as i32,
                        size: 3,
                        text: "hi\n".into(),
                        ..Default::default()
                    })
                }
                _ => result::Value::Host(pb::Host {
                    read_only_folders: vec![pb::ReadOnlyFolder { name: "logs".into(), path: "/var/log".into() }],
                    ..Default::default()
                }),
            };
            recorded.lock().unwrap().push(Seen {
                method: req.method.clone(),
                required: req.required_capabilities.clone(),
                worktree_id,
                folder,
                path,
            });
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

fn everything() -> Vec<String> {
    farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect()
}

/// A folder call names `read_only_folders` and carries no worktree; a
/// worktree call names nothing and carries no folder. Goes red when the
/// session drops either the field or the required capability.
#[tokio::test]
async fn a_folder_call_names_its_capability_and_a_worktree_call_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let seen = a_recording_runner(&socket, everything()).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let wt = uuid::Uuid::now_v7();

    let listing = dispatch(&session, "worktree.list_dir", &json!({ "folder": "logs", "path": "2026" })).await.unwrap();
    assert_eq!(listing["entries"][0]["name"], "src", "{listing}");
    let file = dispatch(&session, "worktree.read_file", &json!({ "folder": "logs", "path": "a.log" })).await.unwrap();
    assert_eq!(file["text"], "hi\n", "{file}");
    dispatch(&session, "worktree.list_dir", &json!({ "worktree": wt.to_string() })).await.unwrap();
    dispatch(&session, "worktree.read_file", &json!({ "worktree": wt.to_string(), "path": "README.md" }))
        .await
        .unwrap();

    let seen = seen.lock().unwrap().clone();
    let files: Vec<_> = seen.iter().filter(|s| s.method.starts_with("worktree.")).collect();
    assert_eq!(files.len(), 4, "{seen:?}");
    for (call, method, path) in [(0, "worktree.list_dir", "2026"), (1, "worktree.read_file", "a.log")] {
        assert_eq!(files[call].method, method);
        assert_eq!(files[call].required, ["read_only_folders"], "{:?}", files[call]);
        assert_eq!((files[call].folder.as_str(), files[call].path.as_str()), ("logs", path));
        assert!(files[call].worktree_id.is_empty());
    }
    for (call, path) in [(2, ""), (3, "README.md")] {
        assert!(files[call].required.is_empty(), "{:?}", files[call]);
        assert!(files[call].folder.is_empty());
        assert_eq!((files[call].worktree_id.as_slice(), files[call].path.as_str()), (wt.as_bytes().as_slice(), path));
    }
}

/// Both places, or neither, never leave the phone.
#[tokio::test]
async fn both_places_or_neither_is_refused_before_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let seen = a_recording_runner(&socket, everything()).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let wt = uuid::Uuid::now_v7().to_string();
    for method in ["worktree.list_dir", "worktree.read_file"] {
        for args in [json!({ "worktree": wt, "folder": "logs" }), json!({ "path": "src" })] {
            match dispatch(&session, method, &args).await {
                Err(SessionError::Protocol(m)) => assert!(m.contains("worktree or a folder"), "{m}"),
                other => panic!("{method} {args}: {other:?}"),
            }
        }
    }
    assert!(seen.lock().unwrap().is_empty(), "something reached the runner");
}

/// A runner without `read_only_folders` is refused without a round trip, and
/// `host` says null, not an empty list.
#[tokio::test]
async fn an_older_runner_has_no_folders_and_refuses_one() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let caps = vec!["worktree_files".to_string()];
    let seen = a_recording_runner(&socket, caps).await;
    let session = Session::connect_local(&socket).await.expect("connect");

    match dispatch(&session, "worktree.list_dir", &json!({ "folder": "logs" })).await {
        Err(SessionError::Refused { code, .. }) => assert_eq!(code, pb::ErrorCode::CapabilityUnsupported as i32),
        other => panic!("{other:?}"),
    }
    let host = dispatch(&session, "host", &json!({})).await.unwrap();
    assert!(host["readOnlyFolders"].is_null(), "{host}");
    assert_eq!(seen.lock().unwrap().iter().filter(|s| s.method.starts_with("worktree.")).count(), 0);
}

/// `host` carries the folder names, and only the names.
#[tokio::test]
async fn host_carries_the_folder_names() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    a_recording_runner(&socket, everything()).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let host = dispatch(&session, "host", &json!({})).await.unwrap();
    assert_eq!(host["readOnlyFolders"], json!(["logs"]), "{host}");
}

/// Both calls are routed to an app under their own names.
#[test]
fn the_files_calls_are_routed() {
    use farcooler_protocol::method::Method;
    assert_eq!(super::route::route(Method::WorktreeListDir), Some("worktree.list_dir"));
    assert_eq!(super::route::route(Method::WorktreeReadFile), Some("worktree.read_file"));
}

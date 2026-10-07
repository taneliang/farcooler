//! `terminal.compose` with its images uploaded first (ov-393), end to end
//! over a socket: the client core's own `Session::compose` against this
//! daemon's handler, serving the board and a claude stand-in, so a 10 MB
//! screenshot travels as `terminal.paste_file` chunks with `stage` and the
//! compose names it. And what's left on the runner after a send, sent or
//! refused.

use std::path::{Path, PathBuf};

use farcooler_client::session::Session;
use farcooler_transport::{HandshakeConfig, Peer, UnixListenerServer};

use super::compose_tests::{hook_on_submit, idle_claude};
use super::*;

/// The board's daemon, served on a socket of its own as `farcoolerd` serves
/// it, and a client core connected to it. The socket's directory goes with
/// the test.
async fn served(b: &Board) -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let server = UnixListenerServer::bind(&socket).unwrap();
    let (svc, watcher) = (b.svc.clone(), b.watcher.clone());
    tokio::spawn(async move {
        let _ = server
            .serve(move |_| {
                let peer = Peer { client_id: None, scope: farcooler_protocol::v1::Scope::HostAdmin };
                let stop = Arc::new(tokio::sync::Notify::new());
                Some((
                    HandshakeConfig { daemon_version: farcooler_protocol::BUILD.to_string() },
                    crate::rpc::RpcFactory::new(svc.clone(), watcher.clone(), stop, peer),
                ))
            })
            .await;
    });
    let session = Session::connect_local(&socket).await.expect("connected");
    (dir, session)
}

/// A PNG's signature and `len` bytes in all: what the runner sniffs.
fn png(len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.resize(len, 0x5a);
    b
}

/// Files directly in `dir` whose name `named` picks.
fn files(dir: &Path, named: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.file_name().to_str().is_some_and(&named)).map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// A 10 MB screenshot, ten times what one request may carry, composed from
/// the client core: claude's box takes it as `[Image #1]` and the text, Sent;
/// the staged upload is gone, and the copy claude was pasted is whole.
#[tokio::test]
async fn a_ten_megabyte_screenshot_composes_over_the_socket() {
    let b = board().await;
    let (agent, si) = idle_claude(&b).await;
    let _hook = hook_on_submit(&b, &si);
    let (_socket, session) = served(&b).await;
    assert!(session.can(farcooler_protocol::capability::COMPOSE_UPLOAD), "{:?}", session.capabilities());
    let image = png(10 * 1024 * 1024);
    let queued = session.compose(agent.id, "what's in\nthis one", &[("image/png".into(), image.clone())]).await;
    assert!(!queued.expect("sent"), "Sent, not Queued");
    assert_eq!(si.submitted(), ["[Image #1] what's in\\nthis one"], "{}", si.log());

    let root = b.svc.root_dir();
    let staged = crate::pastes::staged::staged_dir_in(root).unwrap();
    assert!(files(&staged, |_| true).is_empty(), "the upload is used once and deleted");
    let composed = files(&crate::paths::pastes_dir_in(root).unwrap(), crate::pastes::staged::is_composed);
    assert_eq!(composed.len(), 1, "{composed:?}");
    assert_eq!(std::fs::read(&composed[0]).unwrap(), image, "whole, through every chunk");
}

/// A send refused at the gate, after its images were written and before a
/// path was pasted (codex: `unsupported`), leaves nothing behind: not the
/// upload, not the copy.
#[tokio::test]
async fn a_refused_compose_leaves_no_image_behind() {
    let b = board().await;
    let agent = b.agent("Agent 2", "codex").await;
    let si = b.stand_in(&agent, "codex", "codex").await;
    b.doing(agent.id, AgentActivity::Idle).await;
    let (_socket, session) = served(&b).await;
    let refused = session.compose(agent.id, "look", &[("image/png".into(), png(2 * 1024 * 1024))]).await;
    match refused {
        Err(farcooler_client::session::SessionError::Refused { what, .. }) => assert_eq!(what, "unsupported"),
        other => panic!("{other:?}"),
    }
    nothing_typed(&si);
    let root = b.svc.root_dir();
    assert!(files(&crate::pastes::staged::staged_dir_in(root).unwrap(), |_| true).is_empty());
    let pastes = crate::paths::pastes_dir_in(root).unwrap();
    assert!(files(&pastes, crate::pastes::staged::is_composed).is_empty(), "{:?}", files(&pastes, |_| true));
}

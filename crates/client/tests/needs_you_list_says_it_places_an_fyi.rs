//! The Needs You read names `needs_you_fyi` in `Request.accepts` (ov-756), and
//! no other read does. Through `Session::needs_you` against a peer that only
//! reports what it is sent: deleting the line in `Session::start` leaves every
//! daemon test green (they set `accepts` by hand) while no app is ever sent
//! an FYI, so this is the test that can see it.

use farcooler_client::session::Session;
use farcooler_protocol::v1::{ServerHello, WireEnvelope, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};

/// A runner that serves the list and reports `(method, accepts)` of each request.
async fn a_runner(socket: &std::path::Path) -> tokio::sync::mpsc::UnboundedReceiver<(String, Vec<String>)> {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    let (requests, read) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let reply = WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(wire_envelope::Body::ServerHello(ServerHello {
                selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                daemon_version: "a runner".into(),
                max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
                max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
                capabilities: vec![
                    farcooler_protocol::capability::WORKTREES.to_string(),
                    farcooler_protocol::capability::TERMINALS.to_string(),
                    farcooler_protocol::capability::NEEDS_YOU.to_string(),
                ],
                ..Default::default()
            })),
        };
        if writer.write_frame(&reply).await.is_err() {
            return;
        }
        while let Ok(Some(frame)) = reader.read_frame().await {
            if let Some(wire_envelope::Body::Request(request)) = frame.body {
                let _ = requests.send((request.method, request.accepts));
            }
        }
    });
    read
}

#[tokio::test]
async fn only_the_needs_you_read_says_it_places_an_fyi() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let mut requests = a_runner(&socket).await;
    let mut session = Session::connect_local(&socket).await.expect("connect");
    assert!(session.can(farcooler_protocol::capability::NEEDS_YOU));

    let bound = std::time::Duration::from_secs(5);
    for expected in [("needs_you.list", vec!["needs_you_fyi".to_string()]), ("worktree.list", Vec::new())] {
        let sent = tokio::time::timeout(bound, async {
            tokio::select! {
                answered = async { if expected.0 == "needs_you.list" { session.needs_you().await.map(|_| ()) } else { session.worktrees().await.map(|_| ()) } } => panic!("never answered: {answered:?}"),
                first = requests.recv() => first,
            }
        })
        .await
        .expect("the request never reached the peer");
        assert_eq!(sent, Some((expected.0.to_string(), expected.1)));
    }
}

use std::sync::Arc;
use std::time::Duration;

use farcooler_client::session::{FleetEvent, Session};
use farcooler_protocol::v1::{Event, ServerHello, WireEnvelope, event::Payload, wire_envelope::Body};
use farcooler_protocol::{PROTOCOL_VERSION, capability};
use farcooler_transport::codec::{FrameReader, FrameWriter};

#[tokio::test]
async fn a_new_app_keeps_receiving_plan_updates_from_a_runner_with_pages() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("runner.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let workspace = uuid::Uuid::now_v7();
    let peer = tokio::spawn(async move {
        // The app opens one command connection and one event connection.
        let mut connections = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, write) = stream.into_split();
            let mut reader = FrameReader::new(read);
            let mut writer = FrameWriter::new(write);
            assert!(matches!(reader.read_frame().await.unwrap().unwrap().body, Some(Body::ClientHello(_))));
            writer
                .write_frame(&envelope(Body::ServerHello(ServerHello {
                    selected_protocol_version: PROTOCOL_VERSION,
                    capabilities: vec![capability::BOARD_PLAN.into(), "board_pages".into()],
                    ..Default::default()
                })))
                .await
                .unwrap();
            connections.push(writer);
        }
        let events = &mut connections[1];
        // A v1 WireEnvelope (field 14) containing the retired Event arm
        // (field 28), with the old PagesChanged slot field set to "p".
        let retired_event = [0, 0, 0, 10, 0x08, 0x01, 0x72, 0x06, 0xe2, 0x01, 0x03, 0x12, 0x01, b'p'];
        events.write_raw(&retired_event).await.unwrap();
        events
            .write_frame(&envelope(Body::Event(Event {
                payload: Some(Payload::PlanChanged(farcooler_protocol::v1::PlanChanged {
                    workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
                    actor: "manager".into(),
                })),
                ..Default::default()
            })))
            .await
            .unwrap();
    });

    tokio::time::timeout(Duration::from_secs(5), async {
        let app = Session::connect_local(&socket).await.unwrap();
        let (send, mut news) = tokio::sync::mpsc::unbounded_channel();
        let _subscription = app.subscribe(Arc::new(move |event| send.send(event).unwrap())).await.unwrap();
        assert_eq!(news.recv().await, Some(FleetEvent::Plan { workspace }));
        peer.await.unwrap();
    })
    .await
    .expect("the app stopped receiving events from the old runner");
}

fn envelope(body: Body) -> WireEnvelope {
    WireEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(body),
    }
}

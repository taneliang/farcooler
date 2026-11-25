//! The EOF drain must contain an oversized answer too (ov-1162).
use std::{pin::Pin, sync::Arc, task::{Context, Poll}, time::Duration};
use farcooler_protocol::{PROTOCOL_VERSION, MAX_CONTROL_ENVELOPE_BYTES, ids, v1::{self as pb, response, result, wire_envelope}};
use farcooler_transport::{Connection, FrameReader, FrameWriter, Handler, HandshakeConfig, Peer, serve_connection};
use tokio::{io::{AsyncRead, ReadBuf}, sync::Semaphore};

/// Release the slow handler only when the serving loop actually reads EOF.
/// This proves the drain path rather than relying on a sleep to make it likely.
struct ReadEof<R> { inner: R, eof: Arc<Semaphore> }
impl<R: AsyncRead + Unpin> AsyncRead for ReadEof<R> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let read = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(read, Poll::Ready(Ok(()))) && buf.filled().len() == before {
            self.eof.add_permits(1);
        }
        read
    }
}
struct LargeReply { eof: Arc<Semaphore> }
impl Handler for LargeReply {
    fn peer(&self) -> Peer { Peer { client_id: None, scope: pb::Scope::Control } }
    async fn handle(&self, req: pb::Request) -> pb::Response {
        let json = if req.method == "large" {
            let _permit = self.eof.acquire().await.unwrap();
            "x".repeat(MAX_CONTROL_ENVELOPE_BYTES)
        } else { "ok".into() };
        pb::Response { request_id: req.request_id, outcome: Some(response::Outcome::Result(pb::Result {
            value: Some(result::Value::WorkflowReply(pb::WorkflowReply { schema: 1, json })),
        })) }
    }
}

#[tokio::test]
async fn an_oversized_answer_in_the_eof_drain_preserves_the_following_answer() {
    let (server, client) = tokio::io::duplex(8192);
    let eof = Arc::new(Semaphore::new(0));
    let serving = tokio::spawn(async move {
        let (r, w) = tokio::io::split(server);
        let mut conn = Connection::new(ReadEof { inner: r, eof: eof.clone() }, w);
        serve_connection(&mut conn, &HandshakeConfig { daemon_version: "fixture".into() }, &LargeReply { eof }).await
    });
    let (r, w) = tokio::io::split(client);
    let mut reader = FrameReader::new(r);
    let mut writer = FrameWriter::new(w);
    writer.write_frame(&pb::WireEnvelope { protocol_version: PROTOCOL_VERSION, message_id: ids::new_id(),
        body: Some(wire_envelope::Body::ClientHello(pb::ClientHello {
            supported_protocol_versions: vec![PROTOCOL_VERSION], client_name: "eof".into(), client_version: "1".into(),
            writer_client_instance: Default::default(),
        })) }).await.unwrap();
    reader.read_frame().await.unwrap().unwrap();
    let mut requests = Vec::new();
    for method in ["large", "small"] {
        let request = farcooler_transport::request(method);
        requests.push(request.request_id.clone());
        writer.write_frame(&pb::WireEnvelope { protocol_version: PROTOCOL_VERSION, message_id: ids::new_id(),
            body: Some(wire_envelope::Body::Request(request)) }).await.unwrap();
    }
    writer.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        for (index, id) in requests.iter().enumerate() {
            let envelope = reader.read_frame().await.unwrap().expect("the drain lost an answer");
            let Some(wire_envelope::Body::Response(answer)) = envelope.body else { panic!("not a response") };
            assert_eq!(&answer.request_id, id);
            if index == 0 {
                let Some(response::Outcome::Error(error)) = answer.outcome else { panic!("not an error") };
                assert_eq!(error.what, "response_too_large");
                assert!(!error.retryable);
            } else {
                let Some(response::Outcome::Result(pb::Result { value: Some(result::Value::WorkflowReply(reply)) })) = answer.outcome else { panic!("not a result") };
                assert_eq!(reply.json, "ok");
            }
        }
        assert!(reader.read_frame().await.unwrap().is_none());
    }).await.expect("drain did not finish");
    // EOF is still reported as EOF; only the oversized outgoing answer is contained.
    assert!(matches!(serving.await.unwrap(), Err(farcooler_transport::ConnectionError::Closed)));
}

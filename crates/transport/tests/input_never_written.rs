//! Input the connection ended before it wrote (ov-250).
//!
//! The phones keep typed input that provably did not arrive and drop what
//! may have. An urgent call fails as `NotWritten` only when the writer had
//! not begun its frame: one written, or begun and cut off partway, fails as
//! the plain connection error, because the runner may have read it.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use farcooler_protocol::PROTOCOL_VERSION;
use farcooler_protocol::v1::{self, WireEnvelope, wire_envelope};
use farcooler_transport::{CallOptions, Client, ClientError, FrameReader, FrameWriter, request};
use tokio::io::{AsyncWrite, WriteHalf};

type Duplex = tokio::io::DuplexStream;

/// The client's write half, which takes `budget` more bytes and then fails
/// with a broken pipe. Effectively unlimited until a test narrows it.
struct Cutoff {
    inner: WriteHalf<Duplex>,
    budget: Arc<AtomicUsize>,
}

impl AsyncWrite for Cutoff {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let left = self.budget.load(Ordering::SeqCst);
        if left == 0 {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let take = buf.len().min(left);
        let polled = Pin::new(&mut self.inner).poll_write(cx, &buf[..take]);
        if let Poll::Ready(Ok(n)) = &polled {
            self.budget.fetch_sub(*n, Ordering::SeqCst);
        }
        polled
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// A runner that says hello and then, when `read_one` is set, reads one
/// request and hangs up, answering nothing.
async fn connected(
    read_one: bool,
) -> (Client<tokio::io::ReadHalf<Duplex>, Cutoff>, Arc<AtomicUsize>) {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut reader = FrameReader::new(r);
        let mut writer = FrameWriter::new(w);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = WireEnvelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(wire_envelope::Body::ServerHello(v1::ServerHello {
                selected_protocol_version: PROTOCOL_VERSION,
                ..Default::default()
            })),
        };
        if writer.write_frame(&hello).await.is_err() {
            return;
        }
        if read_one {
            let _ = reader.read_frame().await;
        } else {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    let (r, w) = tokio::io::split(client_io);
    let budget = Arc::new(AtomicUsize::new(usize::MAX));
    let client = Client::over(r, Cutoff { inner: w, budget: budget.clone() }, "test", "0")
        .await
        .expect("handshake");
    (client, budget)
}

fn input() -> CallOptions {
    CallOptions { deadline: None, urgent: true }
}

async fn finished(answer: farcooler_transport::Answer) -> Result<v1::Result, ClientError> {
    tokio::time::timeout(Duration::from_secs(30), answer.answer()).await.expect("never answered")
}

/// Queued, and the connection ended before the writer took it up.
#[tokio::test]
async fn input_queued_when_the_connection_ends_was_never_written() {
    let (client, _) = connected(false).await;
    // On this single-threaded runtime the writer has not run yet.
    let answer = client.send(request("key"), input()).expect("queued");
    drop(client);
    let outcome = finished(answer).await;
    assert!(
        matches!(&outcome, Err(ClientError::NotWritten(cause)) if matches!(**cause, ClientError::Closed)),
        "got {outcome:?}"
    );
}

/// Sent after the connection already ended: nothing was queued at all.
#[tokio::test]
async fn input_sent_after_the_connection_ended_was_never_written() {
    // Fail the connection from the writer's side, then send again.
    let (client, budget) = connected(true).await;
    budget.store(0, Ordering::SeqCst);
    let doomed = client.send(request("x"), input()).expect("queued");
    let _ = finished(doomed).await;
    let late = client.send(request("y"), input());
    assert!(matches!(late, Err(ClientError::NotWritten(_))), "urgent: {:?}", late.err());
    let ordinary = client.send(request("z"), CallOptions::default());
    assert!(
        matches!(ordinary, Err(ref e) if !matches!(e, ClientError::NotWritten(_))),
        "an ordinary call's error is as it always was: {:?}",
        ordinary.err()
    );
}

/// Written whole, then the runner hung up unanswered: maybe delivered.
#[tokio::test]
async fn input_written_whole_is_never_reported_as_unwritten() {
    let (client, _) = connected(true).await;
    let outcome = finished(client.send(request("key"), input()).expect("queued")).await;
    assert!(
        matches!(outcome, Err(ClientError::Closed) | Err(ClientError::Codec(_))),
        "got {outcome:?}"
    );
}

/// The write broke partway through the frame: some of it may have arrived.
#[tokio::test]
async fn input_written_partly_is_never_reported_as_unwritten() {
    let (client, budget) = connected(false).await;
    budget.store(5, Ordering::SeqCst);
    let outcome = finished(client.send(request("key"), input()).expect("queued")).await;
    assert!(matches!(outcome, Err(ClientError::Codec(_))), "got {outcome:?}");
}

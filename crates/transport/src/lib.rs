//! Transport adapters over the wire framing in `overnight-protocol`.
//!
//! Rule 1: the daemon opens no network listener. The only entry points are a
//! mode-0600 Unix socket (`listener.rs`) and a process launched by sshd
//! speaking the identical framing over stdio (`stdio.rs`). Both sit on the
//! same `codec.rs` and `connection.rs`, so the wire behavior cannot drift
//! between them.

pub mod client;
pub mod codec;
pub mod connection;
pub mod listener;
pub mod stdio;

pub use client::{Client, ClientError, request};
pub use codec::{CodecError, FrameReader, FrameWriter};
pub use connection::{Connection, ConnectionError, HandshakeConfig, TOO_SLOW_DISCONNECT, serve_connection};
pub use listener::UnixListenerServer;
pub use stdio::serve_stdio;

use overnight_protocol::v1::{Request, Response};

/// Implemented by the daemon crate. Transport owns envelope mechanics
/// (framing, the Hello handshake, request_id correlation, rule-4
/// backpressure); this trait is the one seam where a `Request` reaches
/// business logic and comes back as a `Response`.
///
/// Declared with an explicit `impl Future<..> + Send` return (RPITIT) rather
/// than plain `async fn` so the future is guaranteed `Send`: connections are
/// dispatched from tasks spawned onto a multi-threaded runtime, and a
/// non-Send future would fail to spawn.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send;

    /// Events to push to this connection, if the handler produces any.
    ///
    /// Defaulted to none so a handler that only answers requests — a test, a
    /// one-shot stdio session — needs no change. Returning a receiver opts a
    /// connection into the push stream instead of leaving it to poll.
    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<overnight_protocol::v1::Event>> {
        None
    }
}

//! Transport adapters over the wire framing in `farcooler-protocol`.
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
pub use listener::{SessionPreamble, UnixListenerServer};
pub use stdio::serve_stdio;

use farcooler_protocol::v1::{Request, Response, Scope};

/// Who is on the other end, for the whole life of one connection.
///
/// A connection is a principal, not a series of unrelated requests: sshd proved
/// who this is once, at authentication, and nothing later in the conversation
/// can add to it or take from it. So this is answered once by the handler that
/// was built for this connection rather than carried on each `Request`, where a
/// client could vary it between calls and the daemon would have to decide which
/// answer to believe.
///
/// Scope and identity travel together because they are one fact with two
/// halves, decided at one moment by one file on this runner. Keeping the scope
/// somewhere else — it used to be on the handshake config, with the dispatcher
/// holding its own copy — is how a session came to be advertised `read` and
/// permitted everything.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Which enrolled device this is, as this runner's own `authorized_keys`
    /// says rather than as the connection claims.
    ///
    /// `None` is a caller that named no device — every local socket client, and
    /// every key enrolled before forced commands carried an id. It is not an
    /// unknown device: it is a caller that revocation by device id can never
    /// match, which is exactly right for the owning user's own Mac app.
    pub client_id: Option<String>,
    /// What this connection may do. The one copy: `ServerHello` advertises it
    /// and the dispatcher enforces it, both from here.
    pub scope: Scope,
}

/// Implemented by the daemon crate. Transport owns envelope mechanics
/// (framing, the Hello handshake, request_id correlation, rule-4
/// backpressure); this trait is the one seam where a `Request` reaches
/// business logic and comes back as a `Response`.
///
/// One implementer per connection, which is what makes `peer` and `closed`
/// meaningful: the handler is where a connection's identity lives, and where
/// something outside it can reach in and end it.
///
/// Declared with an explicit `impl Future<..> + Send` return (RPITIT) rather
/// than plain `async fn` so the future is guaranteed `Send`: connections are
/// dispatched from tasks spawned onto a multi-threaded runtime, and a
/// non-Send future would fail to spawn.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, req: Request) -> impl std::future::Future<Output = Response> + Send;

    /// Whose connection this is. Read once, before the handshake it answers.
    ///
    /// Not defaulted. A default would be a guess about identity and privilege
    /// made on behalf of whoever forgot to write one, and the two guesses
    /// available — nobody, or the owning user — are respectively useless and
    /// dangerous.
    fn peer(&self) -> Peer;

    /// Events to push to this connection, if the handler produces any.
    ///
    /// Defaulted to none so a handler that only answers requests — a test, a
    /// one-shot stdio session — needs no change. Returning a receiver opts a
    /// connection into the push stream instead of leaving it to poll.
    fn events(&self) -> Option<tokio::sync::broadcast::Receiver<farcooler_protocol::v1::Event>> {
        None
    }

    /// Resolves when this connection must stop being served.
    ///
    /// The seam revocation needs and could not otherwise have: removing a
    /// device's key stops the NEXT authentication and does nothing at all to
    /// the session it is holding right now, so something above this crate has
    /// to be able to end a connection it does not own. Transport keeps owning
    /// the socket; this is the only thing it is told.
    ///
    /// Defaulted to never, because a handler that has nothing which could
    /// revoke it — a test, a one-shot session — should not have to say so.
    fn closed(&self) -> impl std::future::Future<Output = ()> + Send {
        std::future::pending()
    }
}

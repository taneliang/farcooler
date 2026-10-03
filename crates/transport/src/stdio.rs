//! Rule 1: the same framing must work unchanged over stdio so `farcooler
//! transport stdio` can be launched by sshd as the other entry point besides
//! the Unix socket. This reuses `connection::serve_connection`, the exact
//! codec-driven path the socket listener uses; nothing here re-implements
//! framing or the handshake.

use tokio::io::{stdin, stdout};

use crate::Handler;
use crate::connection::{Connection, ConnectionError, HandshakeConfig, refuse, serve_connection};

/// Runs one handshake-then-dispatch session over stdin/stdout until the
/// connection closes.
pub async fn serve_stdio<H>(cfg: HandshakeConfig, handler: H) -> Result<(), ConnectionError>
where
    H: Handler,
{
    let mut conn = Connection::new(stdin(), stdout());
    serve_connection(&mut conn, &cfg, &handler).await
}

/// Answer the session on stdin/stdout with `err` and nothing else. The stdio
/// side of `UnixListenerServer::refuse_every`, for a `--stdio` process that
/// found no daemon to relay to and cannot serve one itself.
pub async fn refuse_stdio(err: farcooler_core::DomainError) -> Result<(), ConnectionError> {
    refuse(stdin(), stdout(), err).await
}

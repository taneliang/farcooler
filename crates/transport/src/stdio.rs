//! Rule 1: the same framing must work unchanged over stdio so `overnight
//! transport stdio` can be launched by sshd as the other entry point besides
//! the Unix socket. This reuses `connection::serve_connection`, the exact
//! codec-driven path the socket listener uses; nothing here re-implements
//! framing or the handshake.

use tokio::io::{stdin, stdout};

use crate::Handler;
use crate::connection::{Connection, ConnectionError, HandshakeConfig, serve_connection};

/// Runs one handshake-then-dispatch session over stdin/stdout until the
/// connection closes.
pub async fn serve_stdio<H>(cfg: HandshakeConfig, handler: H) -> Result<(), ConnectionError>
where
    H: Handler,
{
    let mut conn = Connection::new(stdin(), stdout());
    serve_connection(&mut conn, &cfg, &handler).await
}

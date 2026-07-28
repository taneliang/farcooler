//! The client half of the wire: connect, handshake, call.
//!
//! Written once and shared by the CLI, the tests, and eventually the SSH
//! transport, because a second implementation is a second set of bugs about
//! version negotiation and request correlation.
//!
//! Correlation is by `request_id` rather than by arrival order. The daemon is
//! free to answer out of order and to interleave events between responses, so a
//! client that assumed the next frame was its answer would eventually read an
//! event as a reply — rarely, and under load, which is the worst way to find
//! out.

use std::path::Path;

use overnight_protocol::v1::{
    ClientHello, Event, Request, Response, ServerHello, WireEnvelope, response, wire_envelope,
};
use overnight_protocol::{PROTOCOL_VERSION, ids};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixStream;

use crate::codec::{CodecError, FrameReader, FrameWriter};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error("could not reach the daemon: {0}")]
    Connect(#[source] std::io::Error),
    #[error("the daemon closed the connection")]
    Closed,
    #[error("the daemon did not answer with a ServerHello")]
    NoHello,
    #[error("the daemon speaks protocol {daemon}, this client speaks {client}")]
    VersionMismatch { daemon: u32, client: u32 },
    #[error("{message}")]
    Daemon { code: i32, retryable: bool, message: String },
    #[error("the daemon returned an empty result")]
    EmptyResult,
}

pub struct Client<R, W> {
    reader: FrameReader<R>,
    writer: FrameWriter<W>,
    server: ServerHello,
    /// Events that arrived while waiting for a response. Kept rather than
    /// dropped: they are the daemon telling us something changed, and a client
    /// that discards them silently goes stale.
    pending_events: Vec<Event>,
}

impl Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf> {
    /// Connect to a daemon's Unix socket and complete the handshake.
    pub async fn connect(
        socket: impl AsRef<Path>,
        client_name: &str,
        client_version: &str,
    ) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket.as_ref()).await.map_err(ClientError::Connect)?;
        let (read, write) = stream.into_split();
        Self::over(read, write, client_name, client_version).await
    }
}

impl<R, W> Client<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// Handshake over any pair of streams, so the same client works over stdio
    /// through sshd as it does over a local socket.
    pub async fn over(
        read: R,
        write: W,
        client_name: &str,
        client_version: &str,
    ) -> Result<Self, ClientError> {
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);

        writer
            .write_frame(&WireEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_id: ids::new_id(),
                body: Some(wire_envelope::Body::ClientHello(ClientHello {
                    supported_protocol_versions: vec![PROTOCOL_VERSION],
                    client_name: client_name.to_string(),
                    client_version: client_version.to_string(),
                })),
            })
            .await?;

        let envelope = reader.read_frame().await?.ok_or(ClientError::Closed)?;
        let Some(wire_envelope::Body::ServerHello(server)) = envelope.body else {
            return Err(ClientError::NoHello);
        };
        if server.selected_protocol_version != PROTOCOL_VERSION {
            return Err(ClientError::VersionMismatch {
                daemon: server.selected_protocol_version,
                client: PROTOCOL_VERSION,
            });
        }

        Ok(Self { reader, writer, server, pending_events: Vec::new() })
    }

    pub fn server_hello(&self) -> &ServerHello {
        &self.server
    }

    /// Events that arrived while waiting for responses.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending_events)
    }

    /// Send a request and wait for the response that matches it.
    pub async fn call(
        &mut self,
        request: Request,
    ) -> Result<overnight_protocol::v1::Result, ClientError> {
        let request_id = request.request_id.clone();
        self.writer
            .write_frame(&WireEnvelope {
                protocol_version: PROTOCOL_VERSION,
                message_id: ids::new_id(),
                body: Some(wire_envelope::Body::Request(request)),
            })
            .await?;

        loop {
            let envelope = self.reader.read_frame().await?.ok_or(ClientError::Closed)?;
            match envelope.body {
                Some(wire_envelope::Body::Response(r)) if r.request_id == request_id => {
                    return unwrap_response(r);
                }
                // Another request's answer. Only possible once this client
                // pipelines, but dropping it silently then would be a bug that
                // only shows up under concurrency.
                Some(wire_envelope::Body::Response(_)) => continue,
                Some(wire_envelope::Body::Event(e)) => self.pending_events.push(e),
                _ => continue,
            }
        }
    }
}

fn unwrap_response(r: Response) -> Result<overnight_protocol::v1::Result, ClientError> {
    match r.outcome {
        Some(response::Outcome::Result(value)) => Ok(value),
        Some(response::Outcome::Error(e)) => {
            Err(ClientError::Daemon { code: e.code, retryable: e.retryable, message: e.message })
        }
        None => Err(ClientError::EmptyResult),
    }
}

/// Build a request. `method` decides everything else about it.
pub fn request(method: &str) -> Request {
    Request {
        request_id: ids::to_bytes(uuid::Uuid::now_v7()),
        method: method.to_string(),
        target_resource_id: None,
        expected_resource_version: None,
        expected_lease_generation: None,
        idempotency_key: None,
        payload: Some(overnight_protocol::v1::request::Payload::Empty(
            overnight_protocol::v1::Empty {},
        )),
    }
}

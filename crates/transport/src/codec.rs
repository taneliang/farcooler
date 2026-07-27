//! Async framed codec shared by every transport adapter (rule 1): the same
//! length-delimited `WireEnvelope` stream runs over a Unix socket and over
//! stdio without duplicating parsing logic in either place.

use bytes::BytesMut;
use overnight_protocol::framing;
use overnight_protocol::v1::WireEnvelope;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bytes requested per underlying read call. An I/O chunk size, not a
/// protocol limit; `MAX_CONTROL_ENVELOPE_BYTES` is what actually bounds a
/// frame, enforced inside `framing::decode` before any allocation.
const READ_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error(transparent)]
    Framing(#[from] framing::FramingError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("peer closed mid-frame")]
    Truncated,
}

/// Encode one envelope into a length-delimited frame.
pub fn encode_frame(envelope: &WireEnvelope) -> Result<Vec<u8>, CodecError> {
    Ok(framing::encode(envelope)?)
}

/// Buffered, incremental reader. `read_frame` may need several underlying
/// reads to complete one frame, and one underlying read may complete several
/// frames; both are handled without re-reading or copying already-buffered
/// bytes, since `framing::decode` only ever consumes what it fully parses.
pub struct FrameReader<R> {
    inner: R,
    buf: BytesMut,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self { inner, buf: BytesMut::with_capacity(READ_CHUNK_BYTES) }
    }

    /// `Ok(None)` means the peer closed cleanly between frames. A close in
    /// the middle of a frame is `Err(Truncated)`: rule 3 requires a
    /// truncated frame to never reach dispatch, so it must not look like an
    /// ordinary end of stream to the caller.
    pub async fn read_frame(&mut self) -> Result<Option<WireEnvelope>, CodecError> {
        loop {
            if let Some(envelope) = framing::decode(&mut self.buf)? {
                return Ok(Some(envelope));
            }

            // `decode` already validated any declared length against the cap
            // before this point, so reserving here never over-allocates on a
            // hostile prefix; it just gives the next read somewhere to land.
            self.buf.reserve(READ_CHUNK_BYTES);
            let n = self.inner.read_buf(&mut self.buf).await?;
            if n == 0 {
                return if self.buf.is_empty() { Ok(None) } else { Err(CodecError::Truncated) };
            }
        }
    }
}

/// Thin write side, kept separate from `FrameReader` so a connection can hand
/// the writer half to a dedicated task (see `connection.rs`) while the reader
/// stays with the caller.
pub struct FrameWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub async fn write_frame(&mut self, envelope: &WireEnvelope) -> Result<(), CodecError> {
        let bytes = encode_frame(envelope)?;
        self.write_raw(&bytes).await
    }

    /// Writes an already-encoded frame. `Connection`'s writer task measures
    /// the encoded length up front for the rule-4 backpressure accounting,
    /// so this avoids encoding the same envelope twice.
    pub async fn write_raw(&mut self, bytes: &[u8]) -> Result<(), CodecError> {
        self.inner.write_all(bytes).await?;
        self.inner.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overnight_protocol::v1::{ClientHello, wire_envelope};

    fn hello() -> WireEnvelope {
        WireEnvelope {
            protocol_version: 1,
            message_id: overnight_protocol::ids::new_id(),
            body: Some(wire_envelope::Body::ClientHello(ClientHello {
                supported_protocol_versions: vec![1],
                client_name: "test".into(),
                client_version: "0.1.0".into(),
            })),
        }
    }

    /// Proves transport invariance (rule 1): the codec doesn't know or care
    /// that this is a duplex pipe rather than a Unix socket or stdio.
    #[tokio::test]
    async fn round_trip_over_duplex() {
        let (client, server) = tokio::io::duplex(64);
        let (_cr, cw) = tokio::io::split(client);
        let (sr, _sw) = tokio::io::split(server);

        let mut writer = FrameWriter::new(cw);
        writer.write_frame(&hello()).await.unwrap();

        let mut reader = FrameReader::new(sr);
        let got = reader.read_frame().await.unwrap().unwrap();
        assert_eq!(got, hello());
    }

    #[tokio::test]
    async fn frame_split_across_multiple_reads_still_parses() {
        let (client, server) = tokio::io::duplex(4096);
        let (_cr, mut cw) = tokio::io::split(client);
        let (sr, _sw) = tokio::io::split(server);

        let bytes = encode_frame(&hello()).unwrap();
        let reader_task = tokio::spawn(async move {
            let mut reader = FrameReader::new(sr);
            reader.read_frame().await
        });

        cw.write_all(&bytes[..3]).await.unwrap();
        cw.flush().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cw.write_all(&bytes[3..]).await.unwrap();
        cw.flush().await.unwrap();

        let got = reader_task.await.unwrap().unwrap().unwrap();
        assert_eq!(got, hello());
    }

    #[tokio::test]
    async fn eof_mid_frame_is_truncated_not_dispatched() {
        // Deliberately unsplit: `tokio::io::split` halves share the
        // underlying stream, so dropping only one half leaves it open (the
        // peer never sees EOF). Dropping the whole `DuplexStream` does.
        let (mut client, server) = tokio::io::duplex(4096);
        let (sr, _sw) = tokio::io::split(server);

        let bytes = encode_frame(&hello()).unwrap();
        client.write_all(&bytes[..bytes.len() - 1]).await.unwrap();
        client.flush().await.unwrap();
        drop(client); // EOF with a partial frame still sitting in the buffer

        let mut reader = FrameReader::new(sr);
        let err = reader.read_frame().await.unwrap_err();
        assert!(matches!(err, CodecError::Truncated));
    }

    #[tokio::test]
    async fn clean_eof_between_frames_is_none() {
        let (client, server) = tokio::io::duplex(4096);
        drop(client); // whole stream gone, not just one split half
        let (sr, _sw) = tokio::io::split(server);

        let mut reader = FrameReader::new(sr);
        assert_eq!(reader.read_frame().await.unwrap(), None);
    }

    #[tokio::test]
    async fn oversized_prefix_rejected_without_allocating() {
        use bytes::BufMut;

        let (client, server) = tokio::io::duplex(4096);
        let (_cr, mut cw) = tokio::io::split(client);
        let (sr, _sw) = tokio::io::split(server);

        let mut raw = BytesMut::new();
        raw.put_u32(u32::MAX);
        raw.put_slice(b"junk");
        cw.write_all(&raw).await.unwrap();
        cw.flush().await.unwrap();

        let mut reader = FrameReader::new(sr);
        let err = reader.read_frame().await.unwrap_err();
        assert!(matches!(err, CodecError::Framing(framing::FramingError::Oversized(_, _))));
    }
}

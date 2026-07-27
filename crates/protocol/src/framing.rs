//! Length-delimited framing.
//!
//! `envelope_length:u32` network byte order, then one serialized `WireEnvelope`.
//! Total envelope length is validated BEFORE allocation. Invalid, oversized, or
//! truncated frames are a protocol error and never reach method dispatch.

use bytes::{Buf, BufMut, BytesMut};
use prost::Message;

use crate::{MAX_CONTROL_ENVELOPE_BYTES, v1::WireEnvelope};

pub const LENGTH_PREFIX_BYTES: usize = 4;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FramingError {
    #[error("envelope length {0} exceeds the {1} byte cap")]
    Oversized(usize, usize),
    #[error("declared envelope length is zero")]
    ZeroLength,
    #[error("malformed protobuf envelope")]
    Malformed,
}

/// Encode one envelope into a length-delimited frame.
pub fn encode(envelope: &WireEnvelope) -> Result<Vec<u8>, FramingError> {
    let len = envelope.encoded_len();
    if len > MAX_CONTROL_ENVELOPE_BYTES {
        return Err(FramingError::Oversized(len, MAX_CONTROL_ENVELOPE_BYTES));
    }
    let mut out = Vec::with_capacity(LENGTH_PREFIX_BYTES + len);
    out.put_u32(len as u32);
    envelope.encode(&mut out).map_err(|_| FramingError::Malformed)?;
    Ok(out)
}

/// Try to decode one frame from the front of `buf`.
///
/// Returns `Ok(None)` when more bytes are needed. The length is checked against
/// the cap before any allocation, so a hostile prefix cannot make us reserve a
/// gigabyte.
pub fn decode(buf: &mut BytesMut) -> Result<Option<WireEnvelope>, FramingError> {
    if buf.len() < LENGTH_PREFIX_BYTES {
        return Ok(None);
    }

    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if len == 0 {
        return Err(FramingError::ZeroLength);
    }
    if len > MAX_CONTROL_ENVELOPE_BYTES {
        return Err(FramingError::Oversized(len, MAX_CONTROL_ENVELOPE_BYTES));
    }

    if buf.len() < LENGTH_PREFIX_BYTES + len {
        // Truncated so far. Reserve exactly what the validated length needs.
        buf.reserve(LENGTH_PREFIX_BYTES + len - buf.len());
        return Ok(None);
    }

    buf.advance(LENGTH_PREFIX_BYTES);
    let frame = buf.split_to(len);
    WireEnvelope::decode(frame).map(Some).map_err(|_| FramingError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::{ClientHello, wire_envelope};

    fn hello() -> WireEnvelope {
        WireEnvelope {
            protocol_version: 1,
            message_id: vec![7u8; 16].into(),
            body: Some(wire_envelope::Body::ClientHello(ClientHello {
                supported_protocol_versions: vec![1],
                client_name: "test".into(),
                client_version: "0.1.0".into(),
            })),
        }
    }

    #[test]
    fn round_trip() {
        let env = hello();
        let bytes = encode(&env).unwrap();
        let mut buf = BytesMut::from(&bytes[..]);
        let decoded = decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, env);
        assert!(buf.is_empty(), "frame fully consumed");
    }

    #[test]
    fn partial_frame_yields_none_and_keeps_bytes() {
        let bytes = encode(&hello()).unwrap();
        let mut buf = BytesMut::from(&bytes[..bytes.len() - 1]);
        assert_eq!(decode(&mut buf).unwrap(), None);
        // Nothing consumed: the caller can append more and retry.
        assert_eq!(buf.len(), bytes.len() - 1);
    }

    #[test]
    fn two_frames_in_one_buffer() {
        let mut bytes = encode(&hello()).unwrap();
        bytes.extend(encode(&hello()).unwrap());
        let mut buf = BytesMut::from(&bytes[..]);
        assert!(decode(&mut buf).unwrap().is_some());
        assert!(decode(&mut buf).unwrap().is_some());
        assert_eq!(decode(&mut buf).unwrap(), None);
    }

    #[test]
    fn oversized_prefix_rejected_without_allocating() {
        let mut buf = BytesMut::new();
        buf.put_u32(u32::MAX);
        buf.put_slice(b"junk");
        assert_eq!(
            decode(&mut buf),
            Err(FramingError::Oversized(u32::MAX as usize, MAX_CONTROL_ENVELOPE_BYTES))
        );
    }

    #[test]
    fn zero_length_rejected() {
        let mut buf = BytesMut::new();
        buf.put_u32(0);
        assert_eq!(decode(&mut buf), Err(FramingError::ZeroLength));
    }

    #[test]
    fn malformed_body_rejected() {
        let mut buf = BytesMut::new();
        buf.put_u32(3);
        buf.put_slice(&[0xff, 0xff, 0xff]);
        assert_eq!(decode(&mut buf), Err(FramingError::Malformed));
    }
}

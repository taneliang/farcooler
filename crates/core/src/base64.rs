//! Base64, written out rather than pulled in.
//!
//! The same trade `host_install::sha256_hex` makes: one dependency avoided for
//! forty lines, in a workspace that currently has none. It exists because an
//! image travels to the agent as an ACP content block, which is base64, while
//! the protocol carries it as raw bytes — so one side encodes and the other
//! decodes, and both are here so they cannot disagree.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        // The tail is padded rather than truncated: a decoder is entitled to
        // assume a multiple of four, and every real one does.
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// Decode, refusing anything that is not base64 rather than guessing.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        // Padding and whitespace end or interrupt the run; anything else that
        // is not in the alphabet is a corrupt payload, and a silently short
        // image is worse than none.
        if byte == b'=' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        let value = ALPHABET.iter().position(|c| *c == byte)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_matches_the_rfc_4648_vectors() {
        // The published ones, so this agrees with every other implementation
        // rather than only with itself.
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn it_round_trips_bytes_a_png_would_contain() {
        // Every byte value, because an image is not text and the high bytes are
        // exactly where an off-by-one in the shifting would hide.
        let data: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }

    #[test]
    fn it_refuses_a_corrupt_payload_rather_than_truncating_it() {
        assert_eq!(decode("Zm9v"), Some(b"foo".to_vec()));
        assert_eq!(decode("Zm9v!!"), None);
    }
}

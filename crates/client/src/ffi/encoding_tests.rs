//! The client's base64 and hex edges, moved out of `ffi.rs` to keep it inside its size ceiling (ov-274).

use super::*;


#[test]
fn base64_survives_bytes_that_are_not_text() {
    // Kept after the local encoder was deleted in favour of
    // `farcooler_core::base64`: the shared module proves it matches the
    // RFC, and this proves the thing THIS crate depends on — a screen is
    // escape sequences and high bytes, not a string.
    assert_eq!(farcooler_core::base64::encode(&[0x1b, 0x5b, 0x33, 0x31, 0x6d]), "G1szMW0=");
    assert_eq!(farcooler_core::base64::encode(&[0xff, 0x00, 0xfe]), "/wD+");
}

#[test]
fn hex_round_trips_and_rejects_what_is_not_hex() {
    assert_eq!(decode_hex("00ff1b"), Some(vec![0x00, 0xff, 0x1b]));
    assert_eq!(decode_hex(""), Some(vec![]));
    assert_eq!(decode_hex("abc"), None, "odd length is not a byte string");
    assert_eq!(decode_hex("zz"), None);
}

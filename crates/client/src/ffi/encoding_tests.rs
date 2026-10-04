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

/// The bridge's two adapter projections have to be one round trip.
///
/// They were not. `wire_adapter` has read `backend` since the field
/// existed, and `adapter_json` never wrote it — so a phone's only view of
/// an adapter had no backend in it, could send none back, and the daemon
/// read the absent field as ACP. That is a Test button reporting a working
/// adapter for a protocol nothing spoke to it, and an upsert of anything
/// else on the form quietly rewriting the table to ACP on the way past.
///
/// Written as a round trip rather than as an assertion about one key,
/// because the failure was the two halves disagreeing and only a trip
/// through both can see that.
#[test]
fn an_adapter_keeps_its_backend_through_the_bridge_and_back() {
    let saved = farcooler_protocol::v1::Adapter {
        preset: "codex".to_string(),
        program: "codex".to_string(),
        backend: farcooler_protocol::v1::AdapterBackend::Native as i32,
        ..Default::default()
    };

    let shown = adapter_json(std::slice::from_ref(&saved))["adapters"][0].clone();
    assert_eq!(shown["backend"], json!("native"));

    // Back through the door a client sends on, with the object it was
    // handed — which is exactly what an editor holds when Test is pressed.
    let strings = |_: &str| Vec::new();
    let sent = wire_adapter("codex", "codex", &strings, &shown);
    assert_eq!(sent.backend, saved.backend);
}

/// And an object with no `backend` at all still means ACP.
///
/// A phone built before this key exists sends the dictionary it always
/// sent, and must keep the behavior it had rather than failing to decode.
#[test]
fn an_adapter_json_with_no_backend_is_acp() {
    let strings = |_: &str| Vec::new();
    let sent = wire_adapter("cursor", "cursor-agent", &strings, &json!({}));
    assert_eq!(
        farcooler_core::activity::AdapterBackend::from_proto(sent.backend),
        farcooler_core::activity::AdapterBackend::Acp
    );
}

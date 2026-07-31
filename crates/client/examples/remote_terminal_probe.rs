//! Prove a phone can read a screen and type into it, over ssh.
//!
//! The iOS app reaches a host exactly this way — the same FFI, the same JSON,
//! the same base64 — so exercising it here covers everything between the Swift
//! call and tmux. What it deliberately does not cover is the Swift itself.
//!
//!   remote_terminal_probe <user@host:port> <path/to/private_key> <terminal-uuid>

use std::ffi::CString;

// The same entry points the XCFramework exports to Swift, reached through the
// crate rather than through the C ABI: a Rust example links the rlib, and the
// `no_mangle` symbols are only in the staticlib the framework is built from.
// Same functions, same bodies — this is the layer under Swift, not a stand-in
// for it.
use overnight_client::ffi::{
    overnight_client_call, overnight_client_connect, overnight_client_new, overnight_client_poll,
};

fn poll_for(handle: *mut std::ffi::c_void, ticket: u64) -> serde_json::Value {
    let mut held: Vec<serde_json::Value> = Vec::new();
    for _ in 0..900 {
        let raw = unsafe { overnight_client_poll(handle) };
        if !raw.is_null() {
            let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if v.get("ticket").and_then(|t| t.as_u64()) == Some(ticket) {
                    return v;
                }
                // Results come back once each, so anything for another ticket
                // has to be kept rather than dropped on the floor.
                held.push(v);
            }
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("no answer for ticket {ticket}; saw {held:?}");
}

fn call(handle: *mut std::ffi::c_void, method: &str, args: serde_json::Value) -> serde_json::Value {
    let m = CString::new(method).unwrap();
    let a = CString::new(args.to_string()).unwrap();
    let ticket = unsafe { overnight_client_call(handle, m.as_ptr(), a.as_ptr()) };
    let reply = poll_for(handle, ticket);
    assert_eq!(reply["ok"], serde_json::json!(true), "{method} failed: {reply}");
    reply["result"].clone()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (dest, key_path, terminal) = (&args[1], &args[2], &args[3]);
    let (user, rest) = dest.split_once('@').expect("user@host:port");
    let (host, port) = rest.rsplit_once(':').unwrap_or((rest, "22"));
    let key = std::fs::read_to_string(key_path).expect("private key");

    let handle = unsafe { overnight_client_new() };
    let config = serde_json::json!({
        "host": host, "port": port.parse::<u16>().unwrap(), "user": user,
        "private_key": key,
        // The phone pins a fingerprint after showing it to a human; this probe
        // is not testing that half.
        "host_fingerprint": "accept-any",
    });
    let c = CString::new(config.to_string()).unwrap();
    let ticket = unsafe { overnight_client_connect(handle, c.as_ptr()) };
    let started = poll_for(handle, ticket);
    assert_eq!(started["ok"], serde_json::json!(true), "connect failed: {started}");
    println!("connected over ssh");

    let fleet = call(handle, "fleet", serde_json::json!({}));
    let count = fleet["workspaces"].as_array().map(|w| w.len()).unwrap_or(0);
    println!("fleet: {count} workspaces");

    let screen = call(handle, "terminal.screen", serde_json::json!({ "terminal": terminal }));
    let decoded = decode_base64(screen["contents"].as_str().expect("contents")).expect("base64");
    println!(
        "screen: {}x{} cursor {},{} - {} bytes, {} escape sequences",
        screen["columns"], screen["rows"], screen["cursorColumn"], screen["cursorRow"],
        decoded.len(),
        decoded.windows(2).filter(|w| w[0] == 0x1b && w[1] == b'[').count()
    );
    assert!(!decoded.is_empty(), "a live pane has a screen");

    let marker = format!("PHONE_{}", std::process::id());
    let hex: String = format!("echo {marker}").bytes().map(|b| format!("{b:02x}")).collect();
    call(handle, "terminal.write", serde_json::json!({ "terminal": terminal, "hex": hex }));
    call(handle, "terminal.write", serde_json::json!({ "terminal": terminal, "hex": "0d" }));
    std::thread::sleep(std::time::Duration::from_millis(1500));

    let after = call(handle, "terminal.screen", serde_json::json!({ "terminal": terminal }));
    let text =
        String::from_utf8_lossy(&decode_base64(after["contents"].as_str().unwrap()).unwrap())
            .into_owned();
    assert!(text.contains(&marker), "what was typed has to appear on the screen");
    println!("typed `echo {marker}` and read it back");
    println!("PROBE PASSED");
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut index = [255u8; 256];
    for (i, b) in A.iter().enumerate() {
        index[*b as usize] = i as u8;
    }
    let clean: Vec<u8> = text.bytes().filter(|b| *b != b'=' && !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            let v = index[*b as usize];
            if v == 255 {
                return None;
            }
            n |= u32::from(v) << (18 - i * 6);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..chunk.len() - 1]);
    }
    Some(out)
}

//! A fake `farcoolerd --stdio` for the browser check
//! (`services/web/test/browser/csp-check.mjs`), run by a real sshd as a Key A
//! line's forced command.
//!
//! The browser's key is a passkey, and only a real sshd verifies a passkey's
//! `webauthn-sk` signature, so the check puts one in front of this rather than
//! `tests/support`'s russh server. sshd starts one of these per channel: the
//! control session (hello, `terminal.list`, `terminal.write`,
//! `terminal.resize`) and the pane's stream (`terminal.attach`, answered with
//! a replay). Each `terminal.write` is appended, as hex, to the file named by
//! the one argument. Never a real runner: nothing here touches a shell.
//!
//!   fake_daemon <typed-log>

#[path = "../tests/support/mod.rs"]
mod support;

use std::io::{Read as _, Write as _};

use bytes::{Bytes, BytesMut};
use farcooler_protocol::v1::{
    Empty, ServerHello, Terminal, TerminalAttachResult, TerminalList, TerminalState, request, result, wire_envelope,
};
use farcooler_protocol::{capability, framing};

fn main() {
    let typed_log = std::env::args().nth(1).expect("fake_daemon <typed-log>");
    let (mut stdin, mut stdout) = (std::io::stdin().lock(), std::io::stdout().lock());
    let mut buf = BytesMut::new();
    let mut chunk = [0u8; 1 << 14];
    loop {
        let Some(env) = framing::decode(&mut buf).expect("a frame") else {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            continue;
        };
        let out: Vec<Vec<u8>> = match env.body {
            Some(wire_envelope::Body::ClientHello(_)) => {
                vec![support::envelope(wire_envelope::Body::ServerHello(ServerHello {
                    selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                    capabilities: vec![capability::TERMINAL_STREAM.to_string()],
                    ..Default::default()
                }))]
            }
            Some(wire_envelope::Body::Request(req)) => match (req.method.as_str(), req.payload.clone()) {
                ("terminal.list", _) => vec![support::answer(
                    &req,
                    result::Value::TerminalList(TerminalList {
                        items: vec![Terminal {
                            id: Bytes::copy_from_slice(support::TERMINAL.as_bytes()),
                            title: "shell".into(),
                            state: TerminalState::Running as i32,
                            columns: 80,
                            rows: 24,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )],
                ("terminal.attach", _) => vec![
                    support::answer(&req, result::Value::TerminalAttach(TerminalAttachResult::default())),
                    support::output(b"replay$ "),
                ],
                ("terminal.write", Some(request::Payload::TerminalWrite(w))) => {
                    let hex: String = w.payload.iter().map(|b| format!("{b:02x}")).collect();
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&typed_log).unwrap();
                    writeln!(log, "{hex}").unwrap();
                    vec![support::answer(&req, result::Value::Empty(Empty {}))]
                }
                ("terminal.resize", _) => vec![support::answer(&req, result::Value::Empty(Empty {}))],
                (other, _) => panic!("the fake daemon doesn't know {other}"),
            },
            _ => continue,
        };
        for frame in out {
            stdout.write_all(&frame).unwrap();
        }
        stdout.flush().unwrap();
    }
}

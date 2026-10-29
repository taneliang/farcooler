//! A fake `farcoolerd --stdio` for the browser check
//! (`services/web/test/browser/csp-check.mjs`), run by a real sshd as a Key A
//! line's forced command.
//!
//! The browser's key is a passkey, and only a real sshd verifies a passkey's
//! `webauthn-sk` signature, so the check puts one in front of this rather than
//! `tests/support`'s russh server. sshd starts one of these per channel: the
//! control session (hello, `terminal.list`, `terminal.write`,
//! `terminal.resize`) and each pane's stream (`terminal.attach`, answered with
//! a replay). Each `terminal.write` is appended, as hex, to the file named by
//! the one argument. Never a real runner: nothing here touches a shell.
//!
//! **Twelve terminals** (ids ending 01 to 0c), so the check can open more
//! panes than a connection has channels (ov-774).
//!
//! **A write reaches its pane, as the real daemon's does.** The control
//! session and a pane's stream are different processes here, as they are for
//! the real daemon's `--stdio` relays, so the control session hands each write
//! to the pane's process through a datagram socket in a directory beside the
//! typed log. The pane echoes it. A line typed as `seq 1 2000000` and sent
//! with Return starts the output that command would make (2,000,000 lines),
//! which is how the check loads one pane while it types in another.
//!
//! **One ask waiting** (ov-861), named for the runner (`fake_daemon <typed-log>
//! <runner>`), so the check can run two runners and see each one's item and
//! which of them an answer reached. `terminal.agent_answer` appends the ask's
//! id and the option to `answers` beside the typed log, and the ask is gone
//! from the next `needs_you.list`.
//!
//!   fake_daemon <typed-log> [runner]

#[path = "../tests/support/mod.rs"]
mod support;

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bytes::{Bytes, BytesMut};
use farcooler_protocol::v1::{
    Empty, NeedsYouAction, NeedsYouItem, NeedsYouKind, NeedsYouList, ServerHello, Terminal, TerminalAttachResult, TerminalList, TerminalRef,
    TerminalState, request, result, wire_envelope,
};
use farcooler_protocol::{capability, framing};
use uuid::Uuid;

/// How many terminals this runner has.
const TERMINALS: u8 = 12;
/// The line whose Return starts the load.
const LOAD: &[u8] = b"seq 1 2000000";

fn terminal(n: u8) -> Uuid {
    Uuid::from_u128(support::TERMINAL.as_u128() - 1 + u128::from(n))
}

/// The socket pane `n`'s process listens on for what is typed into it. A
/// short name: a unix socket's path is limited to about a hundred bytes.
fn socket(typed_log: &Path, n: u8) -> PathBuf {
    typed_log.with_file_name(format!("p{n}"))
}

/// The ask waiting on this runner, unless it has been answered.
fn waiting(typed_log: &Path, runner: &str) -> NeedsYouList {
    let answered = std::fs::read_to_string(typed_log.with_file_name("answers")).unwrap_or_default();
    let ask = format!("ask-{runner}");
    if answered.lines().any(|l| l.split(' ').next() == Some(ask.as_str())) {
        return NeedsYouList::default();
    }
    let action = |id: &str, title: &str, primary: bool| NeedsYouAction { id: id.into(), title: title.into(), primary, ..Default::default() };
    NeedsYouList {
        items: vec![NeedsYouItem {
            id: format!("ask:{runner}"),
            kind: NeedsYouKind::Ask as i32,
            rank: 1,
            workspace_name: "Main".into(),
            question: format!("Run the tests on {runner}?"),
            ask_id: Some(ask),
            terminal: Some(TerminalRef { id: Bytes::copy_from_slice(terminal(1).as_bytes()), label: "shell 1".into(), ..Default::default() }),
            actions: vec![action("allow", "Allow", true), action("deny", "Deny", false)],
            ..Default::default()
        }],
    }
}

/// The output of `seq 1 2000000`, in frames of about 16 KiB.
fn load(out: Arc<Mutex<std::io::Stdout>>, id: Uuid) {
    std::thread::spawn(move || {
        let mut chunk = Vec::with_capacity(1 << 15);
        for n in 1..=2_000_000u32 {
            chunk.extend_from_slice(format!("{n}\r\n").as_bytes());
            if chunk.len() >= 1 << 14 || n == 2_000_000 {
                let frame = support::output_for(id, &chunk);
                chunk.clear();
                let mut out = out.lock().unwrap();
                if out.write_all(&frame).and_then(|_| out.flush()).is_err() {
                    return;
                }
            }
        }
    });
}

/// A pane's process: a replay, then an echo of what the control session hands
/// it, until the channel closes (stdin ends).
fn pane(typed_log: &Path, n: u8, out: Arc<Mutex<std::io::Stdout>>) {
    let path = socket(typed_log, n);
    let _ = std::fs::remove_file(&path);
    let listener = UnixDatagram::bind(&path).expect("a socket for the pane");
    let id = terminal(n);
    std::thread::spawn(move || {
        let (mut line, mut buf) = (Vec::new(), [0u8; 1 << 12]);
        while let Ok(got) = listener.recv(&mut buf) {
            let mut echo = Vec::new();
            for &byte in &buf[..got] {
                if byte == b'\r' {
                    echo.extend_from_slice(b"\r\n");
                    if line == LOAD {
                        load(out.clone(), id);
                    }
                    line.clear();
                } else {
                    echo.push(byte);
                    line.push(byte);
                }
            }
            let mut out = out.lock().unwrap();
            if out.write_all(&support::output_for(id, &echo)).and_then(|_| out.flush()).is_err() {
                return;
            }
        }
    });
}

fn main() {
    let typed_log = PathBuf::from(std::env::args().nth(1).expect("fake_daemon <typed-log>"));
    let runner = std::env::args().nth(2).unwrap_or_else(|| "studio".into());
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let mut stdin = std::io::stdin().lock();
    let mut buf = BytesMut::new();
    let mut chunk = [0u8; 1 << 14];
    let mut mine = None;
    loop {
        let Some(env) = framing::decode(&mut buf).expect("a frame") else {
            match stdin.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            continue;
        };
        let target = |req: &farcooler_protocol::v1::Request| {
            req.target_resource_id.as_deref().and_then(|b| b.last().copied()).filter(|n| (1..=TERMINALS).contains(n))
        };
        let out: Vec<Vec<u8>> = match env.body {
            Some(wire_envelope::Body::ClientHello(_)) => {
                vec![support::envelope(wire_envelope::Body::ServerHello(ServerHello {
                    selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                    capabilities: vec![
                        capability::TERMINAL_STREAM.to_string(),
                        capability::WORKSTREAMS.to_string(),
                        capability::BOARD_PLAN.to_string(),
                        capability::NEEDS_YOU.to_string(),
                    ],
                    ..Default::default()
                }))]
            }
            Some(wire_envelope::Body::Request(req)) => match (req.method.as_str(), req.payload.clone()) {
                ("terminal.list", _) => vec![support::answer(
                    &req,
                    result::Value::TerminalList(TerminalList {
                        items: (1..=TERMINALS)
                            .map(|n| Terminal {
                                id: Bytes::copy_from_slice(terminal(n).as_bytes()),
                                title: format!("shell {n}"),
                                state: TerminalState::Running as i32,
                                columns: 80,
                                rows: 24,
                                ..Default::default()
                            })
                            .collect(),
                        ..Default::default()
                    }),
                )],
                ("terminal.attach", _) => {
                    let n = target(&req).expect("an attach names a terminal");
                    pane(&typed_log, n, stdout.clone());
                    mine = Some(socket(&typed_log, n));
                    vec![
                        support::answer(&req, result::Value::TerminalAttach(TerminalAttachResult::default())),
                        support::output_for(terminal(n), b"replay$ "),
                    ]
                }
                ("terminal.write", Some(request::Payload::TerminalWrite(w))) => {
                    let hex: String = w.payload.iter().map(|b| format!("{b:02x}")).collect();
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&typed_log).unwrap();
                    writeln!(log, "{hex}").unwrap();
                    if let Some(n) = target(&req) {
                        // A pane nobody is attached to has no socket: its
                        // keystrokes go nowhere, as a detached pane's would.
                        let _ = UnixDatagram::unbound().and_then(|s| s.send_to(&w.payload, socket(&typed_log, n)));
                    }
                    vec![support::answer(&req, result::Value::Empty(Empty {}))]
                }
                // The plan tab's reads (ov-775): one board with one theme.
                ("workspace.list", _) => vec![support::answer(
                    &req,
                    result::Value::WorkspaceList(farcooler_protocol::v1::WorkspaceList {
                        items: vec![farcooler_protocol::v1::Workspace {
                            id: Bytes::copy_from_slice(support::WORKSPACE.as_bytes()),
                            name: "Main".into(),
                            is_main: true,
                            ..Default::default()
                        }],
                    }),
                )],
                ("plan.get", _) => vec![support::answer(
                    &req,
                    result::Value::Plan(farcooler_protocol::v1::Plan {
                        themes: vec![farcooler_protocol::v1::BoardThemeView {
                            theme: Some(farcooler_protocol::v1::BoardTheme {
                                id: Bytes::copy_from_slice(Uuid::from_u128(0xb01).as_bytes()),
                                name: "Visual language".into(),
                                next: "Frosted window plane".into(),
                                state: farcooler_protocol::v1::BoardThemeState::Active as i32,
                                ..Default::default()
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                )],
                ("needs_you.list", _) => vec![support::answer(&req, result::Value::NeedsYouList(waiting(&typed_log, &runner)))],
                ("terminal.agent_answer", Some(request::Payload::AgentAnswer(a))) => {
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(typed_log.with_file_name("answers")).unwrap();
                    writeln!(log, "{} {}", a.request_id, a.option_id).unwrap();
                    vec![support::answer(&req, result::Value::Terminal(Terminal { id: a.terminal_id, ..Default::default() }))]
                }
                ("terminal.resize", _) => vec![support::answer(&req, result::Value::Empty(Empty {}))],
                // The page's liveness probe (ov-772): a link that never
                // answers it is given up on after nine quiet seconds.
                ("daemon.version", _) => vec![support::answer(
                    &req,
                    result::Value::DaemonVersion(farcooler_protocol::v1::DaemonVersion { daemon_version: "fake".into(), ..Default::default() }),
                )],
                (other, _) => panic!("the fake daemon doesn't know {other}"),
            },
            _ => continue,
        };
        let mut stdout = stdout.lock().unwrap();
        for frame in out {
            stdout.write_all(&frame).unwrap();
        }
        stdout.flush().unwrap();
    }
    if let Some(path) = mine {
        let _ = std::fs::remove_file(path);
    }
}

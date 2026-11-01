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
//! **Two workspaces** (ov-871): terminals 1 to 12 work for Main, 13 (a shell)
//! and 14 (an agent) for Billing, which has a plan of its own with a ticket on
//! one card (`bil-7`, a task with notes). Each item Needs You lists names its
//! workspace. A file called `billing` beside the typed log adds Billing's two
//! items, an ask and a decision on that task; the decision's answer is a
//! `task.note` and is appended to `notes` beside the typed log. `terminal.compose`
//! appends its text to `composed` and each image's type and size to `images`.
//! An agent's conversation is one turn and a reply.
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
    Empty, NeedsYouAction, NeedsYouItem, NeedsYouKind, NeedsYouList, ServerHello, TaskRef, Terminal, TerminalAttachResult, TerminalList, TerminalRef,
    TerminalRole, TerminalState, request, result, wire_envelope,
};
use farcooler_protocol::{capability, framing};
use uuid::Uuid;

/// How many terminals this runner has: twelve for Main, then Billing's two.
const TERMINALS: u8 = 14;
/// The first of Billing's terminals.
const BILLING_FROM: u8 = 13;
/// Billing's id, and its decision's task.
const BILLING: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_00a2);
const TASK: Uuid = Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0d01);

fn id_bytes(id: Uuid) -> Bytes {
    Bytes::copy_from_slice(id.as_bytes())
}

/// The workspace terminal `n` works for.
fn workspace(n: u8) -> Uuid {
    if n >= BILLING_FROM { BILLING } else { support::WORKSPACE }
}

fn title(n: u8) -> String {
    match n {
        13 => "billing shell".into(),
        14 => "billing agent".into(),
        n => format!("shell {n}"),
    }
}

/// An agent's conversation, as the projector's rows: a turn and a reply.
const TURN: &str = r#"{"ord":0,"rev":1,"id":"turn:a","turn":null,"provisional":false,"kind":{"Turn":{"prompt":"Add the invoice PDF export","origin":"Typed","outcome":null,"background_running":0,"activity":"Idle"}}}"#;
const PROSE: &str = r#"{"ord":1,"rev":2,"id":"prose:1","turn":"turn:a","provisional":false,"kind":{"Prose":{"text":"The export is written. Tell me what to change.","conclusion":false,"at_ms":null}}}"#;
const EPOCH: u64 = 1_700_000_000_000_001;
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

/// What waits on this runner, unless it has been answered: its ask for Main,
/// and, once `billing` exists, Billing's ask and its decision.
fn waiting(typed_log: &Path, runner: &str) -> NeedsYouList {
    let answered = std::fs::read_to_string(typed_log.with_file_name("answers")).unwrap_or_default();
    let noted = std::fs::read_to_string(typed_log.with_file_name("notes")).unwrap_or_default();
    let action = |id: &str, title: &str, primary: bool| NeedsYouAction { id: id.into(), title: title.into(), primary, ..Default::default() };
    let mut items = Vec::new();
    let ask = format!("ask-{runner}");
    if !answered.lines().any(|l| l.split(' ').next() == Some(ask.as_str())) {
        items.push(NeedsYouItem {
            id: format!("ask:{runner}"),
            kind: NeedsYouKind::Ask as i32,
            rank: 1,
            workspace_id: id_bytes(support::WORKSPACE),
            workspace_name: "Main".into(),
            question: format!("Run the tests on {runner}?"),
            ask_id: Some(ask),
            terminal: Some(TerminalRef { id: id_bytes(terminal(1)), label: "shell 1".into(), ..Default::default() }),
            actions: vec![action("allow", "Allow", true), action("deny", "Deny", false)],
            ..Default::default()
        });
    }
    if typed_log.with_file_name("billing").exists() {
        if !answered.lines().any(|l| l.split(' ').next() == Some("ask-billing")) {
            items.push(NeedsYouItem {
                id: "ask:billing".into(),
                kind: NeedsYouKind::Ask as i32,
                rank: 2,
                workspace_id: id_bytes(BILLING),
                workspace_name: "Billing".into(),
                question: "Run the invoice tests?".into(),
                ask_id: Some("ask-billing".into()),
                terminal: Some(TerminalRef { id: id_bytes(terminal(BILLING_FROM)), label: "billing shell".into(), ..Default::default() }),
                actions: vec![action("allow", "Allow", true), action("deny", "Deny", false)],
                ..Default::default()
            });
        }
        if noted.is_empty() {
            items.push(NeedsYouItem {
                id: "decision:billing".into(),
                kind: NeedsYouKind::Decision as i32,
                rank: 3,
                workspace_id: id_bytes(BILLING),
                workspace_name: "Billing".into(),
                question: "Which invoice format first?".into(),
                task: Some(TaskRef { id: id_bytes(TASK), key: "bil-7".into(), title: "Invoice export".into(), ..Default::default() }),
                actions: vec![action("PDF", "PDF", true), action("CSV", "CSV", false)],
                ..Default::default()
            });
        }
    }
    NeedsYouList { items }
}

/// Billing's task, with a note.
fn task_detail() -> farcooler_protocol::v1::TaskDetail {
    use farcooler_protocol::v1 as pb;
    pb::TaskDetail {
        task: Some(pb::Task {
            id: id_bytes(TASK),
            key: "bil-7".into(),
            title: "Invoice export".into(),
            intent: "Customers download an invoice as a file.".into(),
            status: pb::TaskStatus::InProgress as i32,
            workspace_id: id_bytes(BILLING),
            acceptance: vec![
                pb::TaskAcceptanceItem { id: Bytes::from_static(&[3; 16]), text: "A PDF opens in Preview".into(), met: true },
                pb::TaskAcceptanceItem { id: Bytes::from_static(&[4; 16]), text: "A CSV opens in Numbers".into(), met: false },
            ],
            ..Default::default()
        }),
        notes: vec![pb::TaskNote {
            id: Bytes::from_static(&[5; 16]),
            task_id: id_bytes(TASK),
            kind: pb::TaskNoteKind::Question as i32,
            actor: "orchestrator".into(),
            at: 1_800_000_000_000,
            body: "PDF or CSV first?".into(),
            ..Default::default()
        }],
        ..Default::default()
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

fn chat_page() -> farcooler_protocol::v1::AgentRowPage {
    use farcooler_protocol::v1::{AgentRow, AgentRowPage};
    let row = |id: &str, ord: u64, rev: u64, json: &str| AgentRow { id: id.into(), ord, rev, row_json: json.into() };
    AgentRowPage { epoch: EPOCH, rev: 2, rows: vec![row("turn:a", 0, 1, TURN), row("prose:1", 1, 2, PROSE)], ..Default::default() }
}

fn chat_nothing_new() -> farcooler_protocol::v1::AgentRowChanges {
    farcooler_protocol::v1::AgentRowChanges { epoch: EPOCH, rev: 2, ..Default::default() }
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
                        capability::TASKS.to_string(),
                        capability::AGENT_ROWS.to_string(),
                        capability::AGENT_COMPOSE.to_string(),
                        capability::COMPOSE.to_string(),
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
                                title: title(n),
                                state: TerminalState::Running as i32,
                                columns: 80,
                                rows: 24,
                                workspace_id: Some(id_bytes(workspace(n))),
                                role: if n == 14 { TerminalRole::Agent as i32 } else { TerminalRole::Shell as i32 },
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
                // The plan tab's reads (ov-775): Main's board has one theme, and
                // Billing's a theme and a ticket on one card.
                ("workspace.list", _) => vec![support::answer(
                    &req,
                    result::Value::WorkspaceList(farcooler_protocol::v1::WorkspaceList {
                        items: vec![
                            farcooler_protocol::v1::Workspace { id: id_bytes(support::WORKSPACE), name: "Main".into(), is_main: true, ..Default::default() },
                            farcooler_protocol::v1::Workspace { id: id_bytes(BILLING), name: "Billing".into(), ordinal: 1, ..Default::default() },
                        ],
                    }),
                )],
                ("plan.get", Some(request::Payload::PlanGet(get))) => {
                    use farcooler_protocol::v1 as pb;
                    let billing = get.workspace_id == id_bytes(BILLING);
                    let theme = |id: u128, name: &str, next: &str, tasks: Vec<bytes::Bytes>, counts: pb::PlanStatusCounts| pb::BoardThemeView {
                        theme: Some(pb::BoardTheme {
                            id: id_bytes(Uuid::from_u128(id)),
                            name: name.into(),
                            next: next.into(),
                            state: pb::BoardThemeState::Active as i32,
                            ..Default::default()
                        }),
                        task_ids: tasks,
                        counts: Some(counts),
                        ..Default::default()
                    };
                    let plan = if billing {
                        // Billing's plan has the column's parts (ov-875): an
                        // initiative in motion, one builder with its ticket,
                        // and the workspace's orchestrator in its pane.
                        let now = 1_800_000_000_000_i64;
                        pb::Plan {
                            now_ms: now,
                            themes: vec![theme(
                                0xb02,
                                "Invoices",
                                "PDF export",
                                vec![id_bytes(TASK)],
                                pb::PlanStatusCounts { done: 1, in_progress: 1, ..Default::default() },
                            )],
                            lines: vec![pb::TicketLine {
                                task_id: id_bytes(TASK),
                                stage: pb::TicketStage::Building as i32,
                                reason: "Writing the PDF".into(),
                                stage_since: now - 5 * 60_000,
                                ..Default::default()
                            }],
                            cards: vec![pb::PlanCard { task_id: id_bytes(TASK), key: "bil-7".into(), title: "Invoice export".into(), ..Default::default() }],
                            agents: vec![
                                pb::Agent {
                                    id: id_bytes(Uuid::from_u128(0xa01)),
                                    role: pb::AgentRole::Orchestrator as i32,
                                    harness: "claude".into(),
                                    terminal_id: Some(id_bytes(terminal(14))),
                                    working: true,
                                    started_at: now - 60 * 60_000,
                                    last_at: now,
                                    ..Default::default()
                                },
                                pb::Agent {
                                    id: id_bytes(Uuid::from_u128(0xa02)),
                                    role: pb::AgentRole::Builder as i32,
                                    harness: "claude".into(),
                                    doing_now: "Writing the PDF".into(),
                                    terminal_id: Some(id_bytes(terminal(BILLING_FROM))),
                                    tickets: vec![pb::AgentTicket {
                                        task_id: id_bytes(TASK),
                                        role: pb::WorkRole::Build as i32,
                                        started_at: now - 5 * 60_000,
                                    }],
                                    working: true,
                                    started_at: now - 5 * 60_000,
                                    last_at: now,
                                    ..Default::default()
                                },
                            ],
                            ..Default::default()
                        }
                    } else {
                        pb::Plan {
                            themes: vec![theme(0xb01, "Visual language", "Frosted window plane", vec![], pb::PlanStatusCounts { backlog: 2, ..Default::default() })],
                            ..Default::default()
                        }
                    };
                    vec![support::answer(&req, result::Value::Plan(plan))]
                }
                ("task.get", _) => vec![support::answer(&req, result::Value::TaskDetail(task_detail()))],
                ("task.note", Some(request::Payload::TaskNoteAppend(n))) => {
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(typed_log.with_file_name("notes")).unwrap();
                    writeln!(log, "{} {}", n.kind, n.body).unwrap();
                    vec![support::answer(&req, result::Value::TaskNote(Default::default()))]
                }
                ("agent.rows", _) => vec![support::answer(&req, result::Value::AgentRowPage(chat_page()))],
                ("agent.rows_follow", _) => {
                    // Held a moment, off this loop: the control session must
                    // go on answering keystrokes while a conversation waits.
                    let (out, done) = (stdout.clone(), support::answer(&req, result::Value::AgentRowChanges(chat_nothing_new())));
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        let mut out = out.lock().unwrap();
                        let _ = out.write_all(&done).and_then(|_| out.flush());
                    });
                    continue;
                }
                ("terminal.compose", Some(request::Payload::AgentPrompt(p))) => {
                    use farcooler_protocol::v1::agent_prompt_block::Content;
                    let mut text = String::new();
                    let mut images = std::fs::OpenOptions::new().create(true).append(true).open(typed_log.with_file_name("images")).unwrap();
                    for block in p.blocks {
                        match block.content {
                            Some(Content::Text(t)) => text = t,
                            Some(Content::Image(i)) => writeln!(images, "{} {}", i.mime_type, i.data.len()).unwrap(),
                            _ => {}
                        }
                    }
                    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(typed_log.with_file_name("composed")).unwrap();
                    writeln!(log, "{text}").unwrap();
                    vec![support::answer(&req, result::Value::TerminalTold(farcooler_protocol::v1::TerminalTold { queued: false }))]
                }
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

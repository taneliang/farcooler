//! The agent screen's controls, the whole way a phone sends them: `dispatch`
//! with the JSON an app passes, a `Session`, the daemon's own `RpcFactory` at
//! the scope the phone was enrolled with, and the shim at the far end.
//!
//! In process, with the daemon as a library, so a test can stand where the
//! shim stands and see what reached it. Every hop is the shipped code except
//! two: SSH, which is a byte pipe under the session, and the C strings around
//! `farcooler_client_call`, which carry this JSON unchanged.
//!
//! It exists because each hop had its own list of methods and they disagreed.
//! The daemon refused Edit, Cancel and Send Now on a queued message as a
//! method it did not know, and the FFI had no arm for the model and config
//! pickers. Every unit test was green.

use std::sync::Arc;

use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher};
use farcooler_protocol::method::Method;
use farcooler_protocol::v1::{ErrorCode, Scope, TerminalIntent};
use farcooler_transport::{HandshakeConfig, Peer, UnixListenerServer};
use serde_json::{Value, json};
use tokio::io::AsyncBufReadExt;

use super::{dispatch, route::route};
use crate::session::{Session, SessionError};

/// A runner whose every connection is at one scope, with one agent pane.
pub(super) struct Runner {
    _dir: tempfile::TempDir,
    pub(super) socket: std::path::PathBuf,
    pub(super) service: Arc<Service>,
    pane: uuid::Uuid,
}

/// Take down any tmux server the service started, as the daemon's own
/// harnesses do and for their reason: nothing else ever will.
impl Drop for Runner {
    fn drop(&mut self) {
        farcooler_tmux::reap_server(self.service.tmux.socket());
    }
}

pub(super) async fn a_runner(scope: Scope) -> Runner {
    // Short, because the agent socket lives in it and macOS caps a socket path
    // at 104 bytes. The system temporary directory alone is most of that.
    std::fs::create_dir_all("/tmp/fc-phone").unwrap();
    let dir = tempfile::Builder::new().prefix("p").tempdir_in("/tmp/fc-phone").unwrap();
    let socket = dir.path().join("d.sock");
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let service = Arc::new(
        Service::open_in(dir.path().to_path_buf())
            .await
            .expect("service")
            .enrolling_into(home.join(".ssh").join("authorized_keys")),
    );

    // A repository, its checkout and a claude pane, made in the store: nothing
    // here needs git or tmux, only a pane id the daemon can find.
    let store = &service.store;
    let host = uuid::Uuid::now_v7();
    let root = store.create_repository_root(host, "/repos/phone", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "repo", "/repos/phone/.git", "").unwrap();
    let main = store.ensure_main_workspace(repo.id).unwrap();
    let worktree = store.create_worktree(repo.id, "main", "/repos/phone", true).unwrap();
    store.assign_worktree(worktree.id, main.id).unwrap();
    let pane = store
        .create_terminal_for_task(worktree.id, "pane", "claude", TerminalIntent::Running, 80, 24, None)
        .unwrap()
        .id;

    let server = UnixListenerServer::bind(&socket).expect("bind");
    let (svc, watcher) = (service.clone(), Watcher::new(service.clone()));
    tokio::spawn(async move {
        let _ = server
            .serve(move |_| {
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
                    RpcFactory::new(
                        svc.clone(),
                        watcher.clone(),
                        Arc::new(tokio::sync::Notify::new()),
                        Peer { client_id: None, scope },
                    ),
                ))
            })
            .await;
    });
    tokio::task::yield_now().await;
    Runner { _dir: dir, socket, service, pane }
}

/// The pane's shim, as `farcooler agent-host` is to the daemon.
struct Shim {
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>,
    _write: tokio::net::unix::OwnedWriteHalf,
}

impl Shim {
    /// Dial the pane's agent socket and wait for the daemon's `subscribe`.
    ///
    /// Waiting is what makes the next call deterministic: the daemon registers
    /// the shim as the pane's writer before it writes that line, so a message
    /// sent after it has somewhere to go.
    async fn dial(runner: &Runner) -> Shim {
        let root = runner.service.root_dir().to_path_buf();
        runner.service.agents().ensure_listening(&root, runner.pane);
        let path = farcooler_daemon::agent_supervisor::socket_path(&root, runner.pane);
        for _ in 0..100 {
            if let Ok(stream) = tokio::net::UnixStream::connect(&path).await {
                let (read, write) = stream.into_split();
                let mut shim = Shim { lines: tokio::io::BufReader::new(read).lines(), _write: write };
                assert_eq!(shim.next(5_000).await.expect("a subscribe")["kind"], "subscribe");
                return shim;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the runner never listened for its pane");
    }

    /// The next line the daemon sent, as JSON, or `None` if it sent nothing
    /// within `millis`.
    async fn next(&mut self, millis: u64) -> Option<Value> {
        let line = tokio::time::timeout(std::time::Duration::from_millis(millis), self.lines.next_line())
            .await
            .ok()?
            .expect("the socket is readable")?;
        Some(serde_json::from_str(&line).expect("a JSON line"))
    }
}

/// Each agent-screen control a phone sends that changes the conversation, with
/// the arguments iOS and Android pass (`AgentStream.swift`, `AgentStream.kt`)
/// and the line the shim must receive for it.
fn the_agent_screen_controls(pane: uuid::Uuid) -> Vec<(&'static str, Value, Value)> {
    let terminal = pane.to_string();
    vec![
        (
            "terminal.agent_edit_queued",
            json!({ "terminal": terminal, "queuedId": "q-1", "text": "say it shorter" }),
            json!({ "kind": "edit_queued", "id": "q-1", "text": "say it shorter" }),
        ),
        (
            "terminal.agent_steer_queued",
            json!({ "terminal": terminal, "queuedId": "q-2" }),
            json!({ "kind": "steer_queued", "id": "q-2" }),
        ),
        (
            "terminal.agent_cancel_queued",
            json!({ "terminal": terminal, "queuedId": "q-3" }),
            json!({ "kind": "cancel_queued", "id": "q-3" }),
        ),
        (
            "terminal.agent_set_model",
            json!({ "terminal": terminal, "model": "opus" }),
            json!({ "kind": "set_model", "model": "opus" }),
        ),
        (
            "terminal.agent_set_config",
            json!({ "terminal": terminal, "configId": "effort", "value": "high" }),
            json!({ "kind": "set_config", "id": "effort", "value": "high" }),
        ),
    ]
}

/// A phone enrolled at `control`, the scope that can type into a pane, edits,
/// sends and withdraws its own queued messages and changes the model and a
/// config option, and each one reaches the agent.
#[tokio::test]
async fn a_control_scoped_phone_s_queue_and_pickers_reach_the_agent() {
    let runner = a_runner(Scope::Control).await;
    let mut shim = Shim::dial(&runner).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    for (method, args, heard) in the_agent_screen_controls(runner.pane) {
        if let Err(e) = dispatch(&session, method, &args).await {
            panic!("{method} from a control-scoped phone was refused: {e:?}");
        }
        assert_eq!(shim.next(5_000).await, Some(heard), "{method} did not reach the agent");
    }
}

/// A phone enrolled at `read` sees the shape of the fleet and cannot type into
/// a pane, so it cannot rewrite, send or withdraw what is queued for one
/// either: each control is refused as a scope denial, by name, and nothing
/// reaches the agent.
#[tokio::test]
async fn a_read_scoped_phone_is_refused_the_queue_and_pickers_and_the_agent_hears_nothing() {
    let runner = a_runner(Scope::Read).await;
    let mut shim = Shim::dial(&runner).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    for (method, args, _) in the_agent_screen_controls(runner.pane) {
        match dispatch(&session, method, &args).await {
            Err(SessionError::Refused { code, .. }) => assert_eq!(
                code,
                ErrorCode::ScopeDenied as i32,
                "{method} from a read-scoped phone has to be a scope denial, not some other refusal"
            ),
            other => panic!("{method} from a read-scoped phone was not refused: {other:?}"),
        }
    }
    assert_eq!(shim.next(300).await, None, "a refused control reached the agent");
}

/// A runner that refuses every request it is sent, as `not_found`.
///
/// For asking whether `dispatch` has an arm for a name, and nothing else: an
/// arm that reaches the runner comes back refused, an arm that checks its
/// arguments first comes back with its own complaint, and only a name with no
/// arm at all comes back as `unknown method`.
async fn a_runner_that_refuses_everything(socket: &std::path::Path) {
    use farcooler_protocol::v1::{self as pb, response, wire_envelope};
    use farcooler_transport::codec::{FrameReader, FrameWriter};
    let envelope = |body| pb::WireEnvelope {
        protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(),
        body: Some(body),
    };
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else { return };
        let (read, write) = stream.into_split();
        let mut reader = FrameReader::new(read);
        let mut writer = FrameWriter::new(write);
        let Ok(Some(_hello)) = reader.read_frame().await else { return };
        let hello = envelope(wire_envelope::Body::ServerHello(pb::ServerHello {
            selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            daemon_version: "a runner that refuses everything".into(),
            max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
            max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
            capabilities: farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        }));
        if writer.write_frame(&hello).await.is_err() {
            return;
        }
        while let Ok(Some(frame)) = reader.read_frame().await {
            let Some(wire_envelope::Body::Request(req)) = frame.body else { continue };
            let reply = envelope(wire_envelope::Body::Response(pb::Response {
                request_id: req.request_id,
                outcome: Some(response::Outcome::Error(pb::Error {
                    code: ErrorCode::NotFound as i32,
                    message: "refused".into(),
                    ..Default::default()
                })),
            }));
            if writer.write_frame(&reply).await.is_err() {
                return;
            }
        }
    });
}

/// Whether `dispatch` has an arm for `name`, asked of a runner that refuses
/// everything, with no arguments. A routed name with no arm falls to the
/// last arm's "routed but not handled", which is the miss this looks for
/// (ov-373: it used to look only for "unknown method", which a routed name
/// never answers, so a removed arm passed).
async fn is_routed(session: &mut Session, name: &str) -> bool {
    let answer = tokio::time::timeout(std::time::Duration::from_secs(5), dispatch(session, name, &json!({})))
        .await
        .unwrap_or_else(|_| panic!("{name} never answered"));
    !matches!(answer, Err(SessionError::Protocol(m))
        if m == format!("unknown method: {name}") || m == format!("method routed but not handled: {name}"))
}

/// Every wire method the iOS and Android apps name in their sources.
///
/// Read out of the apps rather than listed here, because a list typed here is
/// the thing that drifted: it held five hand-picked groups, and the model and
/// config pickers were in none of them. A string literal that is not a wire
/// method (`"fleet"`, a key, a label) parses as no `Method` and is ignored.
///
/// The phone apps, and the AgentKit sources the iPhone, the watch and its
/// widget compile into themselves (`agentkit_the_phones_compile`): iOS's
/// answer is named there, as `PhoneTaskAnswer.method`, and a task write
/// added beside it would reach a phone screen as surely as one in
/// `apps/ios`. The rest of AgentKit is the Mac's, which reaches its runner
/// through the CLI.
fn the_wire_methods_the_phones_name() -> std::collections::BTreeMap<Method, String> {
    let apps = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let mut files = Vec::new();
    let mut pending = vec![apps.join("ios"), apps.join("android/app/src/main")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|x| x == "swift" || x == "kt") {
                files.push(path);
            }
        }
    }
    let generator = std::fs::read_to_string(apps.join("ios/generate-project.py")).unwrap();
    let agentkit = agentkit_the_phones_compile(&generator, &apps.join("shared/AgentKit/Sources/AgentKit"));
    assert!(agentkit.len() > 50, "found {} phone-compiled AgentKit sources, so this proves nothing", agentkit.len());
    files.extend(agentkit);
    methods_named_in(&files)
}

/// AgentKit's sources that the iPhone, the watch and its widget compile, by
/// the three lists in `apps/ios/generate-project.py` that build them. iOS has
/// no SwiftPM project; it compiles exactly these files, so these and no
/// others are AgentKit's phone code. A listed file that isn't there panics
/// when it is read.
fn agentkit_the_phones_compile(generator: &str, sources: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for list in ["AGENTKIT_SOURCES", "WATCH_AGENTKIT_SOURCES", "WATCH_WIDGET_AGENTKIT_SOURCES"] {
        let head = format!("\n{list} = [");
        let at = generator.find(&head).unwrap_or_else(|| panic!("generate-project.py has no {list}"));
        let body = &generator[at + head.len()..];
        let body = &body[..body.find("\n]").unwrap_or_else(|| panic!("{list} never closes"))];
        for line in body.lines().filter(|l| !l.trim_start().starts_with('#')) {
            files.extend(line.split('"').skip(1).step_by(2).filter(|n| n.ends_with(".swift")).map(|n| sources.join(n)));
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Wire methods the phones compile in and never send, each where it is
/// named. None now: the Mac's half of ov-184 took out the last, the move menu
/// in `TaskBoardModel.swift`. An entry added here must still be found, so it
/// goes when its use does.
const NAMED_BUT_NOT_SENT: [(&str, Method); 0] = [];

/// Each wire method a string literal in `files` names, with the first file
/// that names it, leaving out `NAMED_BUT_NOT_SENT`.
fn methods_named_in(files: &[std::path::PathBuf]) -> std::collections::BTreeMap<Method, String> {
    let mut found = std::collections::BTreeMap::new();
    let mut exempted = std::collections::BTreeSet::new();
    for path in files {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        for literal in text.split('"').skip(1).step_by(2) {
            let Some(method) = Method::parse(literal) else { continue };
            if NAMED_BUT_NOT_SENT.contains(&(name, method)) {
                exempted.insert((name, method));
                continue;
            }
            found.entry(method).or_insert_with(|| path.display().to_string());
        }
    }
    let stale: Vec<_> = NAMED_BUT_NOT_SENT.iter().filter(|e| !exempted.contains(e)).collect();
    let scanned_agentkit = files.iter().any(|f| f.ends_with("AgentKit/TaskBoardModel.swift"));
    assert!(!scanned_agentkit || stale.is_empty(), "no longer named, so drop them from NAMED_BUT_NOT_SENT: {stale:?}");
    found
}

/// **A task write planted in AgentKit's phone code is found** (ov-184).
///
/// The scan once read `apps/ios` only, and iOS's answer had moved into
/// AgentKit, so a write beside it went unseen by `no_phone_can_write_a_task`.
/// Here a generator that lists one AgentKit file on the watch's list, and
/// one that is in no list, which must stay unread.
#[test]
fn a_task_write_in_agentkit_s_phone_code_is_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Planted.swift"), "let m = \"task.create\"\n").unwrap();
    std::fs::write(dir.path().join("MacOnly.swift"), "let m = \"task.update\"\n").unwrap();
    let generator = "X = 1\nAGENTKIT_SOURCES = [\n    # \"MacOnly.swift\" is not here\n]\n\
                     WATCH_AGENTKIT_SOURCES = [\n    \"Planted.swift\",\n]\n\
                     WATCH_WIDGET_AGENTKIT_SOURCES = [\n]\n";
    let files = agentkit_the_phones_compile(generator, dir.path());
    assert_eq!(files, vec![dir.path().join("Planted.swift")]);
    let named = methods_named_in(&files);
    assert!(named.contains_key(&Method::TaskCreate), "the planted write went unseen: {named:?}");
    assert!(!named.contains_key(&Method::TaskUpdate), "a file in no list was read: {named:?}");

    // And the real lists reach the file iOS's answer is named in.
    let apps = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps");
    let generator = std::fs::read_to_string(apps.join("ios/generate-project.py")).unwrap();
    let real = agentkit_the_phones_compile(&generator, &apps.join("shared/AgentKit/Sources/AgentKit"));
    let shell: Vec<_> = real.into_iter().filter(|f| f.ends_with("ShellNavigation.swift")).collect();
    assert!(methods_named_in(&shell).contains_key(&Method::TaskNote), "iOS's answer is outside the scan");
}

/// Every wire method a phone sends has a route, and an arm in `dispatch`.
///
/// The arm, not only a declaration of one: each is called, against a runner
/// that refuses everything, and only `unknown method` fails it.
#[tokio::test]
async fn every_method_a_phone_sends_is_routed() {
    let phones = the_wire_methods_the_phones_name();
    assert!(
        phones.contains_key(&Method::TerminalAgentPrompt) && phones.len() > 20,
        "the scan found {phones:?}, so this test proves nothing"
    );
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    a_runner_that_refuses_everything(&socket).await;
    let mut session = Session::connect_local(&socket).await.expect("connect");

    let mut unrouted = Vec::new();
    for (&method, file) in &phones {
        let routed = match route(method) {
            Some(name) => is_routed(&mut session, name).await,
            None => false,
        };
        if !routed {
            unrouted.push(format!("{} (named in {file})", method.name()));
        }
    }
    assert!(unrouted.is_empty(), "a phone sends these and `dispatch` refuses them as unknown: {unrouted:#?}");
}

/// Every route `route` declares has an arm in `dispatch`, called the way an
/// app calls it, against a runner that refuses everything.
#[tokio::test]
async fn every_route_has_an_arm() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    a_runner_that_refuses_everything(&socket).await;
    let mut session = Session::connect_local(&socket).await.expect("connect");

    let mut missing = Vec::new();
    for &method in Method::ALL {
        if let Some(name) = route(method)
            && !is_routed(&mut session, name).await
        {
            missing.push(format!("{method:?} as {name}"));
        }
    }
    assert!(missing.is_empty(), "declared routes `dispatch` has no arm for: {missing:#?}");
}

/// The task writes the orchestrator owns (ov-184). The CLI makes them; a
/// phone, through this library, never can.
const PHONES_NEVER_WRITE_A_TASK: [Method; 5] =
    [Method::TaskCreate, Method::TaskUpdate, Method::TaskSetStatus, Method::TaskMove, Method::TaskBlock];

/// **No phone can create, edit, re-status, move or block a task** (ov-184).
///
/// Three ways a write could come back: a route declared for it, an arm in
/// `dispatch` under its wire name, and a phone source naming it. Each is
/// checked, so re-adding any one of them fails here. Answering stays, through
/// `task.note`, which `task_note_of` holds to answers.
#[tokio::test]
async fn no_phone_can_write_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    a_runner_that_refuses_everything(&socket).await;
    let mut session = Session::connect_local(&socket).await.expect("connect");

    for method in PHONES_NEVER_WRITE_A_TASK {
        assert_eq!(route(method), None, "{} has a phone route", method.name());
        assert!(!is_routed(&mut session, method.name()).await, "`dispatch` has an arm for {}", method.name());
    }
    let phones = the_wire_methods_the_phones_name();
    let named: Vec<_> = PHONES_NEVER_WRITE_A_TASK
        .iter()
        .filter_map(|m| phones.get(m).map(|file| format!("{} in {file}", m.name())))
        .collect();
    assert!(named.is_empty(), "a phone names a task write the orchestrator owns: {named:#?}");

    // The answer path is still routed, so the check above is not passing on
    // a session that refuses every name.
    assert!(is_routed(&mut session, "task.note").await, "answering lost its arm");
}

/// **A phone's answer still goes out; nothing else on a task does** (ov-184).
///
/// Through `dispatch`, as an app calls it: an `answer` reaches the runner
/// (this one refuses everything, so reaching it is a refusal, not a protocol
/// error), as the user, which is what wakes the agent waiting on it
/// (`Store::add_note_waking`). Every other kind stops here, before the wire.
#[tokio::test]
async fn a_phone_answer_reaches_the_runner_and_no_other_note_does() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    a_runner_that_refuses_everything(&socket).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let task = uuid::Uuid::now_v7().to_string();

    let answer = dispatch(&session, "task.note", &json!({ "task": task, "kind": "answer", "body": "Postgres" }))
        .await;
    assert!(matches!(answer, Err(SessionError::Refused { .. })), "the answer never reached the runner: {answer:?}");

    for kind in ["comment", "progress", "finding", "question", "decision", "status_change", "created"] {
        let note = dispatch(&session, "task.note", &json!({ "task": task, "kind": kind, "body": "x" })).await;
        assert!(matches!(note, Err(SessionError::Protocol(_))), "a {kind} note went out: {note:?}");
    }
}

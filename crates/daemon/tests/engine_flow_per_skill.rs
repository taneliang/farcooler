//! ov-1012: the flow the manager skill teaches, run by a real orchestrator pane and a person.
//!
//! The test plays the owner: it approves the drafted workflow on its card and nothing else. The orchestrator is a
//! pane the daemon starts (a stand-in agent that re-runs this test binary), so every write it makes passes the
//! daemon's admission as the `orchestrator` seat. It proposes the draft, moves the ticket, starts it, passes its
//! review at an orchestrator gate and queues it, each write built from the command the skill shows for that verb
//! (the test reads them out of the rendered SKILL.md: change a verb or drop a flag there and this goes red).
//! The engine, never the test, makes the worktree, builds, gates and pushes to the landing repository.
use std::{path::Path, sync::Arc};
use farcooler_daemon::landing_templates::{self as lt, Options};
use farcooler_daemon::skill_install::{Harness, render};
use farcooler_daemon::{rpc::RpcFactory, service::Service, watch::Watcher,
    workflow_runtime::{CodingCi, CodingLanding, CodingCommand, CodingConfig, CodingHarness}};
use farcooler_protocol::{v1::{self as pb, request, result}, workflow_commands::{Action, Mutation}};
use farcooler_store::models::{NoteKind, TaskStatus};
use farcooler_transport::{Client, HandshakeConfig, UnixListenerServer};
mod common;
use uuid::Uuid;

type Link = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn git(repo: &Path, args: &[&str]) -> String {
    let out = tokio::process::Command::new("/usr/bin/git").current_dir(repo).args(["-c", "core.hooksPath=/dev/null"]).args(args).output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().into()
}
async fn configured() -> (tempfile::TempDir, CodingConfig) {
    let dir = tempfile::Builder::new().prefix("ov971").tempdir_in("/tmp").unwrap();
    let repo = dir.path().join("selected");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "--quiet"]).await;
    git(&repo, &["-c", "user.name=Test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", "base"]).await;
    let base = git(&repo, &["rev-parse", "HEAD"]).await;
    git(&repo, &["update-ref", "refs/heads/workflow-target", &base]).await;
    // The landing target is a private bare repository; nothing here ever pushes to it.
    let bare = dir.path().join("private-remote.git");
    std::fs::create_dir(&bare).unwrap();
    git(&bare, &["init", "--bare", "--quiet"]).await;
    git(&repo, &["push", bare.to_str().unwrap(), "HEAD:refs/heads/main"]).await;
    let landing = CodingLanding { job: "landing-971".into(), repository: bare.canonicalize().unwrap(), target: "refs/heads/main".into(),
        config_digest: farcooler_workflow::digest(&std::fs::read(bare.join("config")).unwrap()) };
    // A CI stand-in that always reports the commit it is asked about as green, in the receipt the engine reads.
    let script = dir.path().join("ci.py");
    std::fs::write(&script, "import json, os, pathlib\ncandidate = os.environ['FARCOOLER_CANDIDATE']\npathlib.Path(os.environ['FARCOOLER_RECEIPT']).write_text(json.dumps(dict(binding=os.environ['FARCOOLER_OPERATION_BINDING'], repository='offline/repository', run=dict(id=73, name='ci', head_sha=candidate, status='completed', conclusion='success', html_url='fixture://ci/73', created_at='2026-01-01T00:00:00Z', event='push'))))\n").unwrap();
    let green_ci = CodingCommand { argv: vec!["/usr/bin/python3".into(), script.to_string_lossy().into_owned()],
        environment: [("PATH".to_string(), "/usr/bin:/bin".to_string())].into(), timeout_ms: 5000 };
    let build = CodingCommand { argv: vec!["/bin/sh".into(), "-c".into(), "echo built > result.txt && git add result.txt && git commit -q -m build".into()],
        environment: ["PATH=/usr/bin:/bin", "GIT_AUTHOR_NAME=Build", "GIT_AUTHOR_EMAIL=b@localhost", "GIT_COMMITTER_NAME=Build", "GIT_COMMITTER_EMAIL=b@localhost"]
            .iter().map(|kv| { let (k, v) = kv.split_once('=').unwrap(); (k.to_string(), v.to_string()) }).collect(), timeout_ms: 20000 };
    let config = CodingConfig { repository: repo.canonicalize().unwrap(), base, landing_ref: "refs/heads/workflow-target".into(), landing: Some(landing), pull_request: None,
        harness: CodingHarness::Command(build), checks: vec![], reviewer: None,
        ci: CodingCi { repository: "offline/repository".into(), workflow: "ci".into(), event: "push".into(), command: green_ci }, };
    (dir, config)
}

struct Host { service: Arc<Service>, workspace: Uuid, socket: std::path::PathBuf, stop: Arc<tokio::sync::Notify>, server: tokio::task::JoinHandle<()> }
impl Host {
    async fn open(root: &Path) -> Self {
        let service = Arc::new(Service::open_in(root.join("state")).await.unwrap());
        // Registered as a real checkout, so the orchestrator's home and pane have somewhere to be.
        service.add_root(root).await.expect("root");
        let repo = service.register_repository(&root.join("selected")).await.expect("registered");
        let workspace = service.store.ensure_main_workspace(repo.id).unwrap().id;
        service.start_workflows().unwrap();
        let watcher = Watcher::new(service.clone());
        let socket = root.join("d.sock");
        let listener = UnixListenerServer::bind(&socket).unwrap();
        let stop = Arc::new(tokio::sync::Notify::new());
        let (svc, end, rpc_stop) = (service.clone(), stop.clone(), stop.clone());
        let server = tokio::spawn(async move {
            tokio::select! {
                result = listener.serve_with_pid(move |preamble, pid| Some((HandshakeConfig { daemon_version: "ov971".into() },
                    RpcFactory::new(svc.clone(), watcher.clone(), rpc_stop.clone(), farcooler_daemon::sessions::peer_from_preamble(preamble.as_ref())?).with_socket_caller(pid)))) => result.unwrap(),
                _ = end.notified() => (),
            }
        });
        Self { service, workspace, socket, stop, server }
    }
    async fn connect(&self) -> Link { Client::connect(&self.socket, "ov971", "1").await.unwrap() }
    async fn close(self) {
        self.stop.notify_one();
        self.server.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), self.service.stop_workflows()).await.unwrap().unwrap();
    }
}
async fn call(link: &mut Link, workspace: Uuid, action: Action) -> serde_json::Value {
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(workspace.as_bytes()));
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: serde_json::to_string(&action).unwrap() }));
    let result::Value::WorkflowReply(reply) = link.call(req).await.unwrap().value.unwrap() else { panic!("wrong result") };
    serde_json::from_str(&reply.json).unwrap()
}
/// A note on the card, as `actor` says it.
async fn say(link: &mut Link, card: Uuid, kind: pb::TaskNoteKind, actor: &str, body: &str) {
    assert!(note(link, card, kind, actor, body, None).await, "the note was refused");
}
/// Whether the runner took the note.
async fn note(link: &mut Link, card: Uuid, kind: pb::TaskNoteKind, actor: &str, body: &str, supersedes: Option<Uuid>) -> bool {
    let mut req = farcooler_transport::request("task.note");
    req.payload = Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend { task_id: bytes::Bytes::copy_from_slice(card.as_bytes()), kind: kind as i32,
        body: body.into(), extra_json: String::new(), supersedes: supersedes.map(|s| bytes::Bytes::copy_from_slice(s.as_bytes())), actor: actor.into() }));
    matches!(link.call(req).await, Ok(r) if r.value.is_some())
}
/// The card's latest question, and the words at its end that approve it.
fn asked(host: &Host, card: Uuid) -> (farcooler_store::models::TaskNote, String) {
    let q = host.service.store.notes_for(card, Some(NoteKind::Question)).unwrap().pop().expect("a question");
    let at = q.body.rfind("answer `approve ").expect("the approving words") + "answer `".len();
    let words = q.body[at..].split('`').next().unwrap().to_string();
    (q, words)
}
fn pin(host: &Host) -> Option<String> { host.service.store.workflow_configuration(&host.workspace.to_string()).unwrap().map(|c| c.pin) }

async fn as_a_person(name: &str) -> bool {
    common::as_a_person("ov971-person", name, std::time::Duration::from_secs(120)).await
}
async fn settles(host: &Host, card: Uuid, to: TaskStatus) {
    for _ in 0..200 {
        if host.service.store.get_task(card).unwrap().status == to { return; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let said: Vec<_> = host.service.store.notes_for(card, Some(NoteKind::Finding)).unwrap().into_iter().map(|n| n.body).collect();
    panic!("the card never reached {to:?}; it is {:?}: {said:?}", host.service.store.get_task(card).unwrap().status);
}



/// The skill as the daemon renders it for Claude.
fn skill() -> String {
    render(Harness::Claude, "farcooler").into_iter().find(|f| f.relative.ends_with("SKILL.md")).expect("a SKILL.md").contents
}
/// The flags (and the placeholder each takes) of the one command the skill shows for `workflow <verb>`.
fn taught(skill: &str, verb: &str) -> std::collections::BTreeMap<String, String> {
    let prefix = format!("farcooler workflow {verb} ");
    let mut fenced = false;
    for line in skill.lines() {
        if line.trim_start().starts_with("```") { fenced = !fenced; continue; }
        if !fenced || !line.starts_with(&prefix) { continue; }
        let words: Vec<&str> = line[prefix.len()..].split_whitespace().collect();
        let mut flags = std::collections::BTreeMap::new();
        let mut at = 0;
        while at < words.len() {
            if words[at].starts_with("--") {
                let takes = words.get(at + 1).is_some_and(|w| !w.starts_with("--"));
                flags.insert(words[at].to_string(), if takes { words[at + 1].to_string() } else { String::new() });
                at += 1 + usize::from(takes);
            } else { at += 1; }
        }
        return flags;
    }
    panic!("the skill shows no `workflow {verb}` command")
}
/// The skill's command for `verb`, which must carry every flag the write needs.
fn command(skill: &str, verb: &str, needs: &[&str]) -> std::collections::BTreeMap<String, String> {
    let flags = taught(skill, verb);
    for need in needs { assert!(flags.contains_key(*need), "the skill's `workflow {verb}` has no {need}: {flags:?}"); }
    flags
}
/// The write the skill describes: the revision and epoch `get` gave, one more than the largest source sequence.
fn at(got: &serde_json::Value, id: &str) -> Mutation {
    let sequence = got["state"]["source_sequences"].as_object().into_iter().flat_map(|m| m.values()).filter_map(|v| v.as_u64()).max().map_or(1, |m| m + 1);
    Mutation { id: id.into(), revision: got["state"]["revision"].as_u64().unwrap(), epoch: got["state"]["epoch"].as_u64().unwrap(), sequence }
}
fn ok(receipt: &serde_json::Value) { assert!(receipt.pointer("/receipt/result/Ok").is_some(), "{receipt}"); }
async fn waits_for(link: &mut Link, ws: Uuid, key: &str, what: &str, done: impl Fn(Option<&serde_json::Value>, &serde_json::Value) -> bool) -> serde_json::Value {
    for _ in 0..900 {
        let got = call(link, ws, Action::Get).await;
        if done(got["state"]["items"].get(key), &got) { return got; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let last = call(link, ws, Action::Get).await;
    let slim: Vec<_> = last["state"]["items"].as_object().unwrap().iter().map(|(k, v)| format!("{k}: phase {} stage {} failure {}", v["phase"], v["stage"], v["failure"])).collect();
    panic!("never reached {what}: {slim:?}");
}
fn log(out: &str, line: &str) {
    use std::io::Write;
    writeln!(std::fs::OpenOptions::new().create(true).append(true).open(out).unwrap(), "{line}").unwrap();
}

/// The orchestrator pane. The daemon starts this test binary as the stand-in agent (see the test below); run
/// any other way it has nothing to do.
#[tokio::test]
async fn the_orchestrator_pane_runs_the_skills_commands() {
    let Ok(socket) = std::env::var("OV1012_SOCKET") else { return };
    let (ws, key, out) = (std::env::var("OV1012_WS").unwrap().parse::<Uuid>().unwrap(), std::env::var("OV1012_KEY").unwrap(), std::env::var("OV1012_OUT").unwrap());
    let skill = skill();
    let mut link: Link = Client::connect(&socket, "orchestrator-pane", "1").await.unwrap();

    // Section 4: no workflow yet. The skill sends the orchestrator to `workflow-draft`, which proposes; it never publishes.
    assert!(skill.contains("workspace workflow-draft") && skill.contains("--orchestrator-gate"), "the skill no longer teaches drafting");
    let (definition, adapter): (farcooler_workflow::Definition, farcooler_protocol::workflow_config::AdapterConfiguration) =
        serde_json::from_str(&std::fs::read_to_string(std::env::var("OV1012_BUNDLE").unwrap()).unwrap()).unwrap();
    let draft = call(&mut link, ws, Action::Propose { reason: "One ticket at a time, gated by true and the orchestrator's word.".into(), definition, adapter: Box::new(adapter) }).await;
    assert_eq!(draft["draft"]["state"], "pending", "{draft}");
    log(&out, "proposed");

    // A person approves it. Until then the board has no configuration.
    let mut got = call(&mut link, ws, Action::Get).await;
    for _ in 0..1200 {
        if got["configuration"]["pin"].is_string() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        got = call(&mut link, ws, Action::Get).await;
    }
    let start_pin = got["configuration"]["pin"].as_str().expect("the owner's approval never published").to_string();
    log(&out, "published");

    // Section 3: a card made on a published board is tracked by itself. Move it, start it, review it, queue it.
    let item = &got["state"]["items"][&key];
    assert_eq!(item["definition"]["source"], "ticket/v1", "tracked and at rest: {got}");
    command(&skill, "move", &["--to", "--run", "--pin", "--command-id", "--revision", "--epoch", "--sequence"]);
    ok(&call(&mut link, ws, Action::Move { mutation: at(&got, "move-1"), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(), to: "todo".into() }).await);
    log(&out, "workflow move");

    let got = call(&mut link, ws, Action::Get).await;
    let item = &got["state"]["items"][&key];
    command(&skill, "start", &["--start-pin", "--run", "--pin", "--worktree", "--branch", "--command-id", "--revision", "--epoch", "--sequence"]);
    ok(&call(&mut link, ws, Action::Start { mutation: at(&got, "start-1"), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(), start_pin,
        models: Default::default(), with: vec![],
        worktree: Some(farcooler_protocol::workflow_commands::NamedTree { name: format!("{key}-result-file"), branch: format!("{key}-result-file"), adopt: false }) }).await);
    log(&out, "workflow start");

    // The engine builds and gates; the run waits at the orchestrator's gate for a verdict on the evidence `get` shows.
    let got = waits_for(&mut link, ws, &key, "the review", |i, _| i.is_some_and(|i| i["phase"] == "WaitingReview")).await;
    let item = &got["state"]["items"][&key];
    command(&skill, "review", &["--run", "--pin", "--passed", "--evidence", "--command-id", "--revision", "--epoch", "--sequence"]);
    ok(&call(&mut link, ws, Action::Review { mutation: at(&got, "review-1"), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(),
        evidence: serde_json::from_value(item["last_evidence"].clone()).expect("last_evidence is what get gives"), passed: true, reason: None }).await);
    log(&out, "workflow review");

    // After the last gate the ticket waits to be queued into the landing group; then the engine lands it.
    let got = waits_for(&mut link, ws, &key, "the landing group", |i, _| i.is_some_and(|i| i["phase"] == "WaitingJoin")).await;
    let item = &got["state"]["items"][&key];
    command(&skill, "queue", &["--run", "--pin", "--command-id", "--revision", "--epoch", "--sequence"]);
    ok(&call(&mut link, ws, Action::Queue { mutation: at(&got, "queue-1"), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into() }).await);
    log(&out, "workflow queue");
    waits_for(&mut link, ws, &key, "landed", |i, g| i.map_or(g["state"]["archive"].get(&key).is_some(), |i| i["phase"] == "Done")).await;
    log(&out, "landed");
    std::thread::sleep(std::time::Duration::from_secs(600));
}

#[tokio::test]
async fn a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches() {
    if as_a_person("a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches").await { return; }
    let (dir, config) = configured().await;
    let bare = dir.path().join("private-remote.git");
    let landed_before = git(&bare, &["rev-parse", "refs/heads/main"]).await;
    farcooler_daemon::service::stub_agents_in_this_process();
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);
    let tmux_socket = host.service.tmux.socket().to_string();
    struct Reap(String);
    impl Drop for Reap { fn drop(&mut self) { farcooler_tmux::reap_server(&self.0); } }
    let _reap = Reap(tmux_socket);

    // The board has a card, and the orchestrator's draft: one ticket at a time, behind `true` and the orchestrator's word.
    let ticket = host.service.store.create_task(ws, "Add a result file", farcooler_store::models::Actor::User).unwrap();
    let draft = lt::template("ticket-push", &config, &["true".to_string()], &[lt::Extra::Orchestrator], &Options::default()).unwrap();
    let bundle_file = dir.path().join("bundle.json");
    std::fs::write(&bundle_file, serde_json::to_string(&(&draft.definition, &draft.adapter)).unwrap()).unwrap();

    // The stand-in agent is this test binary, started in the orchestrator's pane.
    let out = dir.path().join("orchestrator.out");
    let stand_in = dir.path().join("orchestrator-stand-in");
    std::fs::write(&stand_in, format!("#!/bin/sh\nOV1012_SOCKET='{}' OV1012_WS='{}' OV1012_KEY='{}' OV1012_BUNDLE='{}' OV1012_OUT='{}' exec '{}' --exact the_orchestrator_pane_runs_the_skills_commands --nocapture > '{}.log' 2>&1\n",
        host.socket.display(), ws, ticket.key, bundle_file.display(), out.display(), std::env::current_exe().unwrap().display(), out.display())).unwrap();
    { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap(); }
    // SAFETY: set before the pane starts, and nothing else here reads the environment concurrently.
    unsafe { std::env::set_var("FARCOOLER_STAND_IN_AGENT", &stand_in) };
    let mut start = farcooler_transport::request("workspace.start_orchestrator");
    start.target_resource_id = Some(bytes::Bytes::copy_from_slice(ws.as_bytes()));
    start.payload = Some(request::Payload::WorkspaceStartOrchestrator(pb::WorkspaceStartOrchestrator { harness: "claude".into(), replace: false, handoff_task: String::new() }));
    link.call(start).await.expect("the orchestrator pane starts");

    // The person's part: the draft's card asks, and the owner answers with the words that approve it.
    let said = |what: &str| std::fs::read_to_string(&out).unwrap_or_default().lines().any(|l| l == what);
    let mut card = None;
    for _ in 0..1200 {
        let listed = call(&mut link, ws, Action::Drafts { draft: None }).await;
        if let Some(c) = listed["drafts"][0]["card"].as_str() { card = Some(c.parse::<Uuid>().unwrap()); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let pane_log = || std::fs::read_to_string(format!("{}.log", out.display())).unwrap_or_default();
    let card = card.unwrap_or_else(|| panic!("the orchestrator never proposed: {}", pane_log()));
    assert_eq!(pin(&host), None, "a draft publishes nothing");
    let words = asked(&host, card).1;
    say(&mut link, card, pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card, TaskStatus::Done).await;

    // The orchestrator does the rest; the test only waits for it to say the ticket landed.
    for _ in 0..1800 {
        if said("landed") { break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(said("landed"), "the orchestrator stopped short: {:?}\n{}", std::fs::read_to_string(&out), pane_log());
    for step in ["proposed", "published", "workflow move", "workflow start", "workflow review", "workflow queue"] { assert!(said(step), "{step} never ran"); }
    let landed = git(&bare, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(landed, landed_before, "the engine pushed the ticket's commit to the landing repository");
    assert_eq!(git(&bare, &["show", "refs/heads/main:result.txt"]).await, "built");
    assert_eq!(git(&bare, &["log", "-1", "--format=%s", "refs/heads/main"]).await, "build");
    drop(link);
    host.close().await;
}

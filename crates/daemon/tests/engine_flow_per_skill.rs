//! ov-1012: the flow the manager skill teaches, end to end through the daemon's wire commands. The test plays the
//! owner and approves a drafted workflow on its card; then a ticket goes track -> move -> start -> review ->
//! landed, each write built the way the skill says (read `get` first; revision, epoch, run and pin from it).
//! The engine, never the test, makes the worktree, builds, runs the gate and pushes to the landing repository.
//! The review is a person's gate here: an orchestrator's gate needs an orchestrator pane's seat, which a
//! test process can't hold, and the wire rule for that principal is covered in `wire_commands`.
use std::{path::Path, sync::Arc};
use farcooler_daemon::landing_templates::{self as lt, Options};
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
    let command = || CodingCommand { argv: vec!["/usr/bin/true".into()], environment: [("PATH".to_string(), "/usr/bin:/bin".to_string())].into(), timeout_ms: 5000 };
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
        let git_dir = root.join("selected/.git").canonicalize().unwrap().to_string_lossy().into_owned();
        let repo_root = service.store.create_repository_root(service.host_id, "/isolated", 0).unwrap();
        let repo = service.store.create_repository(service.host_id, repo_root.id, "isolated", &git_dir, "").unwrap();
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
fn bundle(config: &CodingConfig, template: &str, gate: &str) -> lt::Bundle {
    lt::template(template, config, &[gate.to_string()], &[], &Options::default()).unwrap()
}
async fn propose(link: &mut Link, workspace: Uuid, b: lt::Bundle, reason: &str) -> serde_json::Value {
    call(link, workspace, Action::Propose { reason: reason.into(), definition: b.definition, adapter: Box::new(b.adapter) }).await
}
fn card_of(reply: &serde_json::Value) -> Uuid { reply["draft"]["card"].as_str().unwrap().parse().unwrap() }
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


/// What `workflow get --json` hands the orchestrator: the revision and epoch to write against.
async fn fresh(link: &mut Link, ws: Uuid, key: &str) -> serde_json::Value {
    let got = call(link, ws, Action::Get).await;
    assert!(got["state"]["items"].get(key).is_some() || key.is_empty(), "{got}");
    got
}
fn at(got: &serde_json::Value, id: &str, sequence: u64) -> Mutation {
    Mutation { id: id.into(), revision: got["state"]["revision"].as_u64().unwrap(), epoch: got["state"]["epoch"].as_u64().unwrap(), sequence }
}
fn ok(receipt: &serde_json::Value) { assert!(receipt.pointer("/receipt/result/Ok").is_some(), "{receipt}"); }
async fn waits_for(link: &mut Link, ws: Uuid, key: &str, what: &str, done: impl Fn(Option<&serde_json::Value>, &serde_json::Value) -> bool) -> serde_json::Value {
    for _ in 0..300 {
        let got = call(link, ws, Action::Get).await;
        if done(got["state"]["items"].get(key), &got) { return got; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let last = call(link, ws, Action::Get).await;
    let slim: Vec<_> = last["state"]["items"].as_object().unwrap().iter().map(|(k, v)| format!("{k}: phase {} stage {} failure {}", v["phase"], v["stage"], v["failure"])).collect();
    panic!("never reached {what}: {slim:?}");
}

#[tokio::test]
async fn a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches() {
    if as_a_person("a_ticket_goes_from_draft_to_landed_on_the_commands_the_skill_teaches").await { return; }
    let (dir, config) = configured().await;
    let bare = dir.path().join("private-remote.git");
    let landed_before = git(&bare, &["rev-parse", "refs/heads/main"]).await;
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);

    // Section 4: no workflow yet, so the orchestrator drafts one and a person approves it on the card.
    let draft = lt::template("ticket-push", &config, &["true".to_string()], &[lt::Extra::Person("owner".into())], &Options::default()).unwrap();
    let reply = propose(&mut link, ws, draft, "One ticket at a time, gated by true and the owner's word.").await;
    assert_eq!(pin(&host), None, "a draft publishes nothing");
    let words = asked(&host, card_of(&reply)).1;
    say(&mut link, card_of(&reply), pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card_of(&reply), TaskStatus::Done).await;
    let configuration_pin = pin(&host).expect("approving publishes");

    // Section 3: a card made on a published board is tracked by itself; here the runner's tracker is nudged
    // the way task.create does.
    let ticket = host.service.store.create_task(ws, "Add a result file", farcooler_store::models::Actor::User).unwrap();
    host.service.track_tickets(Some(ticket.id)).await;
    let key = ticket.key.clone();
    let got = fresh(&mut link, ws, &key).await;
    let item = &got["state"]["items"][&key];
    assert_eq!(item["definition"]["source"], "ticket/v1", "tracked, no run yet: {got}");

    // move: Backlog -> Todo, then start, each with the run and pin read from a fresh get.
    ok(&call(&mut link, ws, Action::Move { mutation: at(&got, "move", 2), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(), to: "todo".into() }).await);
    let got = fresh(&mut link, ws, &key).await;
    let item = &got["state"]["items"][&key];
    ok(&call(&mut link, ws, Action::Start { mutation: at(&got, "start", 3), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(), start_pin: configuration_pin.clone(),
        models: Default::default(), with: vec![],
        worktree: Some(farcooler_protocol::workflow_commands::NamedTree { name: "result-file".into(), branch: "ov-result-file".into(), adopt: false }) }).await);

    // The engine builds and gates; the run stops for the verdict.
    let got = waits_for(&mut link, ws, &key, "the review", |i, _| i.is_some_and(|i| i["phase"] == "WaitingReview")).await;
    let item = &got["state"]["items"][&key];
    let evidence = item["last_evidence"].clone();
    assert!(evidence.is_object(), "the verdict is written against the evidence get shows: {got}");
    // review --passed; the engine, not the caller, lands it.
    ok(&call(&mut link, ws, Action::Review { mutation: at(&got, "review", 4), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into(),
        evidence: serde_json::from_value(evidence).unwrap(), passed: true, reason: None }).await);
    // The verdict passes the ticket to its landing group, where it waits for the word to queue.
    let got = waits_for(&mut link, ws, &key, "the landing group", |i, _| i.is_some_and(|i| i["phase"] == "WaitingJoin")).await;
    let item = &got["state"]["items"][&key];
    ok(&call(&mut link, ws, Action::Queue { mutation: at(&got, "queue", 5), item: key.clone(),
        run: item["run"].as_str().unwrap().into(), pin: item["pin"].as_str().unwrap().into() }).await);
    let got = waits_for(&mut link, ws, &key, "landed", |i, g| i.map_or(g["state"]["archive"].get(&key).is_some(), |i| i["phase"] == "Done")).await;
    let _ = got;

    let landed = git(&bare, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(landed, landed_before, "the engine pushed the ticket's commit to the landing repository");
    assert_eq!(git(&bare, &["show", "refs/heads/main:result.txt"]).await, "built");
    assert_eq!(git(&bare, &["log", "-1", "--format=%s", "refs/heads/main"]).await, "build");
    drop(link);
    host.close().await;
}

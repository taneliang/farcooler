//! Who a caller is, the lead's gate on the orchestrator, a rep's refusals,
//! and the shared account (ov-494), through `Rpc` where a client reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response};
use farcooler_store::models::Terminal;

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

/// A runner with an orchestrator on Main and an agent beside it, and Bob,
/// a member whose phone is `phone-b`.
struct Runner {
    dir: ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
    orchestrator: Terminal,
    agent: Terminal,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let orchestrator = svc.start_orchestrator(workspace, "claude", false, None).await.expect("an orchestrator");
    let agent = svc.create_terminal(worktree.id, "Agent 2", "claude").await.expect("an agent");
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    Runner { dir, svc, workspace, orchestrator, agent }
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

/// The refusal as `<code word>:<what>`, or `None` for a result: the code is
/// pinned with the word, since the apps switch on the code (B2).
async fn refused(factory: &RpcFactory, req: Request) -> Option<String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => None,
        Some(response::Outcome::Error(e)) => Some(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

const NOT_LEAD: &str = "not-yours:not_lead";
const NOT_YOURS: &str = "not-yours:not_yours";
const MESSENGER: &str = "resource-conflict:rep_messenger";

/// Whether a call got past the people rules: answered, or refused for some
/// other reason by its own handler.
fn passed(outcome: &Option<String>) -> bool {
    !outcome.as_deref().is_some_and(|o| o.starts_with("not-yours:") || o == "invalid-argument:actor")
}

/// `typing_into`, and the calls that stop, restart, remove or recast `pane`.
fn acting_on(pane: Uuid) -> Vec<Request> {
    let mut all = typing_into(pane);
    for method in ["terminal.stop", "terminal.restart", "terminal.remove", "terminal.set_role"] {
        all.push(Request { method: method.into(), target_resource_id: Some(id_bytes(pane)), ..Default::default() });
    }
    all
}

/// Every way a client puts words in front of an agent, aimed at `pane`.
fn typing_into(pane: Uuid) -> Vec<Request> {
    let id = || id_bytes(pane);
    let text = vec![pb::AgentPromptBlock { content: Some(pb::agent_prompt_block::Content::Text("ship it".into())) }];
    let prompt = || Some(request::Payload::AgentPrompt(pb::AgentPrompt { terminal_id: id(), blocks: text.clone(), hold_behind_dialog: false }));
    let call = |method: &str, payload| Request {
        method: method.into(),
        target_resource_id: Some(id()),
        payload,
        ..Default::default()
    };
    vec![
        call("terminal.write", Some(request::Payload::TerminalWrite(pb::TerminalWrite { payload: b"y\r".to_vec().into() }))),
        call("terminal.paste_file", Some(request::Payload::TerminalFilePut(pb::TerminalFilePut::default()))),
        call("terminal.draft_prompt", prompt()),
        call("terminal.tell", prompt()),
        call("terminal.compose", prompt()),
        call("terminal.interrupt", Some(request::Payload::AgentCancel(pb::AgentCancel { terminal_id: id() }))),
        call("terminal.send_now", Some(request::Payload::AgentCancel(pb::AgentCancel { terminal_id: id() }))),
        call("terminal.bring_draft", Some(request::Payload::BringDraft(pb::BringDraft { terminal_id: id(), ..Default::default() }))),
        call("terminal.draft_withdraw", Some(request::Payload::DraftWithdraw(pb::DraftWithdraw { terminal_id: id(), ..Default::default() }))),
        call("terminal.agent_answer", Some(request::Payload::AgentAnswer(pb::AgentAnswer { terminal_id: id(), ..Default::default() }))),
    ]
}

/// Only the lead, and the owner always (R-54), act on the orchestrator, by
/// any of the ways a client can: typing, and stopping, restarting, removing
/// or recasting it. Other panes are anyone's with Control. The refusal has
/// its own code, never resource-conflict's "answered elsewhere".
#[tokio::test]
async fn only_the_lead_and_the_owner_act_on_the_orchestrator() {
    let r = runner().await;
    r.svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    let owner = factory(&r.svc, None, Scope::HostAdmin);
    let owners_phone = factory(&r.svc, Some("phone-o"), Scope::Control);
    let bob = factory(&r.svc, Some("phone-b"), Scope::Control);
    let carol = factory(&r.svc, Some("phone-c"), Scope::Control);
    for req in acting_on(r.orchestrator.id) {
        assert_eq!(refused(&bob, req.clone()).await.as_deref(), Some(NOT_LEAD), "{}: Bob isn't the lead", req.method);
    }
    for req in typing_into(r.orchestrator.id) {
        let method = req.method.clone();
        assert!(passed(&refused(&owner, req.clone()).await), "{method}: the owner leads by default");
        assert!(passed(&refused(&owners_phone, req).await), "{method}: a device never bound is the owner's");
    }
    // The owner's lane, not the orchestrator: Bob answers and stops it, and
    // types nothing into it (ov-716 rule 5, `people_team_tests`).
    for req in typing_into(r.agent.id) {
        let outcome = refused(&bob, req.clone()).await;
        match req.method.as_str() {
            "terminal.agent_answer" | "terminal.interrupt" => assert!(passed(&outcome), "{}: {outcome:?}", req.method),
            _ => assert_eq!(outcome.as_deref(), Some(NOT_YOURS), "{}: the owner's lane", req.method),
        }
    }

    r.svc.store.set_workspace_lead(r.workspace, Some("bob")).unwrap();
    for req in typing_into(r.orchestrator.id) {
        let method = req.method.clone();
        assert!(passed(&refused(&bob, req.clone()).await), "{method}: Bob leads now");
        assert!(passed(&refused(&owner, req.clone()).await), "{method}: the owner always may (R-54)");
        assert_eq!(refused(&carol, req).await.as_deref(), Some(NOT_LEAD), "{method}: Carol doesn't lead");
    }

    // Removing Bob releases his phone: nobody's, never the owner's, and it
    // writes nothing at all (ov-712).
    r.svc.store.remove_member("bob").unwrap();
    for req in acting_on(r.orchestrator.id) {
        assert_eq!(refused(&bob, req.clone()).await.as_deref(), Some(NOT_YOURS), "{}: a released phone", req.method);
    }
}

/// A rep is acted on by its own member and the owner, never by another
/// member or a released device, by any of the ways a client can.
#[tokio::test]
async fn only_its_member_and_the_owner_act_on_a_rep() {
    let r = runner().await;
    r.svc.store.set_member("alice", "Alice", &["phone-a".to_string()]).unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "alice").unwrap();
    let alice = factory(&r.svc, Some("phone-a"), Scope::Control);
    let bob = factory(&r.svc, Some("phone-b"), Scope::Control);
    let owner = factory(&r.svc, None, Scope::HostAdmin);
    for req in acting_on(r.agent.id) {
        assert_eq!(refused(&bob, req.clone()).await.as_deref(), Some(NOT_YOURS), "{}: not Bob's rep", req.method);
    }
    for req in typing_into(r.agent.id) {
        let method = req.method.clone();
        assert!(passed(&refused(&alice, req.clone()).await), "{method}: Alice's own");
        assert!(passed(&refused(&owner, req).await), "{method}: the owner's runner");
    }
    r.svc.store.set_member("alice", "Alice", &[]).unwrap();
    for req in typing_into(r.agent.id) {
        assert_eq!(refused(&alice, req.clone()).await.as_deref(), Some(NOT_YOURS), "{}: a dropped device", req.method);
    }
}

/// A caller running in a rep's pane, told by its pid and not by what it
/// claims, writes comments and messages only as that pane: as the owner or
/// the orchestrator it is refused `actor`, and an answer, or acting on any
/// pane, its own included, is no messenger's (`rep_messenger`). The same
/// claims from any other process pass.
#[tokio::test]
async fn a_reps_pane_speaks_only_as_itself() {
    let r = runner().await;
    let task = r.svc.store.create_task(r.workspace, "Dark mode", Actor::Manager).unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    assert!(pane_pid > 1, "tmux named the pane's process");
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6591);
    let elsewhere = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    let note = |actor: &str, kind: pb::TaskNoteKind| Request {
        method: "task.note".into(),
        payload: Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
            task_id: id_bytes(task.id),
            kind: kind as i32,
            body: "Yes, ship it".into(),
            actor: actor.into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    let own = format!("agent:{}", r.agent.id);
    for actor in ["user", "manager", "", &format!("agent:{}", r.orchestrator.id)] {
        assert_eq!(refused(&in_rep, note(actor, pb::TaskNoteKind::Answer)).await.as_deref(), Some(MESSENGER), "as {actor:?}");
        assert_eq!(refused(&in_rep, note(actor, pb::TaskNoteKind::Comment)).await.as_deref(), Some("invalid-argument:actor"), "as {actor:?}");
    }
    assert_eq!(refused(&in_rep, note(&own, pb::TaskNoteKind::Answer)).await.as_deref(), Some("resource-conflict:rep_messenger"));
    assert_eq!(refused(&in_rep, note(&own, pb::TaskNoteKind::Comment)).await, None, "as itself it comments");
    assert_eq!(refused(&elsewhere, note("user", pb::TaskNoteKind::Answer)).await, None, "the owner answers");
    let message = Request {
        method: "message.send".into(),
        payload: Some(request::Payload::MessageSend(pb::MessageSend {
            to: "orchestrator".into(),
            text: "Bob says ship it".into(),
            actor: "manager".into(),
            task: task.key.clone(),
            workspace_id: id_bytes(r.workspace),
        })),
        ..Default::default()
    };
    assert_eq!(refused(&in_rep, message).await.as_deref(), Some("invalid-argument:actor"), "not as the orchestrator");
    let answer = Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(pb::AgentAnswer {
            terminal_id: id_bytes(r.orchestrator.id),
            request_id: "hook-ask:1".into(),
            option_id: "allow".into(),
            actor: "user".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(refused(&in_rep, answer).await.as_deref(), Some(MESSENGER), "not the owner's answer");

    // Nor does it act on any pane, as the owner it would pass for.
    let own_answer = Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(pb::AgentAnswer {
            terminal_id: id_bytes(r.orchestrator.id),
            request_id: "hook-ask:1".into(),
            option_id: "allow".into(),
            actor: own.clone(),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(refused(&in_rep, own_answer).await.as_deref(), Some(MESSENGER), "the orchestrator's prompt");
    for req in acting_on(r.orchestrator.id).into_iter().filter(|q| q.method != "terminal.agent_answer") {
        assert_eq!(refused(&in_rep, req.clone()).await.as_deref(), Some(MESSENGER), "{} into the orchestrator", req.method);
        assert!(passed(&refused(&elsewhere, req).await), "the owner elsewhere still may");
    }
    for req in acting_on(r.agent.id) {
        assert_eq!(refused(&in_rep, req.clone()).await.as_deref(), Some(MESSENGER), "{}: its own pane", req.method);
    }
}

/// A rep only relays, and changes the plan for its member (R-66, ov-527,
/// ov-736). From its pane, every write `task dispatch` makes, `worktree
/// create` and the rest of the task layer's writes are refused
/// `rep_messenger`, in plain words; the board's reads, a message to the
/// orchestrator and a comment pass. From any other process
/// the same calls are left to their handlers.
#[tokio::test]
async fn a_reps_pane_only_reads_messages_and_comments() {
    use farcooler_transport::Handler as _;
    let r = runner().await;
    let task = r.svc.store.create_task(r.workspace, "Dark mode", Actor::Manager).unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6592);
    let elsewhere = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    let bare = |method: &str| Request { method: method.into(), ..Default::default() };
    let note = |kind: pb::TaskNoteKind| Request {
        method: "task.note".into(),
        payload: Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
            task_id: id_bytes(task.id),
            kind: kind as i32,
            body: "Bob would like it by Friday".into(),
            actor: format!("agent:{}", r.agent.id),
            ..Default::default()
        })),
        ..Default::default()
    };

    let mut writes: Vec<Request> = [
        "worktree.create", "terminal.create", "task.update", "task.set_status", "task.create", "member.set",
        "workspace.set_settings", "rep.start", "terminal.remove", "pr.ready", "workspace.rename",
    ]
    .into_iter()
    .map(bare)
    .collect();
    writes.extend([pb::TaskNoteKind::Progress, pb::TaskNoteKind::Question, pb::TaskNoteKind::Finding].map(note));
    for req in writes {
        assert_eq!(refused(&in_rep, req.clone()).await.as_deref(), Some(MESSENGER), "{} from a rep", req.method);
        assert_ne!(refused(&elsewhere, req.clone()).await.as_deref(), Some(MESSENGER), "{} elsewhere", req.method);
    }
    // In words a person reads, never "resource version is stale".
    let Some(response::Outcome::Error(e)) = in_rep.handle(bare("task.create")).await.outcome else { panic!("refused") };
    assert_eq!(Some(e.message.as_str()), farcooler_core::error::sentence("rep_messenger"));

    for req in ["plan.get", "plan.events", "task.list", "task.get_by_key", "member.list", "workspace.list", "host.get"].map(bare) {
        assert_ne!(refused(&in_rep, req.clone()).await.as_deref(), Some(MESSENGER), "{}: a read", req.method);
    }
    // The plan changes it makes for its member are `people_rep_plan_tests`'.
    assert_eq!(refused(&in_rep, note(pb::TaskNoteKind::Comment)).await, None, "a comment");
    let message = Request {
        method: "message.send".into(),
        payload: Some(request::Payload::MessageSend(pb::MessageSend {
            to: "orchestrator".into(),
            text: "Bob asks for dark mode".into(),
            actor: format!("agent:{}", r.agent.id),
            task: task.key.clone(),
            workspace_id: id_bytes(r.workspace),
        })),
        ..Default::default()
    };
    assert_eq!(refused(&in_rep, message).await, None, "a message to the orchestrator");
}

/// The allow-list, whole: a rep's pane may call these methods and no other,
/// and `task.note` only as a comment. A method added later is refused until
/// it's named in `rep_may`. The plan changes it makes for its member carry a
/// payload `rep_may` reads (ov-736), so a bare call to one is refused;
/// `people_rep_plan_tests` has them.
#[test]
fn a_rep_may_call_only_what_is_listed() {
    let mut allowed: Vec<&str> = farcooler_protocol::method::Method::ALL
        .iter()
        .map(|m| m.name())
        .filter(|name| rep_may(&Request { method: name.to_string(), ..Default::default() }))
        .collect();
    allowed.sort();
    let mut listed = vec![
        "host.get", "host.health", "daemon.version", "repository.list", "worktree.list", "branch.list",
        "terminal.list", "layout.list", "theme.list", "changes.inbox", "stack.get", "needs_you.list",
        "report.get", "usage.report", "usage.task", "task.list", "task.get", "task.get_by_key", "task.search",
        "plan.get", "plan.events", "workspace.list", "client.list",
        "member.list", "message.send", "worktree.grep",
    ];
    listed.sort();
    assert_eq!(allowed, listed);
    let note = |kind: pb::TaskNoteKind| Request {
        method: "task.note".into(),
        payload: Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend { kind: kind as i32, ..Default::default() })),
        ..Default::default()
    };
    assert!(rep_may(&note(pb::TaskNoteKind::Comment)));
    for kind in [pb::TaskNoteKind::Answer, pb::TaskNoteKind::Decision, pb::TaskNoteKind::Progress, pb::TaskNoteKind::Unspecified] {
        assert!(!rep_may(&note(kind)), "{kind:?}");
    }
    assert!(!rep_may(&Request { method: "no.such_method".into(), ..Default::default() }));
}

/// The ancestor walk finds the pane a process descends from, and nothing for
/// one outside every pane or in a loop.
#[test]
fn a_process_is_found_in_the_pane_it_descends_from() {
    let pane = Uuid::now_v7();
    let parents: std::collections::HashMap<i32, i32> = [(300, 200), (200, 100), (100, 1), (50, 1), (7, 8), (8, 7)].into();
    let panes: std::collections::HashMap<i32, Uuid> = [(100, pane)].into();
    assert_eq!(pane_among(300, &parents, &panes), Some(pane), "a grandchild");
    assert_eq!(pane_among(100, &parents, &panes), Some(pane), "the pane's own process");
    assert_eq!(pane_among(50, &parents, &panes), None, "outside every pane");
    assert_eq!(pane_among(7, &parents, &panes), None, "a loop ends");
}

/// The lead is a runner admin's to hand on, through the workspace's
/// settings, to a member who exists, and back with `owner`; the workspace
/// says who leads.
#[tokio::test]
async fn the_lead_is_handed_on_by_a_runner_admin() {
    let r = runner().await;
    let set = |lead: &str| Request {
        method: "workspace.set_settings".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings {
            lead: Some(lead.into()),
            ..Default::default()
        })),
        ..Default::default()
    };
    let control = factory(&r.svc, Some("phone-o"), Scope::Control);
    assert_eq!(refused(&control, set("bob")).await.as_deref(), Some("scope-denied:"));
    assert_eq!(r.svc.store.workspace_lead(r.workspace).unwrap(), None);
    let admin = factory(&r.svc, None, Scope::HostAdmin);
    assert_eq!(refused(&admin, set("carol")).await.as_deref(), Some("invalid-argument:lead"), "nobody by that handle");
    // A lead given with a setting that is then refused is not handed on.
    let stale = Request {
        method: "workspace.set_settings".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings {
            lead: Some("bob".into()),
            wake_on_answer: Some(false),
            expected_version: Some(9_999),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(refused(&admin, stale).await.as_deref(), Some("resource-conflict:"));
    assert_eq!(r.svc.store.workspace_lead(r.workspace).unwrap(), None, "a refusal leaves the lead as it was");
    assert_eq!(refused(&admin, set("bob")).await, None);
    assert_eq!(r.svc.store.workspace_lead(r.workspace).unwrap().as_deref(), Some("bob"));
    let listed = crate::workspace_ops::list(&r.svc, None, Scope::Read).unwrap().items.remove(0);
    assert_eq!((listed.lead.as_deref(), listed.lead_name.as_deref()), (Some("bob"), Some("Bob")));
    assert_eq!(refused(&admin, set("owner")).await, None);
    let listed = crate::workspace_ops::list(&r.svc, None, Scope::Read).unwrap().items.remove(0);
    assert_eq!((listed.lead.as_deref(), listed.lead_name.as_deref()), (Some(""), Some("")));
}

/// A rep's pane can't write an answer or a decision, or answer a prompt; it
/// can comment. Any other pane can do all three.
#[tokio::test]
async fn a_rep_never_answers() {
    let r = runner().await;
    let watcher = crate::watch::Watcher::new(r.svc.clone());
    let task = r.svc.store.create_task(r.workspace, "Dark mode", Actor::Manager).unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let note = |actor: Uuid, kind: pb::TaskNoteKind| pb::TaskNoteAppend {
        task_id: id_bytes(task.id),
        kind: kind as i32,
        body: "Yes, ship it".into(),
        actor: format!("agent:{actor}"),
        ..Default::default()
    };
    for kind in [pb::TaskNoteKind::Answer, pb::TaskNoteKind::Decision] {
        let refused = crate::task_ops::note(&r.svc, &watcher, &note(r.agent.id, kind));
        assert!(matches!(refused, Err(DomainError::Conflict { what: "rep_messenger" })), "{kind:?}: {refused:?}");
    }
    crate::task_ops::note(&r.svc, &watcher, &note(r.agent.id, pb::TaskNoteKind::Comment)).expect("a rep comments");
    crate::task_ops::note(&r.svc, &watcher, &note(r.orchestrator.id, pb::TaskNoteKind::Decision)).expect("anyone else decides");

    let owner = factory(&r.svc, None, Scope::HostAdmin);
    let answer = |actor: Uuid| Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(pb::AgentAnswer {
            terminal_id: id_bytes(r.agent.id),
            request_id: "hook-ask:1".into(),
            option_id: "allow".into(),
            actor: format!("agent:{actor}"),
            ..Default::default()
        })),
        ..Default::default()
    };
    assert_eq!(refused(&owner, answer(r.agent.id)).await.as_deref(), Some("resource-conflict:rep_messenger"));
    assert_ne!(refused(&owner, answer(r.orchestrator.id)).await.as_deref(), Some("resource-conflict:rep_messenger"));
}

/// A rep's comment stays its own (ov-527): on its own workspace's cards, with
/// no `message` record in its extra, and superseding only its own notes.
/// Another pane's comment is held to none of it.
#[tokio::test]
async fn a_reps_comment_stays_its_own() {
    let r = runner().await;
    let watcher = crate::watch::Watcher::new(r.svc.clone());
    let ours = r.svc.store.create_task(r.workspace, "Dark mode", Actor::Manager).unwrap();
    let repo = r.svc.store.get_workspace(r.workspace).unwrap().repository_id;
    let billing = r.svc.store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let theirs = r.svc.store.create_task(billing, "Invoices", Actor::Manager).unwrap();
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let comment = |by: Uuid, task: Uuid, extra: &str, supersedes: Option<Uuid>| pb::TaskNoteAppend {
        task_id: id_bytes(task),
        kind: pb::TaskNoteKind::Comment as i32,
        body: "Bob would like it by Friday".into(),
        actor: format!("agent:{by}"),
        extra_json: extra.into(),
        supersedes: supersedes.map(id_bytes),
    };
    let refused = |result: Result<pb::TaskNote>| matches!(result, Err(DomainError::Conflict { what: "rep_messenger" }));
    let own = crate::task_ops::note(&r.svc, &watcher, &comment(r.agent.id, ours.id, "", None)).expect("on its own board");
    let own = Uuid::from_slice(&own.id).unwrap();
    assert!(refused(crate::task_ops::note(&r.svc, &watcher, &comment(r.agent.id, theirs.id, "", None))), "another workspace's card");
    let message = r#"{"message":{"to":"the orchestrator","from":"Alice"}}"#;
    assert!(refused(crate::task_ops::note(&r.svc, &watcher, &comment(r.agent.id, ours.id, message, None))), "a message's record");
    let orchestrators = crate::task_ops::note(&r.svc, &watcher, &comment(r.orchestrator.id, ours.id, "", None)).unwrap();
    let orchestrators = Uuid::from_slice(&orchestrators.id).unwrap();
    assert!(refused(crate::task_ops::note(&r.svc, &watcher, &comment(r.agent.id, ours.id, "", Some(orchestrators)))), "another's note");
    crate::task_ops::note(&r.svc, &watcher, &comment(r.agent.id, ours.id, "", Some(own))).expect("its own note");
    crate::task_ops::note(&r.svc, &watcher, &comment(r.orchestrator.id, theirs.id, "", None)).expect("not a rep");
}

/// The shared account is the email in Claude Code's own file, from its
/// config directory when one is set; nothing when there's nothing to read.
#[test]
fn the_shared_account_is_read_from_claude_codes_file() {
    let home = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let write = |dir: &Path, email: &str| {
        let json = serde_json::json!({ "oauthAccount": { "emailAddress": email, "displayName": "x" } });
        std::fs::write(dir.join(".claude.json"), json.to_string()).unwrap();
    };
    assert_eq!(claude_account(Some(home.path()), None), None, "no file");
    write(home.path(), "owner@example.com");
    write(config.path(), "team@example.com");
    assert_eq!(claude_account(Some(home.path()), None).as_deref(), Some("owner@example.com"));
    assert_eq!(claude_account(Some(home.path()), Some(config.path())).as_deref(), Some("team@example.com"));
    write(home.path(), "two\nlines");
    assert_eq!(claude_account(Some(home.path()), None), None, "never a line break into a prompt");
    std::fs::write(home.path().join(".claude.json"), "{").unwrap();
    assert_eq!(claude_account(Some(home.path()), None), None, "not JSON");
}


/// Turning the nudge on posts to GitHub as the owner, so only a person does
/// it (ov-580): a caller in an agent's pane, told by its pid, is refused
/// `person_only` however it is scoped; turning it off is anyone's; the same
/// call from any other process is left to its handler.
#[tokio::test]
async fn an_agent_pane_cannot_turn_nudging_on_but_can_turn_it_off() {
    let r = runner().await;
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the agent's pane").stamp.pid as i32;
    let in_agent = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6593);
    let elsewhere = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    let set = |on: bool| Request {
        method: "workspace.set_settings".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings { pr_nudge: Some(on), ..Default::default() })),
        ..Default::default()
    };
    assert_eq!(refused(&in_agent, set(true)).await.as_deref(), Some("resource-conflict:person_only"));
    assert!(!r.svc.store.get_landing(r.workspace).unwrap().pr_nudge, "and nothing was turned on");
    assert_eq!(refused(&in_agent, set(false)).await, None, "off is anyone's");
    assert_eq!(refused(&elsewhere, set(true)).await, None, "a person's own terminal may");
    assert!(r.svc.store.get_landing(r.workspace).unwrap().pr_nudge);
    // The orchestrator's pane is an agent too.
    let orch_pid = snapshot.claimants(r.orchestrator.id).into_iter().next().expect("its pane").stamp.pid as i32;
    let in_orchestrator = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(orch_pid));
    assert_eq!(refused(&in_orchestrator, set(true)).await.as_deref(), Some("resource-conflict:person_only"));
}

/// A command an agent's tool shell left behind with `(cmd &)` (orphaned to
/// pid 1, no terminal) is in no pane the walks can name, and that isn't
/// "in no pane" (ov-649): it can't turn nudging on, and once the runner has
/// a rep it gets only what a rep may, since it might be one's. Before, both
/// passed it as the owner.
#[tokio::test]
async fn a_command_left_behind_is_neither_a_person_nor_out_of_every_pane() {
    let r = runner().await;
    let claude = crate::test_support::Outside::stand_in(&r.dir.path().join("fake-agent").join("claude"));
    let left = crate::test_support::Outside::left_by(&claude, 6594);
    let from = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(left.pid));
    let set = |on: bool| Request {
        method: "workspace.set_settings".into(),
        target_resource_id: Some(id_bytes(r.workspace)),
        payload: Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings { pr_nudge: Some(on), ..Default::default() })),
        ..Default::default()
    };
    assert_eq!(refused(&from, set(true)).await.as_deref(), Some("resource-conflict:person_only"));
    assert!(!r.svc.store.get_landing(r.workspace).unwrap().pr_nudge, "nothing was turned on");
    assert_eq!(refused(&from, set(false)).await, None, "off is anyone's, on a runner with no rep");
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    assert_eq!(refused(&from, set(false)).await.as_deref(), Some("resource-conflict:in_pane"), "with a rep, nothing a rep can't");
}

/// The board's read state `rpc` reads for `workspace`: whose, and its floor.
async fn board_reads(rpc: &RpcFactory, workspace: Uuid) -> (String, i64) {
    use farcooler_transport::Handler as _;
    let list = Request {
        method: "task.list".into(),
        payload: Some(request::Payload::TaskList(pb::TaskListRequest { workspace_id: Some(id_bytes(workspace)), ..Default::default() })),
        ..Default::default()
    };
    match rpc.handle(list).await.outcome {
        Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::TaskList(l)) })) => {
            let reads = l.reads.expect("a board's read state");
            (reads.member, reads.floor_ms)
        }
        other => panic!("wrong result: {other:?}"),
    }
}

/// ov-744 (3): a caller in Bob's rep's pane reads the board as Bob, by its
/// pid and not by anything it says; any other process on the local socket
/// reads it as the owner. It marks nothing read for him: those writes stay
/// a messenger's to refuse.
#[tokio::test]
async fn a_reps_pane_reads_its_members_marks_and_writes_none() {
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let mark = |floor: i64| Request {
        method: "workspace.mark_read".into(),
        payload: Some(request::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead { workspace_id: id_bytes(r.workspace), floor_ms: Some(floor), opened: Vec::new() })),
        ..Default::default()
    };
    let bob = factory(&r.svc, Some("phone-b"), Scope::Control);
    let floor = crate::review::now_millis();
    assert_eq!(refused(&bob, mark(floor)).await, None, "Bob marks his board read");
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let in_rep = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(pane_pid));
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6744);
    let elsewhere = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));

    assert_eq!(board_reads(&in_rep, r.workspace).await, ("bob".to_string(), floor), "the rep reads Bob's");
    assert_eq!(in_rep.caller_pane_for_tests(), Some(r.agent.id), "walked once, kept for the connection");
    assert_eq!(elsewhere.caller_pane_for_tests(), None, "not walked before it reads");
    let (whose, owners) = board_reads(&elsewhere, r.workspace).await;
    assert_eq!(whose, "", "anyone else on the socket reads the owner's");
    assert!(owners < floor, "and Bob's floor isn't the owner's");
    for req in [mark(floor + 60_000), Request { method: "terminal.seen".into(), target_resource_id: Some(id_bytes(r.agent.id)), ..Default::default() }] {
        assert_eq!(refused(&in_rep, req.clone()).await.as_deref(), Some(MESSENGER), "{}", req.method);
    }
    assert_eq!(board_reads(&bob, r.workspace).await.1, floor, "Bob's marks are as he left them");
}

/// ov-744 (3): the rep's pane reads Bob's reviews and his looks too:
/// `changes.inbox` and `terminal.list`, each against what Bob did on his
/// phone, and the owner's on the rest of the socket.
#[tokio::test]
async fn a_reps_pane_reads_its_members_reviews_and_looks() {
    use farcooler_transport::Handler as _;
    let r = runner().await;
    r.svc.store.bind_rep(r.agent.id, r.workspace, "bob").unwrap();
    let watcher = crate::watch::Watcher::new(r.svc.clone());
    let on = |client: Option<&str>, pid: Option<i32>| {
        let peer = Peer { client_id: client.map(str::to_string), scope: Scope::Control };
        RpcFactory::new(r.svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer).with_caller_pid(pid)
    };
    let snapshot = r.svc.inventory.refresh().await;
    let pane_pid = snapshot.claimants(r.agent.id).into_iter().next().expect("the rep's pane").stamp.pid as i32;
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6745);
    let (bob, in_rep, elsewhere) = (on(Some("phone-b"), None), on(None, Some(pane_pid)), on(None, Some(person.pid)));

    let repo = r.svc.store.get_workspace(r.workspace).unwrap().repository_id;
    let wt = r.svc.create_worktree(repo, "wt-rep", "feature/rep", "main").await.expect("a worktree");
    r.svc.store.set_review_base(wt.id, "main").unwrap();
    r.svc.review_cache.set_counts_for_tests(wt.id, std::path::Path::new(&wt.worktree_path), crate::review::Counts::Known(1, 4, 0));
    let reviewed = Request {
        method: "changes.mark_read".into(),
        payload: Some(request::Payload::ChangesMarkRead(pb::ChangesMarkRead { worktree_id: id_bytes(wt.id), branch: wt.branch.clone() })),
        ..Default::default()
    };
    assert_eq!(refused(&bob, reviewed).await, None, "Bob reviews it");
    let changed = |rpc: &RpcFactory| {
        let rpc = rpc.clone();
        async move {
            let req = Request { method: "changes.inbox".into(), ..Default::default() };
            match rpc.handle(req).await.outcome {
                Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::ChangesInbox(i)) })) => {
                    i.items.into_iter().find(|w| w.worktree_id == id_bytes(wt.id)).is_some_and(|w| w.changed_since_reviewed)
                }
                other => panic!("wrong result: {other:?}"),
            }
        }
    };
    assert!(!changed(&in_rep).await, "the rep reads Bob's review");
    assert!(changed(&elsewhere).await, "the owner hasn't reviewed it");

    let finished = crate::needs_you::Observation {
        activity: pb::AgentActivity::Done,
        state_since: crate::review::now_millis() - 5_000,
        command: "claude".into(),
        ..Default::default()
    };
    watcher.observe_for_tests(r.agent.id, finished).await;
    let seen = Request { method: "terminal.seen".into(), target_resource_id: Some(id_bytes(r.agent.id)), ..Default::default() };
    assert_eq!(refused(&bob, seen).await, None, "Bob opens the pane");
    let activity = |rpc: &RpcFactory| {
        let rpc = rpc.clone();
        async move {
            let list = Request { method: "terminal.list".into(), ..Default::default() };
            match rpc.handle(list).await.outcome {
                Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::TerminalList(l)) })) => {
                    l.items.into_iter().find(|t| t.id == id_bytes(r.agent.id)).expect("the rep's pane").activity()
                }
                other => panic!("wrong result: {other:?}"),
            }
        }
    };
    assert_eq!(activity(&in_rep).await, pb::AgentActivity::Idle, "the rep reads Bob's look");
    assert_eq!(activity(&elsewhere).await, pb::AgentActivity::Done, "the owner hasn't looked");
}

/// ov-747: Bob's own pane (a shell on his login) writes his marks by the one
/// rule (`caller_person`): his board read, his review, his look at a pane
/// and his screen, each kept apart from the owner's on the rest of the
/// socket. And the socket's events carry each one's: a board's read state,
/// and a pane's row. Red at each call site that asks the device alone.
#[tokio::test]
async fn a_members_own_pane_writes_their_marks_and_hears_their_reads() {
    use farcooler_protocol::v1::{Event, event};
    use farcooler_transport::Handler as _;
    let r = runner().await;
    let watcher = crate::watch::Watcher::new(r.svc.clone());
    let on = |client: Option<&str>, pid: Option<i32>| {
        let peer = Peer { client_id: client.map(str::to_string), scope: Scope::Control };
        RpcFactory::new(r.svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer).with_caller_pid(pid)
    };
    let shell = r.svc.create_terminal(r.agent.worktree_id, "Bob's", "shell").await.expect("Bob's shell");
    let login_id = r.svc.store.ensure_member_login("bob").unwrap();
    let login = farcooler_store::claude_logins::TerminalLogin { handle: "bob".into(), login_id, config_dir: "/r/claude/bob".into(), shared: false };
    r.svc.store.set_terminal_login(shell.id, &login).unwrap();
    let pid = r.svc.inventory.refresh().await.claimants(shell.id).into_iter().next().expect("Bob's pane").stamp.pid as i32;
    let person = crate::test_support::Outside::a_person(r.dir.path(), 6747);
    let (phone, in_bobs, mac) = (on(Some("phone-b"), None), on(None, Some(pid)), on(None, Some(person.pid)));

    // His board.
    let floor = crate::review::now_millis();
    let mark = Request {
        method: "workspace.mark_read".into(),
        payload: Some(request::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead { workspace_id: id_bytes(r.workspace), floor_ms: Some(floor), opened: Vec::new() })),
        ..Default::default()
    };
    assert_eq!(refused(&in_bobs, mark).await, None, "Bob marks his board read in his pane");
    assert_eq!(board_reads(&phone, r.workspace).await, ("bob".to_string(), floor), "his phone reads it");
    assert!(board_reads(&mac, r.workspace).await.1 < floor, "the owner's isn't moved");

    // His review.
    let repo = r.svc.store.get_workspace(r.workspace).unwrap().repository_id;
    let wt = r.svc.create_worktree(repo, "wt-own", "feature/own", "main").await.expect("a worktree");
    r.svc.store.set_review_base(wt.id, "main").unwrap();
    r.svc.review_cache.set_counts_for_tests(wt.id, std::path::Path::new(&wt.worktree_path), crate::review::Counts::Known(1, 4, 0));
    let reviewed = Request {
        method: "changes.mark_read".into(),
        payload: Some(request::Payload::ChangesMarkRead(pb::ChangesMarkRead { worktree_id: id_bytes(wt.id), branch: wt.branch.clone() })),
        ..Default::default()
    };
    assert_eq!(refused(&in_bobs, reviewed).await, None, "Bob reviews it in his pane");
    let changed = |rpc: &RpcFactory| {
        let rpc = rpc.clone();
        async move {
            let req = Request { method: "changes.inbox".into(), ..Default::default() };
            match rpc.handle(req).await.outcome {
                Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::ChangesInbox(i)) })) => {
                    i.items.into_iter().find(|w| w.worktree_id == id_bytes(wt.id)).is_some_and(|w| w.changed_since_reviewed)
                }
                other => panic!("wrong result: {other:?}"),
            }
        }
    };
    assert!(!changed(&phone).await, "his phone reads his review");
    assert!(changed(&mac).await, "the owner hasn't reviewed it");

    // His look at a finished pane.
    let finished = crate::needs_you::Observation {
        activity: pb::AgentActivity::Done,
        state_since: crate::review::now_millis() - 5_000,
        command: "claude".into(),
        ..Default::default()
    };
    watcher.observe_for_tests(r.agent.id, finished).await;
    let seen = Request { method: "terminal.seen".into(), target_resource_id: Some(id_bytes(r.agent.id)), ..Default::default() };
    assert_eq!(refused(&in_bobs, seen).await, None);
    let bob = Person::Member("bob".into());
    assert_eq!(watcher.activity_for(&bob, r.agent.id).await.0, pb::AgentActivity::Idle, "Bob's Done is over");
    assert_eq!(watcher.activity(r.agent.id).await.0, pb::AgentActivity::Done, "the owner's stands");

    // A pane's row, as each reads it.
    let list = Request { method: "terminal.list".into(), ..Default::default() };
    let Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::TerminalList(l)) })) = mac.handle(list).await.outcome else { panic!("a list") };
    let row = l.items.into_iter().find(|t| t.id == id_bytes(r.agent.id)).expect("the pane's row");
    let changed_row = Event { payload: Some(event::Payload::TerminalChanged(row)), ..Event::default() };
    let activity = |rpc: &RpcFactory| match rpc.shape(changed_row.clone()).payload {
        Some(event::Payload::TerminalChanged(t)) => t.activity(),
        other => panic!("{other:?}"),
    };
    assert_eq!((activity(&in_bobs), activity(&mac)), (pb::AgentActivity::Idle, pb::AgentActivity::Done), "a row's activity is its reader's");

    // A board's read state, to whose it is.
    let reads = |member: &str| Event {
        payload: Some(event::Payload::BoardReadsChanged(pb::BoardReads { member: member.into(), ..Default::default() })),
        ..Event::default()
    };
    assert_eq!((in_bobs.admits(&reads("bob")), mac.admits(&reads("bob"))), (true, false), "Bob's reads, to his pane");
    assert_eq!((in_bobs.admits(&reads("")), mac.admits(&reads(""))), (false, true), "the owner's, to the owner");

    // His screen, which isn't the owner's, and doesn't replace the Mac's.
    let watching = |ids: Vec<Uuid>| Request {
        method: "terminal.watching".into(),
        payload: Some(request::Payload::TerminalsWatched(pb::TerminalsWatched { terminal_ids: ids.iter().map(|id| id_bytes(*id)).collect() })),
        ..Default::default()
    };
    assert_eq!(refused(&in_bobs, watching(vec![r.agent.id])).await, None);
    assert!(watcher.watched_by_for_tests(Some("bob"), r.agent.id), "Bob's screen");
    assert!(!watcher.watched_by_for_tests(None, r.agent.id), "not the owner's");
    assert_eq!(refused(&mac, watching(vec![r.agent.id])).await, None);
    assert!(watcher.watched_by_for_tests(None, r.agent.id) && watcher.watched_by_for_tests(Some("bob"), r.agent.id), "each keeps their own claim");

    // Review 747 F4: once Bob is removed, his pane is nobody's. It claims no
    // screen, the owner's least of all, and writes no marks.
    r.svc.store.remove_member("bob").unwrap();
    let other = r.orchestrator.id;
    assert_eq!(refused(&in_bobs, watching(vec![other])).await.as_deref(), Some(NOT_YOURS), "a removed member's pane");
    assert!(!watcher.watched_by_for_tests(None, other), "and the owner's screen isn't claimed for it");
    let mark = Request {
        method: "workspace.mark_read".into(),
        payload: Some(request::Payload::WorkspaceMarkRead(pb::WorkspaceMarkRead { workspace_id: id_bytes(r.workspace), floor_ms: Some(floor + 60_000), opened: Vec::new() })),
        ..Default::default()
    };
    assert_eq!(refused(&in_bobs, mark).await.as_deref(), Some(NOT_YOURS), "nor marks a board read");
    r.svc.stop_terminal(shell.id).await.expect("stopped");
}

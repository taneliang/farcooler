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
    _dir: ScratchDir,
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
    Runner { _dir: dir, svc, workspace, orchestrator, agent }
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

/// The refusal's `what`, or `None` for a result.
async fn refused(factory: &RpcFactory, req: Request) -> Option<String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => None,
        Some(response::Outcome::Error(e)) if e.what.is_empty() => Some(format!("code {}", e.code)),
        Some(response::Outcome::Error(e)) => Some(e.what),
        other => panic!("no outcome: {other:?}"),
    }
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

/// Only the lead types into the orchestrator, by any of the ways a client
/// can. The owner leads until the lead is handed on; then the member does,
/// and the owner is refused too. Other panes are anyone's with Control.
#[tokio::test]
async fn only_the_lead_types_into_the_orchestrator() {
    let r = runner().await;
    let owner = factory(&r.svc, None, Scope::HostAdmin);
    let owners_phone = factory(&r.svc, Some("phone-o"), Scope::Control);
    let bob = factory(&r.svc, Some("phone-b"), Scope::Control);
    let not_lead = Some("not_lead".to_string());
    for req in typing_into(r.orchestrator.id) {
        let method = req.method.clone();
        assert_eq!(refused(&bob, req.clone()).await, not_lead, "{method}: Bob isn't the lead");
        assert_ne!(refused(&owner, req.clone()).await, not_lead, "{method}: the owner leads by default");
        assert_ne!(refused(&owners_phone, req).await, not_lead, "{method}: a device bound to nobody is the owner's");
    }
    for req in typing_into(r.agent.id) {
        assert_ne!(refused(&bob, req.clone()).await, not_lead, "{}: an agent's pane isn't the orchestrator", req.method);
    }

    r.svc.store.set_workspace_lead(r.workspace, Some("bob")).unwrap();
    for req in typing_into(r.orchestrator.id) {
        let method = req.method.clone();
        assert_ne!(refused(&bob, req.clone()).await, not_lead, "{method}: Bob leads now");
        assert_eq!(refused(&owner, req).await, not_lead, "{method}: one lead, and it isn't the owner");
    }
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
    assert_eq!(refused(&control, set("bob")).await.as_deref(), Some(denied().as_str()));
    assert_eq!(r.svc.store.workspace_lead(r.workspace).unwrap(), None);
    let admin = factory(&r.svc, None, Scope::HostAdmin);
    assert_eq!(refused(&admin, set("carol")).await.as_deref(), Some("lead"), "nobody by that handle");
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
    assert_eq!(refused(&owner, answer(r.agent.id)).await.as_deref(), Some("rep_messenger"));
    assert_ne!(refused(&owner, answer(r.orchestrator.id)).await.as_deref(), Some("rep_messenger"));
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

/// How a scope refusal reads in these tests' helpers: its code, as it has no `what`.
fn denied() -> String {
    format!("code {}", DomainError::ScopeDenied { needed: "host_admin" }.wire().0 as i32)
}

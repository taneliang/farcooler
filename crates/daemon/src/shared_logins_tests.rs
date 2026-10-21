//! Members sharing the runner's own logins (ov-652): the switch's default and
//! who may set it, and every launch path in each mode, for each harness, on
//! the fixture's real tmux server with the test stub in place of the agents.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Handler as _;
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::claude_logins::tests::{assert_on, assert_runners, create, dir_of, factory, launched, pid_of, sharing_runner};
use crate::service::Service;
use crate::wire::id_bytes;

/// Put `handle`'s directory in the state `claude auth login` leaves it.
fn sign_in(svc: &Service, handle: &str) {
    let id = svc.store.ensure_member_login(handle).unwrap();
    let dir = crate::claude_logins::ensure_login_dir(svc.root_dir(), &id).unwrap();
    std::fs::write(std::path::Path::new(&dir).join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"bob@example.com"}}"#).unwrap();
}

fn require_own(svc: &Service, harness: &str, on: bool) {
    svc.store.set_member_login_policy(harness, on, "-", 1).unwrap();
}

/// Whether `pane` is recorded as `handle`'s on the runner's own login.
fn shared(svc: &Service, pane: Uuid, handle: &str) -> bool {
    let login = svc.store.terminal_login(pane).unwrap().expect("recorded as the member's pane");
    assert_eq!(login.handle, handle, "still theirs");
    login.shared
}

/// Nothing of a member's own credential, and no Far Cooler credential of
/// any harness: what an owner's pane carries.
fn assert_carries_nothing_of_theirs(command: &str) {
    assert_runners(command);
    for var in ["CURSOR_API_KEY", "AGENT_CLI_CREDENTIAL_STORE", "CURSOR_CONFIG_DIR", "CODEX_HOME", "OPENAI_API_KEY"] {
        assert!(!command.contains(var), "{var} in {command}");
    }
}

async fn call(who: &crate::rpc::RpcFactory, req: Request) -> Result<pb::result::Value, String> {
    match who.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn set_own(harness: &str, on: bool) -> Request {
    Request {
        method: "member.set_own_login".into(),
        payload: Some(request::Payload::MemberOwnLoginSet(pb::MemberOwnLoginSet { harness: harness.into(), on })),
        ..Default::default()
    }
}

fn list() -> Request {
    Request { method: "member.list".into(), ..Default::default() }
}

fn rep_start(workspace: Uuid, handle: &str) -> Request {
    Request {
        method: "rep.start".into(),
        payload: Some(request::Payload::RepStart(pb::RepStart { workspace_id: id_bytes(workspace), handle: handle.into(), harness: String::new() })),
        ..Default::default()
    }
}

fn as_peer(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope, pid: Option<i32>) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
        .with_caller_pid(pid)
}

fn own_logins(value: pb::result::Value) -> pb::MemberOwnLogins {
    match value {
        pb::result::Value::MemberList(list) => list.own_logins.expect("the switches"),
        other => panic!("not a member list: {other:?}"),
    }
}

/// Off for every harness on a new runner, as `member.list` tells it.
#[tokio::test]
async fn every_switch_is_off_by_default() {
    let (_dir, svc, _main, _checkout) = sharing_runner().await;
    let admin = as_peer(&svc, None, Scope::HostAdmin, None);
    let own = own_logins(call(&admin, list()).await.unwrap());
    assert_eq!(own, pb::MemberOwnLogins { claude: false, codex: false, cursor: false, may_change: true });
}

/// Only the runner's admins set a switch: a `control` device, the owner's
/// own or a member's, is refused by the method's scope, and so is any `read`
/// one; an admin's agent in a pane is refused `person_only`. Each change is
/// recorded with who made it, and `may_change` says who may.
#[tokio::test]
async fn only_a_runner_admin_sets_a_switch() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    for (device, scope) in [(Some("phone-b"), Scope::Control), (None, Scope::Control), (Some("phone-b"), Scope::Read)] {
        let who = as_peer(&svc, device, scope, None);
        assert_eq!(call(&who, set_own("codex", true)).await.err().as_deref(), Some("scope-denied:"), "{device:?} {scope:?}");
        assert!(!own_logins(call(&who, list()).await.unwrap()).may_change, "told it may not");
    }
    let pane = create(&factory(&svc, None, None), checkout, "claude").await.expect("the owner's agent");
    let agent = as_peer(&svc, None, Scope::HostAdmin, Some(pid_of(&svc, pane).await));
    assert_eq!(call(&agent, set_own("codex", false)).await.err().as_deref(), Some("resource-conflict:person_only"));
    assert_eq!(svc.store.member_login_policy().unwrap(), Default::default(), "nothing changed");

    let admin = as_peer(&svc, Some("laptop-admin"), Scope::HostAdmin, None);
    let own = own_logins(call(&admin, set_own("codex", true)).await.expect("an admin sets it"));
    assert_eq!((own.claude, own.codex, own.cursor, own.may_change), (false, true, false, true));
    assert_eq!(svc.store.member_login_policy_set("codex").unwrap().map(|(by, _)| by).as_deref(), Some("laptop-admin"), "recorded");
    let owner = as_peer(&svc, None, Scope::HostAdmin, None);
    assert_eq!(call(&owner, set_own("aider", true)).await.err().as_deref(), Some("invalid-argument:harness"));
}

/// Off: a member's new Claude, shell, Codex and Cursor panes, opened from
/// their phone, run on the runner's own login and carry nothing of theirs;
/// each is still recorded as their pane. The owner's are unchanged.
#[tokio::test]
async fn off_a_members_new_panes_run_on_the_runners_logins() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    let bob = factory(&svc, Some("phone-b"), None);
    for preset in ["claude", "shell", "codex", "cursor", "codex:gpt-5"] {
        let pane = create(&bob, checkout, preset).await.unwrap_or_else(|e| panic!("{preset}: {e}"));
        assert_carries_nothing_of_theirs(&launched(&svc, pane).await);
        assert!(shared(&svc, pane, "bob"), "{preset}");
        let term = svc.store.get_terminal(pane).unwrap();
        assert_eq!(svc.claude_dir_of(&term), svc.claude_logins.runner.clone(), "its transcript is under the runner's");
    }
    let owners = create(&factory(&svc, None, None), checkout, "cursor").await.expect("the owner's");
    assert_eq!(svc.store.terminal_login(owners).unwrap(), None, "the owner's pane records nothing");
    assert!(svc.login_needs().is_empty(), "nobody is asked to sign in");
}

/// Off, a member who signed in to Claude still runs on their own Claude
/// login, without the owner's credentials; their Codex and Cursor, which
/// have no login of theirs, share the runner's.
#[tokio::test]
async fn off_a_signed_in_member_keeps_their_own_claude() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    sign_in(&svc, "bob");
    let bob = factory(&svc, Some("phone-b"), None);
    let claude = create(&bob, checkout, "claude").await.expect("Bob's claude");
    let command = launched(&svc, claude).await;
    assert_on(&command, &dir_of(&svc, "bob"));
    assert!(command.contains("-u ANTHROPIC_API_KEY"), "never the owner's credential: {command}");
    assert!(!shared(&svc, claude, "bob"));
    let codex = create(&bob, checkout, "codex").await.expect("Bob's codex");
    assert_carries_nothing_of_theirs(&launched(&svc, codex).await);
    assert!(shared(&svc, codex, "bob"));
}

/// On, today's rule holds: a member's Claude is on their own directory
/// whether or not they've signed in, and Codex and Cursor are refused with
/// the sign-in ask. Each switch is its own.
#[tokio::test]
async fn on_a_members_panes_need_their_own_login() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    require_own(&svc, "claude", true);
    require_own(&svc, "codex", true);
    let bob = factory(&svc, Some("phone-b"), None);
    let claude = create(&bob, checkout, "claude").await.expect("Bob's claude");
    assert_on(&launched(&svc, claude).await, &dir_of(&svc, "bob"));
    assert!(!shared(&svc, claude, "bob"));
    assert_eq!(create(&bob, checkout, "codex").await, Err("resource-conflict:codex_sign_in_needed".to_string()));
    let cursor = create(&bob, checkout, "cursor").await.expect("Cursor's switch is still off");
    assert!(shared(&svc, cursor, "bob"));
    require_own(&svc, "cursor", true);
    assert_eq!(create(&bob, checkout, "cursor").await, Err("resource-conflict:cursor_sign_in_needed".to_string()));
}

/// A relaunch decides afresh: a shared Claude pane comes back on the
/// member's own directory once the switch is on, and back on the runner's
/// once it's off again; a shared Codex pane resumes on the runner's while
/// off, and is refused once on.
#[tokio::test]
async fn a_relaunch_follows_the_switch_as_it_is_now() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    let bob = factory(&svc, Some("phone-b"), None);
    let claude = create(&bob, checkout, "claude").await.expect("Bob's claude");
    let codex = create(&bob, checkout, "codex").await.expect("Bob's codex");

    svc.stop_terminal(codex).await.unwrap();
    svc.restart_terminal(codex).await.expect("resumed while off");
    assert_carries_nothing_of_theirs(&launched(&svc, codex).await);
    assert!(shared(&svc, codex, "bob"));

    require_own(&svc, "claude", true);
    require_own(&svc, "codex", true);
    svc.stop_terminal(claude).await.unwrap();
    svc.restart_terminal(claude).await.expect("restarted on Bob's own");
    assert_on(&launched(&svc, claude).await, &dir_of(&svc, "bob"));
    assert!(!shared(&svc, claude, "bob"));
    svc.stop_terminal(codex).await.unwrap();
    let refused = svc.restart_terminal(codex).await;
    assert!(matches!(refused, Err(farcooler_core::DomainError::Conflict { what: "codex_sign_in_needed" })), "{refused:?}");

    require_own(&svc, "claude", false);
    svc.stop_terminal(claude).await.unwrap();
    svc.restart_terminal(claude).await.expect("restarted on the runner's");
    assert_carries_nothing_of_theirs(&launched(&svc, claude).await);
    assert!(shared(&svc, claude, "bob"));
}

/// A member lead's orchestrator: off, a Codex or Claude one starts on the
/// runner's own login, still recorded as theirs; on, Claude's is on their
/// own directory.
#[tokio::test]
async fn a_member_leads_orchestrator_follows_the_switch() {
    let (_dir, svc, main, _checkout) = sharing_runner().await;
    svc.store.set_workspace_lead(main, Some("bob")).unwrap();
    let codex = svc.start_orchestrator(main, "codex", false, None).await.expect("Bob's codex orchestrator");
    assert_carries_nothing_of_theirs(&launched(&svc, codex.id).await);
    assert!(shared(&svc, codex.id, "bob"));
    let claude = svc.start_orchestrator(main, "claude", true, None).await.expect("Bob's claude orchestrator");
    assert_carries_nothing_of_theirs(&launched(&svc, claude.id).await);
    assert!(shared(&svc, claude.id, "bob"));
    require_own(&svc, "claude", true);
    let own = svc.start_orchestrator(main, "claude", true, None).await.expect("on his own");
    assert_on(&launched(&svc, own.id).await, &dir_of(&svc, "bob"));
}

/// A rep of a member with no Claude login: off, it starts on the runner's
/// login, says so in its first message, and reaches no claude.ai connector
/// (those would be the owner's), whatever its member turned on; on, it's
/// refused with the sign-in ask, as before.
#[tokio::test]
async fn a_rep_follows_the_switch_and_never_gets_the_owners_connectors() {
    let (_dir, svc, main, _checkout) = sharing_runner().await;
    svc.store.set_member_connectors("bob", &["Slack".to_string()]).unwrap();
    let bob = factory(&svc, Some("phone-b"), None);
    let started = match call(&bob, rep_start(main, "bob")).await.expect("Bob's rep") {
        pb::result::Value::RepStarted(s) => Uuid::from_slice(&s.terminal_id).unwrap(),
        other => panic!("{other:?}"),
    };
    let command = launched(&svc, started).await;
    assert_carries_nothing_of_theirs(&command);
    assert!(command.contains("--strict-mcp-config"), "no connector: {command}");
    assert!(shared(&svc, started, "bob"));
    let gate = std::fs::read_to_string(svc.rep_connectors_path(started)).unwrap();
    assert_eq!(gate, r#"{"connectors":[]}"#, "the gate allows none");
    svc.refresh_rep_connectors("bob").unwrap();
    assert_eq!(std::fs::read_to_string(svc.rep_connectors_path(started)).unwrap(), r#"{"connectors":[]}"#, "nor after a change");
    let prompt = svc.rep_launch(&svc.store.get_terminal(started).unwrap(), &svc.store.get_worktree(svc.store.get_terminal(started).unwrap().worktree_id).unwrap()).unwrap().unwrap().opening;
    assert!(prompt.contains("You run on this runner's own Claude login"), "{prompt}");

    svc.stop_terminal(started).await.unwrap();
    require_own(&svc, "claude", true);
    assert_eq!(call(&factory(&svc, Some("phone-a"), None), rep_start(main, "alice")).await.err().as_deref(), Some("resource-conflict:claude_sign_in_needed"));
    let woken = svc.resume_rep(started).await;
    assert!(matches!(woken, Err(farcooler_core::DomainError::Conflict { what: "claude_sign_in_needed" })), "nor woken: {woken:?}");
}

/// A failed turn in a member's shared pane is the runner's login failing:
/// it notes nothing against the member's own.
#[tokio::test]
async fn a_shared_panes_failed_login_is_not_the_members() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    let pane = create(&factory(&svc, Some("phone-b"), None), checkout, "claude").await.expect("Bob's claude");
    sign_in(&svc, "bob");
    assert!(!svc.note_auth_failed(pane).unwrap(), "the runner's login, not Bob's");
    assert!(svc.login_needs().is_empty());
}

/// An ask raised while a switch was on is not shown once it's off: there's
/// nothing to sign in to.
#[tokio::test]
async fn an_ask_goes_when_its_switch_is_off() {
    let (_dir, svc, _main, checkout) = sharing_runner().await;
    require_own(&svc, "codex", true);
    create(&factory(&svc, Some("phone-b"), None), checkout, "codex").await.unwrap_err();
    assert_eq!(svc.login_needs().len(), 1);
    require_own(&svc, "codex", false);
    assert!(svc.login_needs().is_empty());
}

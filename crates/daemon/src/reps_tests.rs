//! Starting a member's rep (ov-494), on the fixture's real tmux server with
//! the test stub in place of claude.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::test_support::ScratchDir;
use crate::wire::id_bytes;

async fn runner() -> (ScratchDir, Arc<Service>, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    (dir, svc, main)
}

/// What tmux was told to run in `terminal`'s pane.
async fn launched(svc: &Service, terminal: Uuid) -> String {
    let snapshot = svc.inventory.refresh().await;
    let pane = snapshot.claimants(terminal).into_iter().next().expect("a pane").clone();
    let out = svc.tmux.run(&["display-message", "-p", "-t", &pane.pane_id, "#{pane_start_command}"]).await.unwrap();
    out.stdout.trim().to_string()
}

/// The first message names the skill, the person, the workspace and the
/// account everyone shares, or says it's the runner's when it can't be read.
#[test]
fn a_reps_first_message_says_who_and_whose_account() {
    assert_eq!(
        first_prompt("Alice Chen", "alice", "Main", Some("team@example.com")),
        "/farcooler:rep You represent Alice Chen (alice) in the Main workspace. Work here runs on the Claude \
         account team@example.com, which everyone on this runner shares."
    );
    assert!(first_prompt("Bob", "bob", "Main", None).ends_with("the Claude account signed in on this runner, which everyone on it shares."));
}

/// A rep is a claude pane in the main checkout, seated in its workspace and
/// bound to its member before it launches, started on the rep skill, with a
/// desk card labeled `rep` that the next rep reuses. A running one is
/// refused; a stopped one is replaced.
#[tokio::test]
async fn a_rep_is_bound_seated_and_given_a_desk() {
    let (_dir, svc, main) = runner().await;
    let started = svc.start_rep(main, "alice", "").await.expect("a rep");
    let pane = svc.store.get_terminal(started.terminal.id).unwrap();
    assert_eq!((pane.workspace_id, pane.role, pane.title.as_str()), (Some(main), models::TerminalRole::Agent, "Alice Chen's rep"));
    assert!(svc.store.get_worktree(pane.worktree_id).unwrap().is_main_checkout);
    let rep = svc.store.rep_of_terminal(pane.id).unwrap().expect("bound");
    assert_eq!((rep.workspace, rep.handle.as_str()), (main, "alice"));
    let command = launched(&svc, pane.id).await;
    assert!(command.contains("/farcooler:rep You represent Alice Chen (alice) in the Main workspace."), "{command}");
    assert!(command.contains(&format!("FARCOOLER_WORKSPACE={main}")), "{command}");
    assert!(command.contains("FARCOOLER_REP=alice"), "{command}");
    assert_eq!(started.desk.title, "Alice Chen's desk");
    assert_eq!(started.desk.labels, [DESK_LABEL]);
    assert_eq!(svc.store.rep_desk(main, "alice").unwrap(), Some(started.desk.id));

    let again = svc.start_rep(main, "alice", "claude").await;
    assert!(matches!(again, Err(DomainError::Conflict { what: "rep_running" })), "{again:?}");
    svc.stop_terminal(pane.id).await.unwrap();
    let next = svc.start_rep(main, "alice", "claude:sonnet").await.expect("a stopped rep is replaced");
    assert_ne!(next.terminal.id, pane.id);
    assert!(svc.store.get_terminal(pane.id).is_err(), "the stopped one is removed");
    assert_eq!(next.desk.id, started.desk.id, "the desk is the member's, not the pane's");

    // In another workspace, the main checkout is still Main's: the rep is
    // seated in its own before it launches, so it files there.
    let repo = svc.store.get_workspace(main).unwrap().repository_id;
    let billing = svc.store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let bobs = svc.start_rep(billing, "bob", "claude").await.expect("Bob's rep in Billing");
    assert_eq!(svc.store.get_terminal(bobs.terminal.id).unwrap().workspace_id, Some(billing));
    assert!(launched(&svc, bobs.terminal.id).await.contains(&format!("FARCOOLER_WORKSPACE={billing}")));
    assert_eq!(bobs.desk.workspace_id, billing);

    let codex = svc.start_rep(main, "alice", "codex").await;
    assert!(matches!(codex, Err(DomainError::InvalidArgument { what: "command_preset" })), "{codex:?}");
    assert!(matches!(svc.start_rep(main, "carol", "claude").await, Err(DomainError::NotFound)), "nobody by that handle");
}

fn factory(svc: &Arc<Service>, client_id: Option<&str>, scope: Scope) -> crate::rpc::RpcFactory {
    let peer = Peer { client_id: client_id.map(str::to_string), scope };
    crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(factory: &crate::rpc::RpcFactory, req: Request) -> std::result::Result<pb::result::Value, String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

/// `rep.start` is a runner admin's, or the member's own; `member.remove`
/// stops their rep; `member.list` says who's asking and who has a rep.
#[tokio::test]
async fn members_start_their_own_rep_and_lose_it_when_removed() {
    let (_dir, svc, main) = runner().await;
    let start = |handle: &str| Request {
        method: "rep.start".into(),
        payload: Some(request::Payload::RepStart(pb::RepStart {
            workspace_id: id_bytes(main),
            handle: handle.into(),
            harness: String::new(),
        })),
        ..Default::default()
    };
    let bob = factory(&svc, Some("phone-b"), Scope::Control);
    assert_eq!(call(&bob, start("alice")).await.err().as_deref(), Some("scope-denied:"), "not Bob's to start");
    let Ok(pb::result::Value::RepStarted(started)) = call(&bob, start("bob")).await else { panic!("Bob starts his own") };
    let desk = svc.store.rep_desk(main, "bob").unwrap().unwrap();
    assert_eq!(started.desk_task_key, svc.store.get_task(desk).unwrap().key);
    let admin = factory(&svc, None, Scope::HostAdmin);
    assert!(call(&admin, start("alice")).await.is_ok(), "an admin starts anyone's");
    let owners_phone = factory(&svc, Some("phone-o"), Scope::Control);
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    assert!(call(&owners_phone, start("carol")).await.is_ok(), "the owner starts anyone's, at Control too");

    let list = Request { method: "member.list".into(), ..Default::default() };
    let Ok(pb::result::Value::MemberList(listed)) = call(&bob, list.clone()).await else { panic!("a list") };
    assert_eq!(listed.you, "bob");
    let bobs = listed.members.iter().find(|m| m.handle == "bob").unwrap();
    assert_eq!((bobs.client_ids.as_slice(), bobs.rep_terminal_ids.as_slice()), (["phone-b".to_string()].as_slice(), [started.terminal_id.clone()].as_slice()));

    let looker = factory(&svc, Some("phone-r"), Scope::Read);
    let Ok(pb::result::Value::MemberList(read)) = call(&looker, list.clone()).await else { panic!("a list") };
    assert_eq!(read.claude_account, "", "the owner's email isn't for a device enrolled only to look");

    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    assert_eq!(call(&bob, remove.clone()).await.err().as_deref(), Some("scope-denied:"));
    let Ok(pb::result::Value::MemberList(after)) = call(&admin, remove).await else { panic!("removed") };
    assert_eq!(after.you, "", "the local socket is the owner");
    assert!(after.members.iter().all(|m| m.handle != "bob"));
    let pane = Uuid::from_slice(&started.terminal_id).unwrap();
    assert!(svc.store.get_terminal(pane).is_err(), "Bob's rep went with him");
}


/// `member.set` binds only devices this runner enrolled, so a typo can't
/// leave a teammate's real phone the owner's. Its own service, since the
/// fixture's reads the real `~/.ssh/authorized_keys`.
#[tokio::test]
async fn a_member_is_bound_only_to_enrolled_devices() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    crate::enrollment::enroll(
        &svc,
        &pb::ClientEnroll {
            public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone".into(),
            label: "Bob's phone".into(),
            client_id: "phone-b".into(),
            scope: Scope::Control as i32,
            shell_access: false,
            node_key: String::new(),
        },
    )
    .await
    .expect("the phone enrolls");
    let admin = factory(&svc, None, Scope::HostAdmin);
    let set = |device: &str| Request {
        method: "member.set".into(),
        payload: Some(request::Payload::MemberSet(pb::MemberSet {
            handle: "bob".into(),
            name: "Bob".into(),
            client_ids: vec![device.into()],
        })),
        ..Default::default()
    };
    assert_eq!(call(&admin, set("phne-b")).await.err().as_deref(), Some("invalid-argument:device"), "a typo");
    assert!(svc.store.member("bob").is_err(), "nothing was written");
    assert!(call(&admin, set("phone-b")).await.is_ok());
    assert_eq!(svc.store.member_of_device("phone-b").unwrap().as_deref(), Some("bob"));
}

//! A Claude login that stops working, and a member with none (ov-587): the
//! rules of `claude_login_renewal`, end to end through the supervisor's
//! listener and `needs_you.list` as each person's device reads it.

use std::sync::Arc;

use farcooler_agent_core::event::{AgentEvent, EndReason, FailureKind};
use farcooler_protocol::v1::{self as pb, ClaudeLoginState, NeedsYouKind, Request, Scope, request, response};
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::service::Service;
use crate::test_support::ScratchDir;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-claude.pl");

struct Runner {
    _dir: ScratchDir,
    svc: Arc<Service>,
    watcher: Arc<crate::watch::Watcher>,
    main: Uuid,
}

/// A runner with Alice, Bob and the owner, the stand-in in place of claude,
/// and a watcher listening for failed logins as a daemon's does.
async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    *svc.claude_logins.sign_ins.program.lock().unwrap() = Some(crate::claude_sign_in::Program {
        shell: "/bin/sh".into(),
        flag: "-c",
        claude: crate::service::shell_quote(FAKE),
        env: Vec::new(),
    });
    *svc.claude_logins.claimed.lock().unwrap() = Some(Some(crate::claude_logins::Version(2, 1, 301)));
    let watcher = crate::watch::Watcher::new(svc.clone());
    Runner { _dir: dir, svc, watcher, main }
}

impl Runner {
    fn who(&self, client_id: Option<&str>, scope: Scope) -> crate::rpc::RpcFactory {
        let peer = Peer { client_id: client_id.map(str::to_string), scope };
        crate::rpc::RpcFactory::new(self.svc.clone(), self.watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer)
    }

    /// Put `handle`'s directory in the state `claude auth login` leaves it.
    fn sign_in(&self, handle: &str) {
        let id = self.svc.store.ensure_member_login(handle).unwrap();
        let dir = crate::claude_logins::ensure_login_dir(self.svc.root_dir(), &id).unwrap();
        std::fs::write(std::path::Path::new(&dir).join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"alice@example.com"}}"#).unwrap();
    }

    /// A turn on `pane` that ended because claude could not authenticate.
    fn fail_auth(&self, pane: Uuid) {
        let ended = AgentEvent::TurnEnded { reason: EndReason::Failed { kind: FailureKind::Auth, detail: "Invalid API key".into() } };
        self.svc.agents().record(pane, vec![ended], &|_, _| {});
    }

    async fn rep(&self, handle: &str) -> Uuid {
        self.svc.start_rep(self.main, handle, "").await.expect("a rep").terminal.id
    }

    async fn needs_you(&self, who: &crate::rpc::RpcFactory) -> Vec<pb::NeedsYouItem> {
        match call(who, Request { method: "needs_you.list".into(), ..Default::default() }).await {
            Ok(pb::result::Value::NeedsYouList(l)) => l.items.into_iter().filter(|i| i.kind() == NeedsYouKind::ClaudeLogin).collect(),
            other => panic!("not a list: {other:?}"),
        }
    }

    fn state(&self, handle: &str) -> i32 {
        self.svc.member_claude_login(handle, true).state
    }
}

async fn call(who: &crate::rpc::RpcFactory, req: Request) -> Result<pb::result::Value, String> {
    use farcooler_transport::Handler as _;
    match who.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn rep_start(workspace: Uuid) -> impl Fn(&str) -> Request {
    move |handle| Request {
        method: "rep.start".into(),
        payload: Some(request::Payload::RepStart(pb::RepStart { workspace_id: crate::wire::id_bytes(workspace), handle: handle.into(), harness: String::new() })),
        ..Default::default()
    }
}

fn sign_in_finish(session: &[u8], code: &str) -> Request {
    Request {
        method: "claude.sign_in_finish".into(),
        payload: Some(request::Payload::ClaudeSignInFinish(pb::ClaudeSignInFinish { session_id: session.to_vec().into(), code: code.into() })),
        ..Default::default()
    }
}

fn sign_in_start(handle: &str) -> Request {
    Request {
        method: "claude.sign_in".into(),
        payload: Some(request::Payload::ClaudeSignInStart(pb::ClaudeSignInStart { handle: handle.into() })),
        ..Default::default()
    }
}

fn sign_out(handle: &str) -> Request {
    Request {
        method: "claude.sign_out".into(),
        payload: Some(request::Payload::ClaudeSignOut(pb::ClaudeSignOut { handle: handle.into() })),
        ..Default::default()
    }
}

/// Alice signs in on her phone with the stand-in, as the apps do.
async fn alice_signs_in_again(r: &Runner) {
    let alice = r.who(Some("phone-a"), Scope::Read);
    let Ok(pb::result::Value::ClaudeSignIn(session)) = call(&alice, sign_in_start("alice")).await else { panic!("no sign-in") };
    let done = call(&alice, sign_in_finish(&session.session_id, "ok-c0de#st4te")).await;
    assert!(matches!(done, Ok(pb::result::Value::MemberClaudeLogin(_))), "{done:?}");
}

/// Acceptance 1: Alice's agent fails to authenticate on her login, and Alice
/// is told, in her words with a button; the owner is told whose login, in
/// theirs and with no email and no button; Bob, on the same runner, is told
/// nothing; and a reader with no viewer (the runner-wide count) is told
/// nothing either.
#[tokio::test]
async fn a_members_failed_login_is_theirs_and_the_owners_alone() {
    let r = runner().await;
    r.sign_in("alice");
    r.sign_in("bob");
    let pane = r.rep("alice").await;
    assert_eq!(r.state("alice"), ClaudeLoginState::SignedIn as i32);

    r.fail_auth(pane);

    assert_eq!(r.state("alice"), ClaudeLoginState::StoppedWorking as i32);
    assert_eq!(r.state("bob"), ClaudeLoginState::SignedIn as i32, "Bob's login is his own");
    let runner = crate::push::runner_name();

    let alices = r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await;
    assert_eq!(alices.len(), 1, "{alices:?}");
    assert_eq!(alices[0].question, format!("Your Claude login on {runner} stopped working."));
    let actions: Vec<(&str, &str)> = alices[0].actions.iter().map(|a| (a.id.as_str(), a.title.as_str())).collect();
    assert_eq!(actions, vec![("sign_in", "Sign In Again")], "still there below Control scope");

    let owner = r.needs_you(&r.who(None, Scope::Control)).await;
    assert_eq!(owner.len(), 1, "{owner:?}");
    assert_eq!(owner[0].question, "Alice Chen’s Claude login stopped working");
    assert!(owner[0].actions.is_empty(), "it is Alice's to fix");
    assert!(!format!("{owner:?}").contains("example.com"), "no email: {owner:?}");

    assert!(r.needs_you(&r.who(Some("phone-b"), Scope::Control)).await.is_empty(), "another member never sees it");
    assert_eq!(r.watcher.needs_you_count().await, Some(0), "the runner-wide count has no member's login in it");
}

/// The owner's own login keeps today's failed-turn behavior: an owner's
/// agent that fails to authenticate marks no member's login, raises no login
/// item for anyone, and leaves every member signed in.
#[tokio::test]
async fn the_owners_own_failed_turn_is_unchanged() {
    let r = runner().await;
    r.sign_in("alice");
    let _alices = r.rep("alice").await;
    let worktree = r.svc.store.list_worktrees_in_order().unwrap().remove(0);
    let owners = r.svc.create_terminal(worktree.id, "Agent", "claude").await.expect("the owner's agent").id;
    assert!(r.svc.store.terminal_login(owners).unwrap().is_none(), "on the runner's own login");

    r.fail_auth(owners);

    assert_eq!(r.state("alice"), ClaudeLoginState::SignedIn as i32);
    assert!(r.svc.store.login_flags().unwrap().is_empty(), "no member's login was marked");
    for who in [r.who(None, Scope::Control), r.who(Some("phone-a"), Scope::Control)] {
        assert!(r.needs_you(&who).await.is_empty());
    }
    assert!(matches!(r.svc.note_auth_failed(owners), Ok(false)));
}

/// A deliberate Sign Out records `none`, and the panes still running on the
/// signed-out login fail their next turn without raising anything.
#[tokio::test]
async fn sign_out_does_not_nag() {
    let r = runner().await;
    r.sign_in("alice");
    let pane = r.rep("alice").await;
    r.fail_auth(pane);
    assert_eq!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.len(), 1, "stopped first");

    let out = call(&r.who(Some("phone-a"), Scope::Read), sign_out("alice")).await;
    assert!(matches!(out, Ok(pb::result::Value::MemberClaudeLogin(_))), "{out:?}");
    assert_eq!(r.state("alice"), ClaudeLoginState::None as i32);
    assert!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.is_empty(), "signing out ends the item");

    r.fail_auth(pane);

    assert_eq!(r.state("alice"), ClaudeLoginState::None as i32, "still `none`");
    assert!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.is_empty(), "the signed-out pane's failure nags no one");
    assert!(r.needs_you(&r.who(None, Scope::Control)).await.is_empty());
}

/// Signing in again clears `stopped_working` and resolves the item, for her
/// and for the owner.
#[tokio::test]
async fn signing_in_again_clears_it() {
    let r = runner().await;
    r.sign_in("alice");
    let pane = r.rep("alice").await;
    r.fail_auth(pane);
    assert_eq!(r.state("alice"), ClaudeLoginState::StoppedWorking as i32);
    assert_eq!(r.needs_you(&r.who(None, Scope::Control)).await.len(), 1);

    alice_signs_in_again(&r).await;

    assert_eq!(r.state("alice"), ClaudeLoginState::SignedIn as i32);
    assert!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.is_empty());
    assert!(r.needs_you(&r.who(None, Scope::Control)).await.is_empty());
    // A turn that began on the old token ends just after the sign-in: its
    // failure is the old login's last word, not news (L1).
    r.fail_auth(pane);
    assert_eq!(r.state("alice"), ClaudeLoginState::SignedIn as i32, "a stale failure does not re-raise it");
    assert!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.is_empty());
    // Past the grace, a failure is news again.
    let login = r.svc.store.member_login("alice").unwrap().unwrap();
    let later = crate::review::now_millis() + farcooler_store::claude_logins::SIGN_IN_GRACE_MS + 1_000;
    assert!(r.svc.store.mark_login_stopped("alice", &login, later).unwrap());
    assert_eq!(r.state("alice"), ClaudeLoginState::StoppedWorking as i32);
}

/// Bob's rep, or `farcooler needs-you` typed in any pane, comes in on the
/// local socket, which reads as the owner; it must be read by the pane's
/// login instead, so it is never told Alice's login stopped working (M1).
#[tokio::test]
async fn a_pane_reads_needs_you_as_its_own_login_not_the_owner() {
    let r = runner().await;
    r.sign_in("alice");
    r.sign_in("bob");
    let alices = r.rep("alice").await;
    let bobs = r.rep("bob").await;
    let owners = {
        let worktree = r.svc.store.list_worktrees_in_order().unwrap().remove(0);
        r.svc.create_terminal(worktree.id, "Agent", "claude").await.expect("the owner's agent").id
    };
    r.fail_auth(alices);
    let snapshot = r.svc.inventory.refresh().await;
    let pid_of = |id: Uuid| snapshot.claimants(id).into_iter().next().expect("a pane").stamp.pid as i32;
    let local = |pane: Uuid| r.who(None, Scope::Control).with_caller_pid(Some(pid_of(pane)));

    assert!(r.needs_you(&local(bobs)).await.is_empty(), "Bob's rep is not told Alice's login stopped");
    assert_eq!(r.needs_you(&local(alices)).await.len(), 1, "Alice's own pane is told, as Alice");
    assert_eq!(r.needs_you(&local(owners)).await.len(), 1, "the owner's pane is the owner");
    assert_eq!(r.needs_you(&r.who(None, Scope::Control)).await.len(), 1, "and so is a caller in no pane");
}

/// `member.list` tells the owner and the member themself their state, email
/// and stopped_working; another member sees only that they're signed in (M2).
#[tokio::test]
async fn another_member_is_not_told_a_login_stopped_working() {
    let r = runner().await;
    r.sign_in("alice");
    let pane = r.rep("alice").await;
    r.fail_auth(pane);
    let state_seen_by = |who: crate::rpc::RpcFactory| async move {
        match call(&who, Request { method: "member.list".into(), ..Default::default() }).await {
            Ok(pb::result::Value::MemberList(l)) => l.members.into_iter().find(|m| m.handle == "alice").and_then(|m| m.claude_login).map(|c| c.state),
            other => panic!("not a list: {other:?}"),
        }
    };
    let stopped = Some(ClaudeLoginState::StoppedWorking as i32);
    let signed_in = Some(ClaudeLoginState::SignedIn as i32);
    assert_eq!(state_seen_by(r.who(Some("phone-a"), Scope::Control)).await, stopped, "Alice");
    assert_eq!(state_seen_by(r.who(None, Scope::Control)).await, stopped, "the owner");
    assert_eq!(state_seen_by(r.who(Some("phone-b"), Scope::Control)).await, signed_in, "Bob only sees she is signed in");
}

/// A relaunched rep pane of a member who signed out is not started on the
/// login that is gone; they are asked to sign in and no pane starts (M3).
#[tokio::test]
async fn a_relaunch_of_a_signed_out_members_rep_asks_instead() {
    let r = runner().await;
    alice_signs_in_again(&r).await;
    let pane = r.rep("alice").await;
    r.svc.stop_terminal(pane).await.unwrap();
    call(&r.who(Some("phone-a"), Scope::Read), sign_out("alice")).await.unwrap();

    let relaunched = r.svc.relaunch_held(pane, false).await;
    assert!(matches!(relaunched, Err(farcooler_core::DomainError::Conflict { what: "claude_sign_in_needed" })), "{relaunched:?}");
    assert!(!r.svc.is_running(&r.svc.store.get_terminal(pane).unwrap()));
    assert_eq!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.len(), 1);
}

/// Acceptance 2: a member with no login is asked to sign in instead of being
/// given a rep, and nothing is started on the owner's login in its place.
/// Signing in resolves the ask, and the rep starts on her own.
#[tokio::test]
async fn a_member_with_no_login_gets_a_sign_in_ask_and_no_pane() {
    let r = runner().await;
    let alice = r.who(Some("phone-a"), Scope::Control);
    let panes_before = r.svc.store.list_worktrees_in_order().unwrap().iter().map(|w| r.svc.store.list_terminals_for_worktree(w.id).unwrap().len()).sum::<usize>();

    let refused = call(&alice, rep_start(r.main)("alice")).await;
    assert_eq!(refused.err().as_deref(), Some("resource-conflict:claude_sign_in_needed"));

    let panes_now = r.svc.store.list_worktrees_in_order().unwrap().iter().map(|w| r.svc.store.list_terminals_for_worktree(w.id).unwrap().len()).sum::<usize>();
    assert_eq!(panes_now, panes_before, "no pane of any login was started");
    let ask = r.needs_you(&alice).await;
    assert_eq!(ask.len(), 1, "{ask:?}");
    assert_eq!(ask[0].question, format!("Sign in to Claude on {} to start your rep.", crate::push::runner_name()));
    assert_eq!(ask[0].actions.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), vec!["Sign In"]);
    assert_eq!(r.needs_you(&r.who(None, Scope::Control)).await[0].question, "Alice Chen needs to sign in to Claude");
    assert!(r.needs_you(&r.who(Some("phone-b"), Scope::Control)).await.is_empty(), "Bob is not asked");

    alice_signs_in_again(&r).await;
    assert!(r.needs_you(&alice).await.is_empty(), "signing in ends the ask");
    let started = call(&alice, rep_start(r.main)("alice")).await;
    assert!(matches!(started, Ok(pb::result::Value::RepStarted(_))), "{started:?}");
    let rep = r.svc.store.rep_for(r.main, "alice").unwrap().expect("her rep").terminal;
    let on = r.svc.store.terminal_login(rep).unwrap().expect("a member's login is recorded");
    assert_eq!(on.handle, "alice", "on her own login, never the runner's");
}

/// A stopped rep whose member has signed out is not woken on the login that
/// is gone: its wake is refused as `rep.start` is, and asks them to sign in.
#[tokio::test]
async fn a_stopped_rep_of_a_signed_out_member_is_not_woken() {
    let r = runner().await;
    alice_signs_in_again(&r).await;
    let pane = r.rep("alice").await;
    r.svc.stop_terminal(pane).await.unwrap();
    call(&r.who(Some("phone-a"), Scope::Read), sign_out("alice")).await.unwrap();

    let woken = r.svc.resume_rep(pane).await;
    assert!(matches!(woken, Err(farcooler_core::DomainError::Conflict { what: "claude_sign_in_needed" })), "{woken:?}");
    assert!(!r.svc.is_running(&r.svc.store.get_terminal(pane).unwrap()), "still stopped");
    assert_eq!(r.needs_you(&r.who(Some("phone-a"), Scope::Read)).await.len(), 1, "she is asked to sign in");

    alice_signs_in_again(&r).await;
    assert!(r.svc.resume_rep(pane).await.is_ok(), "signed in, it wakes");
}

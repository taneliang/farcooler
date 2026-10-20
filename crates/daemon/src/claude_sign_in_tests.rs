//! A member signing in to Claude from their own device (ov-584), against a
//! stand-in for `claude auth login|status|logout` that replays what spike 0
//! measured on claude 2.1.301 (`tests/fixtures/fake-claude.pl`): a code
//! `ok-…#…` succeeds and signs the directory in as member@example.com.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, ClaudeLoginState, Request, Scope, request, response};
use farcooler_transport::Peer;

use super::*;
use crate::test_support::ScratchDir;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-claude.pl");

/// One sign-in test at a time. They share their log lines' callsites, and
/// tracing caches a callsite's interest as of the first thread to reach it:
/// one test reaching it with no subscriber while `the_code_never_reaches_the_log`
/// holds one can leave that test's log empty, which its own first assertion
/// catches.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A runner with Alice and Bob on their phones, Carol's phone released, and
/// the stand-in in place of claude, claiming 2.1.301; and this test's turn.
async fn runner() -> (ScratchDir, Arc<Service>, Uuid, tokio::sync::MutexGuard<'static, ()>) {
    let serial = SERIAL.lock().await;
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let main = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("alice", "Alice Chen", &["phone-a".to_string(), "mac-a".to_string()]).unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &["phone-c".to_string()]).unwrap();
    svc.store.set_member("carol", "Carol", &[]).unwrap();
    use_the_stand_in(&svc, FAKE);
    (dir, svc, main, serial)
}

fn use_the_stand_in(svc: &Service, claude: &str) {
    *lock(&svc.claude_logins.sign_ins.program) =
        Some(Program { shell: "/bin/sh".into(), flag: "-c", claude: crate::service::shell_quote(claude), env: Vec::new() });
    *lock(&svc.claude_logins.claimed) = Some(Some(Version(2, 1, 301)));
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

fn req(method: &str, payload: request::Payload) -> Request {
    Request { method: method.into(), payload: Some(payload), ..Default::default() }
}

fn start(handle: &str) -> Request {
    req("claude.sign_in", request::Payload::ClaudeSignInStart(pb::ClaudeSignInStart { handle: handle.into() }))
}

fn finish(session: &[u8], code: &str) -> Request {
    req(
        "claude.sign_in_finish",
        request::Payload::ClaudeSignInFinish(pb::ClaudeSignInFinish { session_id: session.to_vec().into(), code: code.into() }),
    )
}

fn cancel(session: &[u8]) -> Request {
    req("claude.sign_in_cancel", request::Payload::ClaudeSignInCancel(pb::ClaudeSignInCancel { session_id: session.to_vec().into() }))
}

fn sign_out(handle: &str) -> Request {
    req("claude.sign_out", request::Payload::ClaudeSignOut(pb::ClaudeSignOut { handle: handle.into() }))
}

async fn started(who: &crate::rpc::RpcFactory, handle: &str) -> pb::ClaudeSignIn {
    match call(who, start(handle)).await {
        Ok(pb::result::Value::ClaudeSignIn(s)) => s,
        other => panic!("not a sign-in: {other:?}"),
    }
}

fn login(answer: std::result::Result<pb::result::Value, String>) -> pb::MemberClaudeLogin {
    match answer {
        Ok(pb::result::Value::MemberClaudeLogin(l)) => l,
        other => panic!("not a login: {other:?}"),
    }
}

async fn members(who: &crate::rpc::RpcFactory) -> Vec<pb::Member> {
    let list = Request { method: "member.list".into(), ..Default::default() };
    match call(who, list).await {
        Ok(pb::result::Value::MemberList(l)) => l.members,
        other => panic!("not a list: {other:?}"),
    }
}

fn row<'a>(members: &'a [pb::Member], handle: &str) -> &'a pb::MemberClaudeLogin {
    members.iter().find(|m| m.handle == handle).and_then(|m| m.claude_login.as_ref()).expect("a login row")
}

/// The member's canonical directory, as a launch would hand it to claude.
fn dir_of(svc: &Service, handle: &str) -> String {
    let login_id = svc.store.member_login(handle).unwrap().expect("a login");
    crate::claude_logins::ensure_login_dir(svc.root_dir(), &login_id).unwrap()
}

/// Every terminal row on the runner.
fn terminals(svc: &Service) -> usize {
    svc.store.list_worktrees_in_order().unwrap().iter().map(|w| svc.store.list_terminals_for_worktree(w.id).unwrap().len()).sum()
}

async fn panes(svc: &Service) -> String {
    svc.tmux.run(&["list-panes", "-a", "-F", "#{pane_id}"]).await.map(|o| o.stdout).unwrap_or_default()
}

impl SignIns {
    /// The pid of `handle`'s sign-in process, for a test to watch it end.
    fn pid_of(&self, handle: &str) -> Option<u32> {
        lock(&self.live).get(handle).and_then(|l| l.child.id())
    }
}

/// Alice signs in from her own phone, at Read: the page is claude's, the
/// code goes in, and her row says she's signed in as the account claude
/// signed her in to. Every command named her directory exactly as a launch
/// does, turned the browser off, and never carried the code.
#[tokio::test]
async fn a_member_signs_in_from_their_own_device() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let before = row(&members(&alice).await, "alice").clone();
    assert_eq!(before.state, ClaudeLoginState::None as i32, "not signed in yet");

    let sign_in = started(&alice, "alice").await;
    assert!(sign_in.url.starts_with("https://claude.com/cai/oauth/authorize?"), "{}", sign_in.url);
    assert!(sign_in.expires_at > 0);
    let code = "ok-c0de7oken#st4te";
    let done = login(call(&alice, finish(&sign_in.session_id, &format!("  {code}\n"))).await);
    assert_eq!((done.state, done.email.as_str()), (ClaudeLoginState::SignedIn as i32, "member@example.com"));

    let dir = dir_of(&svc, "alice");
    let ran = lock(&svc.claude_logins.sign_ins.ran).clone();
    assert_eq!(ran.len(), 2, "the login, then its status: {ran:?}");
    for payload in &ran {
        assert!(payload.contains(&format!("BROWSER=/usr/bin/false CLAUDE_CONFIG_DIR={} ", shell_quote(&dir))), "{payload}");
        assert!(!payload.contains("c0de7oken"), "the code is never on a command line: {payload}");
    }
    assert!(ran[0].ends_with(" auth login") && ran[1].ends_with(" auth status --json"), "{ran:?}");
    assert!(ran.iter().all(|p| p.starts_with("BROWSER=")), "one simple command, for any login shell: {ran:?}");
    assert!(!svc.claude_logins.sign_ins.under_way("alice"), "it's over");
}

/// Her Claude email is hers and the owner's to read; Bob sees only that
/// she's signed in.
#[tokio::test]
async fn a_members_claude_email_is_theirs_and_the_owners() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    login(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await);

    let signed_in = ClaudeLoginState::SignedIn as i32;
    let owner = factory(&svc, None, Scope::HostAdmin);
    let bob = factory(&svc, Some("phone-b"), Scope::HostAdmin);
    for (who, email, what) in [(&alice, "member@example.com", "Alice"), (&owner, "member@example.com", "the owner"), (&bob, "", "Bob, a runner admin")] {
        let seen = row(&members(who).await, "alice").clone();
        assert_eq!((seen.state, seen.email.as_str()), (signed_in, email), "{what}");
    }
    assert_eq!(row(&members(&bob).await, "bob").state, ClaudeLoginState::None as i32, "Bob never signed in");
}

/// Only that member, from their own device, starts, finishes or cancels
/// their sign-in, or signs them out (acceptance 1). The owner, another
/// member (a runner admin included), a released device, and a process in a
/// pane that names her phone are all refused. Her sign-in, under way through
/// every refusal, is in no terminal row and no pane, and she finishes it.
#[tokio::test]
async fn only_the_member_from_their_own_device_reaches_their_sign_in() {
    let (dir, svc, main, _serial) = runner().await;
    let rep = svc.start_rep(main, "bob", "").await.expect("Bob's rep").terminal;
    let agent = svc.create_terminal(rep.worktree_id, "Agent 2", "claude").await.expect("an agent");
    let shell = svc.create_terminal(rep.worktree_id, "Shell", "shell").await.expect("a shell");
    let snapshot = svc.inventory.refresh().await;
    let pid_of = |id: Uuid| snapshot.claimants(id).into_iter().next().expect("a pane").stamp.pid as i32;
    let (rows_before, panes_before) = (terminals(&svc), panes(&svc).await);

    let owner = crate::test_support::Outside::a_person(dir.path(), 6589);
    let in_pane = |pane: Uuid| factory(&svc, Some("phone-a"), Scope::Control).with_caller_pid(Some(pid_of(pane)));
    let refused = [
        (factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(owner.pid)), "not-yours:not_yours", "the owner"),
        (factory(&svc, Some("phone-b"), Scope::Control), "not-yours:not_yours", "another member"),
        (factory(&svc, Some("phone-b"), Scope::HostAdmin), "not-yours:not_yours", "another member who's a runner admin"),
        (factory(&svc, Some("phone-c"), Scope::HostAdmin), "not-yours:not_yours", "a released device"),
        (in_pane(agent.id), "resource-conflict:own_device_only", "an agent's pane naming her phone"),
        (in_pane(shell.id), "resource-conflict:own_device_only", "a shell pane naming her phone"),
        (in_pane(rep.id), "resource-conflict:rep_messenger", "a rep's pane naming her phone"),
    ];
    for (who, word, what) in &refused {
        assert_eq!(call(who, start("alice")).await.err().as_deref(), Some(*word), "start, {what}");
        assert_eq!(call(who, sign_out("alice")).await.err().as_deref(), Some(*word), "sign out, {what}");
    }
    assert!(!svc.claude_logins.sign_ins.under_way("alice"), "no refusal started one");

    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    for (who, word, what) in &refused {
        assert_eq!(call(who, finish(&sign_in.session_id, "ok-x#y")).await.err().as_deref(), Some(*word), "finish, {what}");
        assert_eq!(call(who, cancel(&sign_in.session_id)).await.err().as_deref(), Some(*word), "cancel, {what}");
    }
    // Her own other device holds no authority over this phone's sign-in.
    let her_mac = factory(&svc, Some("mac-a"), Scope::Control);
    assert_eq!(call(&her_mac, finish(&sign_in.session_id, "ok-x#y")).await.err().as_deref(), Some("not-yours:not_yours"));
    assert_eq!(call(&her_mac, cancel(&sign_in.session_id)).await.err().as_deref(), Some("not-yours:not_yours"));
    assert!(svc.claude_logins.sign_ins.under_way("alice"), "still hers, through every refusal");
    assert_eq!(terminals(&svc), rows_before, "a sign-in is no terminal row");
    assert_eq!(panes(&svc).await, panes_before, "nor a pane");
    let done = login(call(&alice, finish(&sign_in.session_id, "ok-x#y")).await);
    assert_eq!(done.state, ClaudeLoginState::SignedIn as i32);
}

/// Every process of a sign-in's group, by its group id.
fn group_members(group: u32) -> Vec<String> {
    let out = std::process::Command::new("ps").args(["-axo", "pgid=,stat=,command="]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.split_whitespace().next() == Some(&group.to_string()) && !l.split_whitespace().nth(1).is_some_and(|s| s.starts_with('Z')))
        .map(str::to_string)
        .collect()
}

/// Poll for every process of `group` to be gone, for up to 30 s.
async fn group_gone(group: u32) -> bool {
    for _ in 0..300 {
        if group_members(group).is_empty() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A finished sign-in leaves nothing of its group behind, its watchdog
/// included.
#[tokio::test]
async fn a_finished_sign_in_leaves_no_process() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    let group = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    assert!(group_members(group).len() >= 2, "claude and its watchdog: {:?}", group_members(group));
    login(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await);
    assert!(group_gone(group).await, "{:?}", group_members(group));
}

/// A member's sign-in that's refused leaves no claude running, and starting
/// again, cancelling, or their removal ends the one under way, its process
/// with it.
#[tokio::test]
async fn a_sign_in_ends_with_its_process() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let first = started(&alice, "alice").await;
    let first_pid = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    let second = started(&alice, "alice").await;
    assert!(group_gone(first_pid).await, "starting again ends the first, claude and all");
    assert_eq!(call(&alice, finish(&first.session_id, "ok-a#b")).await.err().as_deref(), Some("not-found:"), "the first is over");

    let second_pid = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    let cancelled = login(call(&alice, cancel(&second.session_id)).await);
    assert_eq!(cancelled.state, ClaudeLoginState::None as i32);
    assert!(group_gone(second_pid).await, "cancelling ends it");

    started(&alice, "alice").await;
    let third_pid = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    let owner = factory(&svc, None, Scope::HostAdmin);
    let remove = req("member.remove", request::Payload::MemberRemove(pb::MemberRemove { handle: "alice".into() }));
    call(&owner, remove).await.expect("removed");
    assert!(!svc.claude_logins.sign_ins.under_way("alice"));
    assert!(group_gone(third_pid).await, "a removal ends it");
}

/// A sign-in the daemon never ends (it crashed, or was killed) ends itself
/// a minute past its life, claude and all.
#[tokio::test]
async fn a_sign_in_ends_itself_when_the_daemon_cant() {
    let (_dir, svc, _main, _serial) = runner().await;
    *lock(&svc.claude_logins.sign_ins.waits) = Waits { lifetime: Duration::ZERO, margin: Duration::ZERO, ..Waits::default() };
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    started(&alice, "alice").await;
    let pid = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    // The daemon goes without dropping it.
    let live = lock(&svc.claude_logins.sign_ins.live).remove("alice").expect("hers");
    std::mem::forget(live);
    assert!(group_gone(pid).await, "it ended itself, claude and all");
}

/// A code with no `#`, half of one, or a control character is refused
/// before it reaches claude, and the sign-in goes on. A code claude refuses
/// ends the sign-in.
#[tokio::test]
async fn a_code_that_isnt_whole_is_refused_and_the_sign_in_goes_on() {
    for bad in ["", "abc", "abc#", "#abc", "ok-a\u{1b}[201~#b", "ok-a#b\nok-c#d"] {
        assert!(pasted_code(bad).is_err(), "{bad:?}");
    }
    assert!(pasted_code(&format!("ok-a#{}", "b".repeat(MAX_CODE))).is_err(), "too long");
    assert_eq!(pasted_code(" ok-a#b \n").unwrap(), "ok-a#b");

    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    assert_eq!(call(&alice, finish(&sign_in.session_id, "ok-abc")).await.err().as_deref(), Some("invalid-argument:sign_in_code"));
    assert!(svc.claude_logins.sign_ins.under_way("alice"), "it goes on");
    assert_eq!(call(&alice, finish(&sign_in.session_id, "wrong#code")).await.err().as_deref(), Some("resource-conflict:code_refused"));
    assert!(!svc.claude_logins.sign_ins.under_way("alice"), "claude refused it, so it's over");
    let row = row(&members(&alice).await, "alice").clone();
    assert_eq!(row.state, ClaudeLoginState::None as i32);
}

/// The code and claude's output never reach the log, whatever happens to
/// them: a token-shaped code that claude refuses, and one it takes.
#[tokio::test]
async fn the_code_never_reaches_the_log() {
    let sink = Sink::default();
    let subscriber = tracing_subscriber::fmt().with_writer(sink.clone()).with_ansi(false).with_max_level(tracing::Level::TRACE).finish();
    let (_dir, svc, _main, _serial) = runner().await;
    let _guard = tracing::subscriber::set_default(subscriber);
    // A callsite another test's thread reached first, with no subscriber,
    // may have cached "never": ask every callsite again.
    tracing::callsite::rebuild_interest_cache();
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let token = "sk-ant-oat01-0123456789abcdefghij";
    let sign_in = started(&alice, "alice").await;
    let refused = call(&alice, finish(&sign_in.session_id, &format!("{token}#state0123456789"))).await;
    assert_eq!(refused.err().as_deref(), Some("resource-conflict:code_refused"));
    let sign_in = started(&alice, "alice").await;
    login(call(&alice, finish(&sign_in.session_id, "ok-0123456789abcdefghij#state0123456789")).await);
    let logged = String::from_utf8(lock(&sink.0).clone()).unwrap();
    assert!(logged.contains("claude refused a sign-in code"), "it logged the refusal: {logged}");
    for secret in ["sk-ant", "0123456789abcdefghij", "state0123456789", "FAKEchallenge", "oauth/authorize", "member@example.com", "Login successful"] {
        assert!(!logged.contains(secret), "{secret} reached the log: {logged}");
    }
}

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Sink {
        self.clone()
    }
}

/// A member is sent only to a sign-in page this runner knows: never another
/// host, a user in the address, a port, plain http, another path, or one
/// with a space or a control character in it.
#[test]
fn a_sign_in_page_is_only_one_this_runner_knows() {
    let page = "https://claude.com/cai/oauth/authorize?code=true&state=s";
    assert_eq!(sign_in_page(&format!("If the browser didn't open, visit: {page}")).as_deref(), Some(page));
    assert!(sign_in_page("visit: https://platform.claude.com/oauth/authorize?x=1").is_some(), "the Console's");
    for bad in [
        "visit: https://evil.example/cai/oauth/authorize?x=1",
        "visit: https://claude.com@evil.example/cai/oauth/authorize?x=1",
        "visit: https://claude.com.evil.example/cai/oauth/authorize?x=1",
        "visit: https://claude.com:8443/cai/oauth/authorize?x=1",
        "visit: http://claude.com/cai/oauth/authorize?x=1",
        "visit: https://claude.com/cai/oauth/authorize/../../evil?x=1",
        "visit: https://claude.com/cai/oauth/authorize",
        "visit: https://claude.com/cai/oauth/authorize?x=\u{7}",
        "Opening browser to sign in…",
    ] {
        assert_eq!(sign_in_page(bad), None, "{bad}");
    }
    assert_eq!(sign_in_page(&format!("visit: {page}&{}", "a".repeat(MAX_URL))), None, "too long");
}

/// A claude that prints any other page is refused, and nothing is sent on.
#[tokio::test]
async fn a_claude_that_gives_another_page_is_refused() {
    let (dir, svc, _main, _serial) = runner().await;
    let script = dir.path().join("evil-claude.sh");
    std::fs::write(&script, "#!/bin/sh\necho \"If the browser didn't open, visit: https://evil.example/cai/oauth/authorize?x=1\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    use_the_stand_in(&svc, script.to_str().unwrap());
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    assert_eq!(call(&alice, start("alice")).await.err().as_deref(), Some("resource-conflict:sign_in_unreadable"));
    assert!(!svc.claude_logins.sign_ins.under_way("alice"));
}

/// Below the claude that takes a pasted code and names its directory, a
/// sign-in is refused before anything runs.
#[tokio::test]
async fn a_sign_in_needs_a_claude_new_enough() {
    let (_dir, svc, _main, _serial) = runner().await;
    *lock(&svc.claude_logins.claimed) = Some(Some(Version(2, 1, 267)));
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    assert_eq!(call(&alice, start("alice")).await.err().as_deref(), Some("resource-conflict:sign_in_too_old"));
    assert!(lock(&svc.claude_logins.sign_ins.ran).is_empty(), "nothing ran");
}

/// Signing out runs `claude auth logout` in her directory, and her row
/// says she's signed out.
#[tokio::test]
async fn a_member_signs_out() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    login(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await);
    let out = login(call(&alice, sign_out("alice")).await);
    assert_eq!((out.state, out.email.as_str()), (ClaudeLoginState::None as i32, ""));
    let dir = dir_of(&svc, "alice");
    let ran = lock(&svc.claude_logins.sign_ins.ran).clone();
    let logout = format!("BROWSER=/usr/bin/false CLAUDE_CONFIG_DIR={} {} auth logout", shell_quote(&dir), shell_quote(FAKE));
    let at = ran.iter().position(|p| *p == logout).unwrap_or_else(|| panic!("no logout: {ran:?}"));
    // Its confirmation read the same directory.
    let status = format!("BROWSER=/usr/bin/false CLAUDE_CONFIG_DIR={} {} auth status --json", shell_quote(&dir), shell_quote(FAKE));
    assert_eq!(ran.get(at + 1), Some(&status), "{ran:?}");
}

/// A removal signs the member out with `claude auth logout` before it
/// deletes their login, and says so; a sign-out it can't confirm is
/// `revoke`, the one case the admin is sent to claude.ai.
#[tokio::test]
async fn a_removal_signs_the_member_out_first() {
    let (_dir, svc, _main, _serial) = runner().await;
    for handle in ["alice", "bob"] {
        let phone = if handle == "alice" { "phone-a" } else { "phone-b" };
        let member = factory(&svc, Some(phone), Scope::Read);
        let sign_in = started(&member, handle).await;
        login(call(&member, finish(&sign_in.session_id, "ok-a#b")).await);
    }
    let (alices, bobs) = (dir_of(&svc, "alice"), dir_of(&svc, "bob"));
    let owner = factory(&svc, None, Scope::HostAdmin);
    let remove = |handle: &str| req("member.remove", request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() }));
    let removed = |answer: std::result::Result<pb::result::Value, String>| match answer {
        Ok(pb::result::Value::MemberList(l)) => l.removed.expect("what was removed"),
        other => panic!("{other:?}"),
    };

    lock(&svc.claude_logins.sign_ins.ran).clear();
    let alice_gone = removed(call(&owner, remove("alice")).await);
    assert!(alice_gone.failed.is_empty(), "{:?}", alice_gone.failed);
    let ran = lock(&svc.claude_logins.sign_ins.ran).clone();
    assert!(ran.iter().any(|p| p.contains(&format!("CLAUDE_CONFIG_DIR={} ", shell_quote(&alices))) && p.ends_with(" auth logout")), "{ran:?}");
    assert!(ran.iter().all(|p| !p.contains(&bobs)), "never Bob's: {ran:?}");

    // Bob's directory can't be written, so claude can't sign him out.
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&bobs, std::fs::Permissions::from_mode(0o500)).unwrap();
    let bob_gone = removed(call(&owner, remove("bob")).await);
    std::fs::set_permissions(&bobs, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(bob_gone.failed.contains(&"revoke".to_string()), "{:?}", bob_gone.failed);
}

/// `auth status --json` read tolerantly: the measured signed-out answer,
/// and anything that isn't one.
#[test]
fn a_status_is_read_for_its_directory() {
    let out = "{\n  \"loggedIn\": false,\n  \"authMethod\": \"none\",\n  \"configDirectory\": \"/r/claude/x\"\n}\n";
    assert_eq!(status_of(out), Some((false, "/r/claude/x".to_string())));
    assert_eq!(status_of("{\"loggedIn\": true, \"configDirectory\": \"/d\", \"email\": \"a@b\"}"), Some((true, "/d".to_string())));
    assert_eq!(status_of("Logged in"), None);
    assert_eq!(status_of("{\"loggedIn\": true}"), None, "a status that names no directory isn't trusted");
}

/// A stand-in script at `path`, executable.
fn script(path: &std::path::Path, body: &str) -> String {
    std::fs::write(path, body).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_str().unwrap().to_string()
}

/// Every login shell a runner may have, fish included: its syntax is not
/// POSIX, and it's the owner's (review H1). CI installs fish, so a CI run
/// without it fails rather than skipping the half that differs.
fn login_shells() -> Vec<String> {
    let mut found: Vec<String> =
        ["/bin/sh", "/bin/bash", "/bin/zsh"].iter().filter(|p| std::path::Path::new(p).exists()).map(|p| p.to_string()).collect();
    match farcooler_core::programs::find("fish") {
        Some(fish) => found.push(fish.display().to_string()),
        None if std::env::var_os("CI").is_some() => panic!("fish is not installed, and CI must run the fish half of this test"),
        None => eprintln!("fish is not installed here; the fish half of this test did not run"),
    }
    found
}

/// A sign-in starts, finishes, reads its status and signs out under every
/// login shell, fish's included, run `-ilc` as a pane runs it.
#[tokio::test]
async fn a_sign_in_works_under_every_login_shell() {
    let (dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    // As a pane runs it, a login and interactive shell, but with an empty
    // home, so no rc file of whoever runs this reads in.
    let home = dir.path().join("empty-home");
    std::fs::create_dir_all(&home).unwrap();
    let home = home.to_str().unwrap().to_string();
    let env = vec![("HOME".to_string(), home.clone()), ("XDG_CONFIG_HOME".to_string(), format!("{home}/.config")), ("ZDOTDIR".to_string(), home.clone())];
    for shell in login_shells() {
        *lock(&svc.claude_logins.sign_ins.program) = Some(Program { shell: shell.clone(), flag: "-ilc", claude: shell_quote(FAKE), env: env.clone() });
        let sign_in = match call(&alice, start("alice")).await {
            Ok(pb::result::Value::ClaudeSignIn(s)) => s,
            other => panic!("{shell}: no sign-in: {other:?}"),
        };
        let done = login(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await);
        assert_eq!(done.state, ClaudeLoginState::SignedIn as i32, "{shell}");
        let out = login(call(&alice, sign_out("alice")).await);
        assert_eq!(out.state, ClaudeLoginState::None as i32, "{shell}");
    }
}

/// No more than `MAX_STARTS` starts a minute for one member (checklist 16),
/// and another member's aren't counted against them.
#[tokio::test]
async fn a_member_starts_at_most_a_few_sign_ins_a_minute() {
    let (_dir, svc, _main, _serial) = runner().await;
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    for _ in 0..5 {
        started(&alice, "alice").await;
    }
    assert_eq!(call(&alice, start("alice")).await.err().as_deref(), Some("resource-conflict:sign_in_too_often"));
    started(&factory(&svc, Some("phone-b"), Scope::Read), "bob").await;
}

/// `claude auth logout` is never run in the runner's own directory, which
/// would sign the owner out (review: the guard was untested).
#[tokio::test]
async fn the_runners_own_login_is_never_signed_out() {
    let (_dir, svc, _main, _serial) = runner().await;
    let runner_dir = svc.claude_logins.runner.clone().expect("the runner's directory");
    std::fs::create_dir_all(&runner_dir).unwrap();
    let config = runner_dir.join(".claude.json");
    std::fs::write(&config, r#"{"oauthAccount":{"emailAddress":"owner@example.com"}}"#).unwrap();
    let program = svc.sign_in_program().unwrap();
    assert!(!svc.claude_logout(&program, runner_dir.to_str().unwrap()).await);
    assert!(lock(&svc.claude_logins.sign_ins.ran).is_empty(), "nothing ran");
    assert_eq!(crate::people::account_in(&config).as_deref(), Some("owner@example.com"), "still signed in");
}

/// A finish that times out after its sign-in's life is over ends it, and
/// never puts back a sign-in that would outlive its expiry (review M1).
#[tokio::test]
async fn a_late_finish_ends_an_expired_sign_in() {
    let (dir, svc, _main, _serial) = runner().await;
    // A claude that takes the code and never answers.
    let hangs = script(&dir.path().join("hangs.sh"), &format!("#!/bin/sh\n{{ IFS= read -r line; sleep 120; }} | {} \"$@\"\n", shell_quote(FAKE)));
    use_the_stand_in(&svc, &hangs);
    *lock(&svc.claude_logins.sign_ins.waits) = Waits { finish: Duration::from_millis(500), ..Waits::default() };
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    assert_eq!(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await.err().as_deref(), Some("resource-conflict:code_refused"));
    assert!(svc.claude_logins.sign_ins.under_way("alice"), "within its life, it goes on");

    // Its life ends while the finish waits: long enough for a slow start.
    *lock(&svc.claude_logins.sign_ins.waits) = Waits { finish: Duration::from_secs(6), lifetime: Duration::from_secs(5), ..Waits::default() };
    let sign_in = started(&alice, "alice").await;
    assert_eq!(call(&alice, finish(&sign_in.session_id, "ok-a#b")).await.err().as_deref(), Some("resource-conflict:code_refused"));
    assert!(!svc.claude_logins.sign_ins.under_way("alice"), "past its life, it's over");
}

/// A sign-in whose claude has ended and been reaped never kills its old
/// group id when it's dropped: the id may be someone else's now (review M1).
#[tokio::test]
async fn an_ended_sign_in_kills_no_one_elses_group() {
    let (_dir, svc, _main, _serial) = runner().await;
    use std::os::unix::process::CommandExt as _;
    let mut victim = std::process::Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
    let mut ended = tokio::process::Command::new("true").spawn().unwrap();
    ended.wait().await.unwrap();
    let live = Live {
        id: Uuid::now_v7(),
        client_id: None,
        login_id: String::new(),
        dir: String::new(),
        group: Some(victim.id() as i32),
        expires: Instant::now(),
        child: ended,
        stdin: None,
    };
    drop(live);
    std::thread::sleep(Duration::from_millis(200));
    let still = victim.try_wait().unwrap().is_none();
    let _ = victim.kill();
    let _ = victim.wait();
    assert!(still, "a reused group id was killed");
    drop(svc);
}

/// A removal while a finish is under way waits for it, so what that finish
/// signed in is wiped with the rest (review M2).
#[tokio::test]
async fn a_removal_waits_for_a_finish_under_way() {
    let (dir, svc, _main, _serial) = runner().await;
    // A claude that takes two seconds over the code.
    let slow = script(&dir.path().join("slow.sh"), &format!("#!/bin/sh\nif [ \"$2\" = login ]; then {{ IFS= read -r line; sleep 2; printf '%s\\n' \"$line\"; }} | {fake} \"$@\"; else exec {fake} \"$@\"; fi\n", fake = shell_quote(FAKE)));
    use_the_stand_in(&svc, &slow);
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    let alices = dir_of(&svc, "alice");
    let finishing = {
        let alice = factory(&svc, Some("phone-a"), Scope::Read);
        let session = sign_in.session_id.clone();
        tokio::spawn(async move { call(&alice, finish(&session, "ok-a#b")).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let owner = factory(&svc, None, Scope::HostAdmin);
    let remove = req("member.remove", request::Payload::MemberRemove(pb::MemberRemove { handle: "alice".into() }));
    call(&owner, remove).await.expect("removed");
    assert!(finishing.is_finished(), "the removal waited for the finish");
    assert!(finishing.await.unwrap().is_err(), "a finish for someone removed is refused");
    assert!(!std::path::Path::new(&alices).exists(), "her directory is gone");
    let service = crate::member_removal::keychain_service(&alices);
    assert!(lock(&svc.claude_logins.keychain.deleted).contains(&service), "her Keychain entry was deleted");
}

/// A finish whose member's row goes while claude takes the code wipes what
/// claude signed in, itself: signed out, Keychain entry and directory gone
/// (review M2). Here nothing else wipes it.
#[tokio::test]
async fn a_finish_for_someone_removed_meanwhile_wipes_its_login() {
    let (dir, svc, _main, _serial) = runner().await;
    let slow = script(&dir.path().join("slow.sh"), &format!("#!/bin/sh\nif [ \"$2\" = login ]; then {{ IFS= read -r line; sleep 2; printf '%s\\n' \"$line\"; }} | {fake} \"$@\"; else exec {fake} \"$@\"; fi\n", fake = shell_quote(FAKE)));
    use_the_stand_in(&svc, &slow);
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    let alices = dir_of(&svc, "alice");
    let finishing = {
        let alice = factory(&svc, Some("phone-a"), Scope::Read);
        let session = sign_in.session_id.clone();
        tokio::spawn(async move { call(&alice, finish(&session, "ok-a#b")).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    svc.store.remove_member("alice").unwrap();
    assert_eq!(finishing.await.unwrap().err().as_deref(), Some("resource-conflict:login_gone"));
    assert!(!std::path::Path::new(&alices).exists(), "her directory is gone");
    let service = crate::member_removal::keychain_service(&alices);
    assert!(lock(&svc.claude_logins.keychain.deleted).contains(&service), "her Keychain entry was deleted");
    let ran = lock(&svc.claude_logins.sign_ins.ran).clone();
    assert!(ran.iter().any(|p| p.ends_with(" auth logout") && p.contains(&shell_quote(&alices))), "{ran:?}");
}

/// A finish that times out while a removal waits for it doesn't put its
/// sign-in back for the rest of its life: the removal ends it once the
/// finish lets go, and nothing of it is left running (re-review, M2's last
/// path).
#[tokio::test]
async fn a_finish_that_times_out_during_a_removal_leaves_no_sign_in() {
    let (dir, svc, _main, _serial) = runner().await;
    let hangs = script(&dir.path().join("hangs.sh"), &format!("#!/bin/sh\n{{ IFS= read -r line; sleep 120; }} | {} \"$@\"\n", shell_quote(FAKE)));
    use_the_stand_in(&svc, &hangs);
    *lock(&svc.claude_logins.sign_ins.waits) = Waits { finish: Duration::from_secs(2), ..Waits::default() };
    let alice = factory(&svc, Some("phone-a"), Scope::Read);
    let sign_in = started(&alice, "alice").await;
    let group = svc.claude_logins.sign_ins.pid_of("alice").expect("running");
    let finishing = {
        let alice = factory(&svc, Some("phone-a"), Scope::Read);
        let session = sign_in.session_id.clone();
        tokio::spawn(async move { call(&alice, finish(&session, "ok-a#b")).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let owner = factory(&svc, None, Scope::HostAdmin);
    let remove = req("member.remove", request::Payload::MemberRemove(pb::MemberRemove { handle: "alice".into() }));
    call(&owner, remove).await.expect("removed");
    assert!(finishing.await.unwrap().is_err());
    assert!(!svc.claude_logins.sign_ins.under_way("alice"), "her sign-in outlived her removal");
    assert!(group_gone(group).await, "{:?}", group_members(group));
}

/// A process with a claude above it, in no pane, that names the member's
/// phone is refused: an agent outside the runner's panes (or in a shell pane
/// it adopted) can't sign a member in by forging their device (ov-620's
/// `caller_seat`), nor can a command it left behind (ov-649). A person's
/// process outside the runner, naming the same phone, signs in. A copy of
/// perl named `claude` stands in for the agent.
#[tokio::test]
async fn a_process_under_an_agent_is_refused_whatever_device_it_names() {
    use std::io::BufRead as _;
    let (dir, svc, _main, _serial) = runner().await;
    let claude = dir.path().join("fake-agent").join("claude");
    std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
    let copied = std::process::Command::new("cp").arg(farcooler_core::programs::find("perl").expect("perl")).arg(&claude).status().unwrap();
    assert!(copied.success());
    let mut agent = std::process::Command::new(&claude)
        .args(["-e", "$| = 1; my $p = fork(); if (!$p) { exec 'sleep', '6584'; } print \"$p\\n\"; waitpid($p, 0);"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the fake claude");
    let mut line = String::new();
    std::io::BufReader::new(agent.stdout.as_mut().unwrap()).read_line(&mut line).unwrap();
    let child: i32 = line.trim().parse().expect("its child's pid");

    let forged = factory(&svc, Some("phone-a"), Scope::Control).with_caller_pid(Some(child));
    let refused = (call(&forged, start("alice")).await, call(&forged, sign_out("alice")).await);
    // SAFETY: signals to the processes this test started.
    unsafe {
        libc::kill(child, libc::SIGTERM);
    }
    let _ = agent.wait();
    assert_eq!(refused.0.err().as_deref(), Some("resource-conflict:person_only"), "start");
    assert_eq!(refused.1.err().as_deref(), Some("resource-conflict:person_only"), "sign out");
    assert!(!svc.claude_logins.sign_ins.under_way("alice"));

    // A command the claude left behind with `(cmd &)`, orphaned to pid 1
    // with no terminal, is no person either (ov-649).
    let left = crate::test_support::Outside::left_by(&claude, 6586);
    let leftover = factory(&svc, Some("phone-a"), Scope::Control).with_caller_pid(Some(left.pid));
    assert_eq!(call(&leftover, start("alice")).await.err().as_deref(), Some("resource-conflict:person_only"), "left behind");
    drop(left);

    // A person's process outside the runner: her phone's session, under a
    // stand-in for root's `sshd`, orphaned to pid 1.
    let sshd = crate::test_support::Outside::stand_in(&dir.path().join("ssh").join("sshd"));
    let person = crate::test_support::Outside::under_root(&sshd, 6585);
    let theirs = factory(&svc, Some("phone-a"), Scope::Control).with_caller_pid(Some(person.pid));
    let started = call(&theirs, start("alice")).await;
    drop(person);
    assert!(matches!(started, Ok(pb::result::Value::ClaudeSignIn(_))), "{started:?}");
}

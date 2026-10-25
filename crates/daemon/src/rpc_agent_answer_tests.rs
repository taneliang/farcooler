//! `terminal.agent_answer` for an ask a claude TUI's hook is holding
//! (`hook_asks`), rather than one an ACP shim is waiting on.
//!
//! Its own `Service`, not `test_support::fixture`'s, because that one enrolls
//! into the real `~/.ssh/authorized_keys`, and these tests enroll a device.
//! The seat tests (ov-647) enroll nothing and need the fixture's panes:
//! `seat_tests`, below.

use std::sync::Arc;

use farcooler_agent_hooks::wire::Decision;
use farcooler_protocol::v1::{AgentAnswer, ClientEnroll, Request, Scope, TerminalIntent, request, response};
use farcooler_transport::Handler;

use super::*;
use crate::rpc::RpcFactory;

/// A public key nobody holds the private half of.
const PHONE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone";

struct Runner {
    _dir: tempfile::TempDir,
    svc: Arc<Service>,
    terminal: Uuid,
}

/// A runner with one Terminal-mode claude pane, and a phone enrolled as
/// `phone-1` under `label`.
async fn a_runner(label: &str) -> Runner {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let keys = ssh.join("authorized_keys");
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(keys));
    crate::enrollment::enroll(
        &svc,
        &ClientEnroll {
            public_key: PHONE_KEY.into(),
            label: label.into(),
            client_id: "phone-1".into(),
            scope: Scope::Control as i32,
            shell_access: false,
            node_key: String::new(),
        },
    )
    .await
    .expect("the phone enrolls");
    let host = Uuid::now_v7();
    let root = svc.store.create_repository_root(host, "/repos/asks", 1_000).unwrap();
    let repo = svc.store.create_repository(host, root.id, "repo", "/repos/asks/.git", "").unwrap();
    let wt = svc.store.create_worktree(repo.id, "main", "/repos/asks", true).unwrap();
    let term = svc.store.create_terminal(wt.id, "pane", "claude", TerminalIntent::Running, 80, 24).unwrap();
    Runner { _dir: dir, svc, terminal: term.id }
}

fn handler(svc: &Arc<Service>, client_id: Option<&str>) -> RpcFactory {
    RpcFactory::new(
        svc.clone(),
        crate::watch::Watcher::new(svc.clone()),
        Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: client_id.map(str::to_string), scope: Scope::Control },
    )
}

fn an_answer(terminal: Uuid, request_id: &str, option_id: &str) -> Request {
    Request {
        method: "terminal.agent_answer".into(),
        payload: Some(request::Payload::AgentAnswer(AgentAnswer {
            terminal_id: crate::wire::id_bytes(terminal),
            request_id: request_id.into(),
            option_id: option_id.into(), answers: Default::default(), actor: String::new(),
        })),
        ..Default::default()
    }
}

/// The refusal's code and `what`, or `None` for a result.
async fn refusal(factory: &RpcFactory, req: Request) -> Option<(i32, String)> {
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(_)) => None,
        Some(response::Outcome::Error(e)) => Some((e.code, e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn code(error: DomainError) -> (i32, String) {
    (error.wire().0 as i32, error.what().to_string())
}

/// Hold an ask on the runner's pane the way `serve` does, and hand back
/// its id and what its hook is told.
fn held(r: &Runner) -> (String, tokio::task::JoinHandle<Option<Decision>>) {
    let (id, rx) = r.svc.hooks().asks().hold(r.terminal);
    let hook = tokio::spawn(async move {
        let settled = rx.await.ok()?;
        if let Some(ack) = settled.ack {
            let _ = ack.send(());
        }
        settled.decision
    });
    (id, hook)
}

#[tokio::test]
async fn an_answer_to_a_held_hook_ask_goes_to_the_hook_not_the_shim() {
    let r = a_runner("iPhone").await;
    let (id, hook) = held(&r);
    let phone = handler(&r.svc, Some("phone-1"));
    assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "allow")).await, None);
    assert_eq!(hook.await.unwrap(), Some(Decision::allow()));
}

#[tokio::test]
async fn a_deny_from_an_enrolled_device_is_named_by_its_label() {
    let r = a_runner("iPhone").await;
    let (id, hook) = held(&r);
    let phone = handler(&r.svc, Some("phone-1"));
    assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "deny")).await, None);
    assert_eq!(hook.await.unwrap(), Some(Decision::Deny { message: "Denied from iPhone".into() }));
}

#[tokio::test]
async fn a_deny_from_the_local_socket_is_named_for_the_mac() {
    let r = a_runner("iPhone").await;
    let (id, hook) = held(&r);
    let mac = handler(&r.svc, None);
    assert_eq!(refusal(&mac, an_answer(r.terminal, &id, "deny")).await, None);
    let expected = format!("Denied from {}", local_name(cfg!(target_os = "macos")));
    assert_eq!(hook.await.unwrap(), Some(Decision::Deny { message: expected }));
}

#[tokio::test]
async fn a_second_answer_is_refused_as_not_held() {
    let r = a_runner("iPhone").await;
    let (id, hook) = held(&r);
    let phone = handler(&r.svc, Some("phone-1"));
    assert_eq!(refusal(&phone, an_answer(r.terminal, &id, "allow")).await, None);
    hook.await.unwrap();
    let refused = refusal(&phone, an_answer(r.terminal, &id, "deny")).await;
    assert_eq!(
        refused,
        Some((farcooler_protocol::v1::ErrorCode::ResourceConflict as i32, "not_held".to_string())),
        "the first answer won; this one is told someone already answered"
    );
}

/// The ask was settled, but its hook went away before it could be told:
/// the answer never reached claude, which is a different sentence.
#[tokio::test]
async fn an_undelivered_answer_is_refused_as_not_delivered() {
    let r = a_runner("iPhone").await;
    let (id, rx) = r.svc.hooks().asks().hold(r.terminal);
    // A hook that hears its ending and hangs up without acknowledging it.
    let hook = tokio::spawn(async move { drop(rx.await) });
    let phone = handler(&r.svc, Some("phone-1"));
    let refused = refusal(&phone, an_answer(r.terminal, &id, "allow")).await;
    hook.await.unwrap();
    assert_eq!(
        refused,
        Some((farcooler_protocol::v1::ErrorCode::ResourceConflict as i32, "not_delivered".to_string()))
    );
}

#[tokio::test]
async fn an_option_the_ask_never_offered_is_refused_by_name() {
    let r = a_runner("iPhone").await;
    let (id, _hook) = held(&r);
    let phone = handler(&r.svc, Some("phone-1"));
    assert_eq!(
        refusal(&phone, an_answer(r.terminal, &id, "allow_always")).await,
        Some(code(DomainError::InvalidArgument { what: "option_id" }))
    );
    assert!(r.svc.hooks().asks().is_holding(r.terminal), "the ask stays held");
}

/// An answer for an ask no hook holds is a conflict, by name: chat mode's
/// asks, answered through a shim, went with it (ov-377).
#[tokio::test]
async fn an_answer_no_hook_holds_is_refused_as_not_held() {
    let r = a_runner("iPhone").await;
    let phone = handler(&r.svc, Some("phone-1"));
    assert_eq!(
        refusal(&phone, an_answer(r.terminal, "chat-1", "allow")).await,
        Some(code(DomainError::Conflict { what: "not_held" }))
    );
}

#[test]
fn a_device_label_is_cut_to_one_short_line() {
    assert_eq!(decider_name(false, Some("iPhone\nrm -rf /")), "iPhonerm -rf /");
    assert_eq!(decider_name(false, Some(&"x".repeat(200))).chars().count(), 40);
    assert_eq!(decider_name(false, Some("  iPad  ")), "iPad");
    assert_eq!(decider_name(false, Some(" \t ")), "a paired device");
    assert_eq!(decider_name(false, None), "a paired device");
    assert_eq!(decider_name(true, None), local_name(cfg!(target_os = "macos")));
}

/// "Denied from Mac" is wrong on a Linux runner.
#[test]
fn a_local_answer_is_named_for_the_computer_the_runner_is() {
    assert_eq!(local_name(true), "Mac");
    assert_eq!(local_name(false), "this computer");
}

#[path = "rpc_agent_answer_seat_tests.rs"]
mod seat_tests;

/// ov-740 review F3: a member answering from their phone ends `Done` for
/// them, not the owner's; the owner's answer ends the owner's.
#[tokio::test]
async fn a_members_answer_is_their_look_not_the_owners() {
    use farcooler_protocol::v1::AgentActivity;
    let r = a_runner("iPhone").await;
    r.svc.store.set_member("bob", "Bob", &["phone-1".to_string()]).unwrap();
    let watcher = crate::watch::Watcher::new(r.svc.clone());
    let on = |client: Option<&str>| {
        let peer = Peer { client_id: client.map(str::to_string), scope: Scope::Control };
        RpcFactory::new(r.svc.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer)
    };
    let done = crate::needs_you::Observation {
        activity: AgentActivity::Done,
        state_since: crate::review::now_millis() - 5_000,
        blocked_question: None,
        turn_failed: false,
        command: "claude".into(),
        trust_prompt: false,
        trust_folder: None,
        not_started: None,
    };
    watcher.observe_for_tests(r.terminal, done).await;
    let bob = crate::people::Person::Member("bob".into());

    let (id, hook) = held(&r);
    assert_eq!(refusal(&on(Some("phone-1")), an_answer(r.terminal, &id, "allow")).await, None);
    hook.await.unwrap();
    assert_eq!(watcher.activity(r.terminal).await.0, AgentActivity::Done, "the owner's Done stands");
    assert_eq!(watcher.activity_for(&bob, r.terminal).await.0, AgentActivity::Idle, "Bob's is over");

    let (id, hook) = held(&r);
    assert_eq!(refusal(&on(None), an_answer(r.terminal, &id, "allow")).await, None);
    hook.await.unwrap();
    assert_eq!(watcher.activity(r.terminal).await.0, AgentActivity::Idle, "the owner's answer ends the owner's");
}

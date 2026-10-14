//! A teammate added by QR (ov-525), as the runner sees it: their phone is
//! enrolled, bound to a member with their team directory account, sees itself
//! as that member, and loses its access when they're removed.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use crate::service::Service;

const BOBS_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone";

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

fn set(handle: &str, devices: &[&str], account: &str, email: &str) -> Request {
    Request {
        method: "member.set".into(),
        payload: Some(request::Payload::MemberSet(pb::MemberSet {
            handle: handle.into(),
            name: "Bob Smith".into(),
            client_ids: devices.iter().map(|d| d.to_string()).collect(),
            account: account.into(),
            email: email.into(),
        })),
        ..Default::default()
    }
}

/// A service on a scratch root whose `authorized_keys` this test can see,
/// with Bob's phone enrolled at Control, as the Mac's ceremony leaves it.
async fn with_bobs_phone_enrolled() -> (tempfile::TempDir, Arc<Service>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    let enroll = pb::ClientEnroll {
        public_key: BOBS_PHONE.into(),
        label: "Bob’s iPhone".into(),
        client_id: "phone-b".into(),
        scope: Scope::Control as i32,
        shell_access: false,
        node_key: String::new(),
    };
    crate::enrollment::enroll(&svc, &enroll).await.expect("the phone enrolls");
    (dir, svc)
}

/// Bound with his directory account, Bob's phone is Bob: `member.list` says
/// so to the phone itself, and shows his account and email to everyone.
#[tokio::test]
async fn a_teammates_phone_is_bound_with_their_account_and_sees_itself() {
    let (_dir, svc) = with_bobs_phone_enrolled().await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b"], "user_bob", "bob@example.com")).await.expect("bound");

    let phone = factory(&svc, Some("phone-b"), Scope::Control);
    let list = Request { method: "member.list".into(), ..Default::default() };
    let Ok(pb::result::Value::MemberList(seen)) = call(&phone, list).await else { panic!("a list") };
    assert_eq!(seen.you, "bob", "the phone sees itself as Bob");
    let bob = seen.members.iter().find(|m| m.handle == "bob").expect("Bob is listed");
    assert_eq!((bob.account.as_str(), bob.email.as_str()), ("user_bob", "bob@example.com"));
    assert_eq!(bob.client_ids, ["phone-b".to_string()]);

    // Only a runner admin binds anyone, the teammate included.
    assert_eq!(call(&phone, set("bob", &["phone-b"], "user_bob", "")).await.err().as_deref(), Some("scope-denied:"));
    // One account is one member: the same person can't be added twice.
    assert_eq!(call(&admin, set("bob2", &[], "user_bob", "")).await.err().as_deref(), Some("resource-conflict:account_taken"));
}

/// Removing a teammate still revokes their devices, so the account on the row
/// changes nothing about what `member.remove` does.
#[tokio::test]
async fn removing_a_teammate_revokes_their_phone() {
    let (_dir, svc) = with_bobs_phone_enrolled().await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b"], "user_bob", "bob@example.com")).await.expect("bound");
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    call(&admin, remove).await.expect("removed");
    let left = crate::enrollment::list(&svc).await.unwrap();
    assert!(left.items.iter().all(|e| e.client_id != "phone-b"), "Bob's phone is still enrolled: {left:?}");
    assert!(svc.store.member("bob").is_err());
    // And his account is free again, for the day he's added back.
    svc.store.set_member_in("bob", "Bob", &[], farcooler_store::people::Directory { account: "user_bob", email: "" }).unwrap();
}

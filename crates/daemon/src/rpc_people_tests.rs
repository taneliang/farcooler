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

    // The people log (ov-757): Bob added, then his devices changed, and the
    // phone taken off revoked (ov-783). Set again as he is, nothing more; a
    // refused call, nothing at all.
    call(&admin, set("bob", &[], "", "")).await.expect("his phone taken off");
    call(&admin, set("bob", &[], "", "")).await.expect("nothing changed");
    let log = svc.store.people_events(10).unwrap();
    let kinds: Vec<(&str, &str, &str)> = log.iter().map(|e| (e.kind.as_str(), e.handle.as_str(), e.actor.as_str())).collect();
    assert_eq!(kinds, [("revoked", "bob", "user"), ("devices", "bob", "user"), ("added", "bob", "user")]);
    assert_eq!(log[0].detail, r#"{"device":"phone-b"}"#);
    assert_eq!(log[1].detail, r#"{"from":["phone-b"],"to":[]}"#);
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
    // And the people log has both, newest first (ov-757).
    let kinds: Vec<(String, String)> = svc.store.people_events(10).unwrap().into_iter().map(|e| (e.kind, e.handle)).collect();
    let said = |k: &str| (k.to_string(), "bob".to_string());
    assert_eq!(kinds, [said("removed"), said("added")]);
    let left = crate::enrollment::list(&svc).await.unwrap();
    assert!(left.items.iter().all(|e| e.client_id != "phone-b"), "Bob's phone is still enrolled: {left:?}");
    assert!(svc.store.member("bob").is_err());
    // And his account is free again, for the day he's added back.
    svc.store.set_member_in("bob", "Bob", &[], farcooler_store::people::Directory { account: "user_bob", email: "" }).unwrap();
}

/// Enroll `key` as `client_id`, at Control, with no tunnel.
async fn enroll(svc: &Service, key: &str, client_id: &str) {
    let enroll = pb::ClientEnroll {
        public_key: key.into(),
        label: client_id.into(),
        client_id: client_id.into(),
        scope: Scope::Control as i32,
        shell_access: false,
        node_key: String::new(),
    };
    crate::enrollment::enroll(svc, &enroll).await.expect("enrolled");
}

/// A device `member.set` takes off a member's list loses its line in
/// `authorized_keys` and its open sessions close at once (ov-783), with each
/// revocation in the people log under who made the change. The device that
/// stays on the list, another member's, and the owner's own keep their lines
/// and their sessions.
#[tokio::test]
async fn a_device_taken_off_a_members_list_is_revoked_and_cut_off() {
    let (_dir, svc) = with_bobs_phone_enrolled().await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi pad", "pad-b").await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz alice", "phone-a").await;
    enroll(&svc, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIERERERERERERERERERERERERERERERERERERERERERE owner", "phone-o").await;
    let admin = factory(&svc, None, Scope::HostAdmin);
    call(&admin, set("bob", &["phone-b", "pad-b"], "", "")).await.expect("Bob bound");
    call(&admin, set("alice", &["phone-a"], "", "")).await.expect("Alice bound");
    let open = |d: &str| svc.sessions().open(Some(d.to_string()));
    let (pad, phone, alices, owners) = (open("pad-b"), open("phone-b"), open("phone-a"), open("phone-o"));

    call(&admin, set("bob", &["phone-b"], "", "")).await.expect("his pad taken off");

    let enrolled = |svc: &Service| std::fs::read_to_string(svc.authorized_keys()).unwrap();
    let keys = enrolled(&svc);
    assert!(!keys.contains("pad-b"), "the pad still has a line:\n{keys}");
    assert!(pad.is_closed(), "the pad's open session still runs");
    for (device, session) in [("phone-b", &phone), ("phone-a", &alices), ("phone-o", &owners)] {
        assert!(keys.contains(device), "{device} lost its line:\n{keys}");
        assert!(!session.is_closed(), "{device}'s session was closed");
    }
    let log = svc.store.people_events(1).unwrap();
    assert_eq!((log[0].kind.as_str(), log[0].handle.as_str(), log[0].actor.as_str()), ("revoked", "bob", "user"));
    assert_eq!(log[0].detail, r#"{"device":"pad-b"}"#);

    // A device taken off and bound again at once, before its revoke, is
    // someone's and keeps its line: what's revoked is what's still released.
    let (_, released) = svc.store.set_member_releasing("bob", "Bob", &[], Default::default()).unwrap();
    assert_eq!(released, ["phone-b".to_string()]);
    svc.store.set_member("alice", "Alice", &["phone-a".to_string(), "phone-b".to_string()]).unwrap();
    let admin_peer = Peer { client_id: None, scope: Scope::HostAdmin };
    assert!(super::revoke_released(&svc, &admin_peer, None, "bob", &released).await.is_none());
    assert!(enrolled(&svc).contains("phone-b") && !phone.is_closed(), "a device bound again was revoked");
}

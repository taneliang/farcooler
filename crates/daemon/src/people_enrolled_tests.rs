//! A device a member's device enrolls (ov-719, ov-716 review finding 1):
//! nobody's, so it only reads, until a runner admin binds it to a member;
//! never a shell; revoked with the member who enrolled it while still
//! unbound; and a member admin neither binds nor revokes the owner's. The
//! teammate enrollment a member admin's Mac runs (`TeammateEnrollment`)
//! still works end to end. Through `Rpc`, where a client reaches them.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Scope, response};

use super::*;
use crate::rpc::RpcFactory;
use crate::service::Service;

const KEY_BOBS_MAC: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICEhISEhISEhISEhISEhISEhISEhISEhISEhISEhISEh mac";
const KEY_TESS: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi phone";
const KEY_OWNERS_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMj phone";
const KEY_OTHER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk phone";
const KEY_OWNERS_SHELL: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUl mac";

/// Bob's Mac, a runner admin's device bound to Bob, a member.
const BOBS_MAC: Option<&str> = Some("mac-b");
/// The owner's phone, which the owner enrolled and nobody bound.
const OWNERS_PHONE: &str = "phone-o";

/// A runner on a scratch root whose `authorized_keys` the test can read,
/// with Bob's Mac enrolled at `host_admin` and bound to Bob, and the
/// owner's phone enrolled at Control, both by the owner.
async fn runner() -> (tempfile::TempDir, Arc<Service>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_BOBS_MAC, "mac-b", Scope::HostAdmin, false)).await.expect("Bob's Mac enrolls");
    call(&local, enroll(KEY_OWNERS_PHONE, OWNERS_PHONE, Scope::Control, false)).await.expect("the owner's phone enrolls");
    call(&local, set("bob", &["mac-b"])).await.expect("Bob's Mac is Bob's");
    (dir, svc)
}

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

fn bobs_mac(svc: &Arc<Service>) -> RpcFactory {
    factory(svc, BOBS_MAC, Scope::HostAdmin)
}

async fn call(factory: &RpcFactory, req: Request) -> std::result::Result<pb::result::Value, String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn enroll(key: &str, client_id: &str, scope: Scope, shell_access: bool) -> Request {
    Request {
        method: "client.enroll".into(),
        payload: Some(request::Payload::ClientEnroll(pb::ClientEnroll {
            public_key: key.into(),
            label: "A device".into(),
            client_id: client_id.into(),
            scope: scope as i32,
            shell_access,
            node_key: String::new(),
        })),
        ..Default::default()
    }
}

fn set(handle: &str, devices: &[&str]) -> Request {
    Request {
        method: "member.set".into(),
        payload: Some(request::Payload::MemberSet(pb::MemberSet {
            handle: handle.into(),
            name: "Someone".into(),
            client_ids: devices.iter().map(|d| d.to_string()).collect(),
            account: String::new(),
            email: String::new(),
        })),
        ..Default::default()
    }
}

fn revoke(client_id: &str) -> Request {
    Request {
        method: "client.revoke".into(),
        payload: Some(request::Payload::ClientRevoke(pb::ClientRevoke { client_id: client_id.into() })),
        ..Default::default()
    }
}

fn remove(handle: &str) -> Request {
    Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    }
}

/// A write any member may make, which a released device may not: a card.
fn a_card() -> Request {
    Request { method: "task.create".into(), ..Default::default() }
}

fn who(svc: &Service, device: &str) -> Person {
    person_of(&svc.store, &Peer { client_id: Some(device.into()), scope: Scope::Control }).unwrap()
}

async fn enrolled(svc: &Service) -> Vec<(String, bool)> {
    crate::enrollment::list(svc).await.unwrap().items.into_iter().map(|e| (e.client_id, e.shell_access)).collect()
}

/// Before ov-719, a device Bob's Mac enrolled was bound to nobody, so it
/// was the owner's: the plan, the daemon, the owner's posts. Now it's
/// nobody's, and reads, until a runner admin binds it; then it's its
/// member's. A device the owner enrolls, from the local socket or their
/// own phone, is still the owner's.
#[tokio::test]
async fn a_device_a_member_enrolls_only_reads_until_it_is_bound() {
    let (_dir, svc) = runner().await;
    let bob = bobs_mac(&svc);
    let Ok(pb::result::Value::ClientEnroll(done)) = call(&bob, enroll(KEY_TESS, "phone-t", Scope::Control, false)).await
    else {
        panic!("Bob's Mac enrolls a device")
    };
    assert!(!done.already_enrolled);
    assert_eq!(who(&svc, "phone-t"), Person::Released, "the owner's, before ov-719");
    let tess = factory(&svc, Some("phone-t"), Scope::Control);
    assert_eq!(call(&tess, a_card()).await.err().as_deref(), Some("not-yours:not_yours"), "it writes nothing");
    let read = Request { method: "member.list".into(), ..Default::default() };
    assert!(call(&tess, read.clone()).await.is_ok(), "it reads");
    let shutdown = Request { method: "daemon.shutdown".into(), ..Default::default() };
    assert!(gate(&svc, &Peer { client_id: Some("phone-t".into()), scope: Scope::HostAdmin }, None, &shutdown).is_err());

    // Bound, it's Tess's.
    call(&bob, set("tess", &["phone-t"])).await.expect("Bob binds it");
    assert_eq!(who(&svc, "phone-t"), Person::Member("tess".into()));
    assert!(gate(&svc, &Peer { client_id: Some("phone-t".into()), scope: Scope::Control }, None, &a_card()).is_ok());

    // The owner's enrollments are the owner's, from the socket or their phone.
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_OTHER, "phone-x", Scope::Control, false)).await.expect("the owner enrolls");
    assert_eq!(who(&svc, "phone-x"), Person::Owner);
    call(&local, revoke("phone-x")).await.expect("revoked");
    let phone = factory(&svc, Some(OWNERS_PHONE), Scope::HostAdmin);
    call(&phone, enroll(KEY_OTHER, "phone-y", Scope::Control, false)).await.expect("the owner's phone enrolls");
    assert_eq!(who(&svc, "phone-y"), Person::Owner);
    assert_eq!(who(&svc, OWNERS_PHONE), Person::Owner);
}

/// A plain line is a shell on the runner's account with no forced command,
/// so no gate, and it outlives the member: a member's device enrolls none.
/// The owner's do, as a Mac's Key B.
#[tokio::test]
async fn a_member_enrolls_no_shell() {
    let (_dir, svc) = runner().await;
    let bob = bobs_mac(&svc);
    assert_eq!(
        call(&bob, enroll(KEY_TESS, "mac-t", Scope::Control, true)).await.err().as_deref(),
        Some("not-yours:not_yours")
    );
    assert!(enrolled(&svc).await.iter().all(|(id, _)| id != "mac-t"), "nothing was written");
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_OWNERS_SHELL, "mac-o", Scope::HostAdmin, true)).await.expect("the owner's Key B");
    let phone = factory(&svc, Some(OWNERS_PHONE), Scope::HostAdmin);
    call(&phone, enroll(KEY_TESS, "mac-p", Scope::HostAdmin, true)).await.expect("from the owner's phone too");
}

/// Removing Bob revokes the devices he enrolled that nobody was bound to,
/// as it revokes his own; one he bound to Tess is hers and stays.
#[tokio::test]
async fn removing_a_member_revokes_the_unbound_devices_they_enrolled() {
    let (_dir, svc) = runner().await;
    let bob = bobs_mac(&svc);
    call(&bob, enroll(KEY_TESS, "phone-t", Scope::Control, false)).await.expect("Tess's phone");
    call(&bob, set("tess", &["phone-t"])).await.expect("bound to Tess");
    call(&bob, enroll(KEY_OTHER, "phone-u", Scope::Control, false)).await.expect("one left unbound");

    call(&factory(&svc, None, Scope::HostAdmin), remove("bob")).await.expect("Bob removed");
    let left: Vec<String> = enrolled(&svc).await.into_iter().map(|(id, _)| id).collect();
    assert!(!left.contains(&"phone-u".to_string()), "Bob's unbound device outlived him: {left:?}");
    assert!(!left.contains(&"mac-b".to_string()), "and his own Mac: {left:?}");
    assert!(left.contains(&"phone-t".to_string()), "Tess's phone is hers: {left:?}");
    assert!(left.contains(&OWNERS_PHONE.to_string()), "and the owner's is theirs: {left:?}");
    assert_eq!(who(&svc, "phone-u"), Person::Released, "and it's nobody's, if it comes back");
    assert_eq!(who(&svc, "phone-t"), Person::Member("tess".into()));
}

/// The owner's phone is bound to nobody and was enrolled by no member. A
/// member admin can't bind it to someone (it would stop being the owner's)
/// or revoke it (the owner locked out of their own runner). The owner can.
/// A member's device also can't put a line under a device id that has one
/// in another shape: marked nobody's, it would take the owner's with it.
#[tokio::test]
async fn a_member_admin_neither_binds_nor_revokes_the_owners_devices() {
    let (_dir, svc) = runner().await;
    let bob = bobs_mac(&svc);
    assert_eq!(call(&bob, set("tess", &[OWNERS_PHONE])).await.err().as_deref(), Some("not-yours:not_yours"));
    assert_eq!(call(&bob, set("bob", &["mac-b", OWNERS_PHONE])).await.err().as_deref(), Some("not-yours:not_yours"));
    assert_eq!(call(&bob, revoke(OWNERS_PHONE)).await.err().as_deref(), Some("not-yours:not_yours"));
    assert_eq!(who(&svc, OWNERS_PHONE), Person::Owner);
    assert!(enrolled(&svc).await.iter().any(|(id, _)| id == OWNERS_PHONE), "still enrolled");

    // Under the owner's Mac's id, whose only line is its Key B.
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_OWNERS_SHELL, "mac-o", Scope::HostAdmin, true)).await.expect("the owner's Key B");
    assert_eq!(
        call(&bob, enroll(KEY_OTHER, "mac-o", Scope::Control, false)).await.err().as_deref(),
        Some("resource-conflict:device_taken")
    );
    assert_eq!(who(&svc, "mac-o"), Person::Owner);
    assert_eq!(enrolled(&svc).await.iter().filter(|(id, _)| id == "mac-o").count(), 1);

    // What isn't the owner's is still a runner admin's: Bob revokes a device
    // he enrolled, and Bob's own.
    call(&bob, enroll(KEY_TESS, "phone-t", Scope::Control, false)).await.expect("enrolled");
    call(&bob, revoke("phone-t")).await.expect("Bob takes his own enrollment off");

    // The owner's own say over their devices, from the socket and the phone.
    let phone = factory(&svc, Some(OWNERS_PHONE), Scope::HostAdmin);
    call(&phone, revoke("mac-o")).await.expect("the owner's phone revokes the owner's Mac");
    call(&local, set("tess", &[OWNERS_PHONE])).await.expect("the owner binds their own phone");
    call(&local, revoke(OWNERS_PHONE)).await.expect("and revokes it");
}

/// What `TeammateEnrollment.enrollOne` runs from Bob's Mac, call for call:
/// who's there, Key A at the chosen scope, the bind. The teammate's phone
/// is then Tess's, with a member's say. A second run finds it already
/// there, takes it off and writes it again, and binds it again; and a bind
/// that failed takes it off (`takeOff`).
#[tokio::test]
async fn teammate_enrollment_from_a_member_admins_mac_works_end_to_end() {
    let (_dir, svc) = runner().await;
    let bob = bobs_mac(&svc);
    let list = Request { method: "member.list".into(), ..Default::default() };
    for run in 0..2 {
        call(&bob, list.clone()).await.expect("who's there");
        let Ok(pb::result::Value::ClientEnroll(done)) = call(&bob, enroll(KEY_TESS, "phone-t", Scope::Control, false)).await
        else {
            panic!("Key A, run {run}")
        };
        if done.already_enrolled {
            call(&bob, revoke("phone-t")).await.expect("an earlier key is taken off");
            call(&bob, enroll(KEY_TESS, "phone-t", Scope::Control, false)).await.expect("and written again");
            // Written again by a member, it's nobody's until bound again: a
            // revoked id still bound to Tess would make Bob's key Tess.
            assert_eq!(who(&svc, "phone-t"), Person::Released, "the new line is Tess's before any bind");
            assert!(svc.store.member("tess").unwrap().devices.is_empty(), "and Tess's list no longer has it");
        }
        assert_eq!(done.already_enrolled, run == 1, "run {run}");
        call(&bob, set("tess", &["phone-t"])).await.expect("bound to Tess");
        assert_eq!(who(&svc, "phone-t"), Person::Member("tess".into()), "run {run}");
    }
    let tess = factory(&svc, Some("phone-t"), Scope::Control);
    let Ok(pb::result::Value::MemberList(seen)) = call(&tess, list).await else { panic!("a list") };
    assert_eq!(seen.you, "tess");
    assert!(gate(&svc, &Peer { client_id: Some("phone-t".into()), scope: Scope::Control }, None, &a_card()).is_ok());

    // A bind that failed: the key is taken off again.
    call(&bob, enroll(KEY_OTHER, "phone-v", Scope::Control, false)).await.expect("written");
    assert!(call(&bob, set("Not A Handle", &["phone-v"])).await.is_err(), "the bind fails");
    call(&bob, revoke("phone-v")).await.expect("taken off");
}

/// `-` is the session preamble's word for no device (`listener::read_opening`),
/// so a line with `--client -` is a session with no client id: the owner,
/// whatever enrolled it, and past every check the gate keys on a device. No
/// line is written under it, from a member's device or the owner's.
#[tokio::test]
async fn nobody_enrolls_a_device_named_as_no_device() {
    let (_dir, svc) = runner().await;
    for caller in [bobs_mac(&svc), factory(&svc, None, Scope::HostAdmin)] {
        let refused = call(&caller, enroll(KEY_TESS, "-", Scope::HostAdmin, false)).await;
        assert!(refused.as_ref().is_err_and(|e| e.starts_with("invalid-argument")), "{refused:?}");
    }
    assert!(enrolled(&svc).await.iter().all(|(id, _)| id != "-"), "nothing was written");
}

/// A device Ned's last device is enrolling when he's removed (he has no other, so
/// nothing else in his removal waits on the lock) (ov-720, ov-719
/// re-review R1): its store row is committed, inside the fence lock, and its
/// line isn't written yet. A removal that asked "has a line?" first, with no
/// lock, skipped it, and the line landed after: a released device with a
/// line. Staged with the enroll held inside its lock hold, after the row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_member_revokes_a_device_whose_line_lands_late() {
    use std::sync::mpsc;
    let (_dir, svc) = runner().await;
    call(&factory(&svc, None, Scope::HostAdmin), set("ned", &[])).await.expect("Ned, with no devices");
    let (recorded_tx, recorded_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let late = {
        let svc = svc.clone();
        tokio::spawn(async move {
            let store = svc.clone();
            let by_bob: crate::enrollment::MemberEnrolled = Box::new(move |id: &str| {
                store.store.record_member_enrolled(id, "ned")?;
                recorded_tx.send(()).unwrap();
                go_rx.recv().unwrap();
                Ok(())
            });
            let request = pb::ClientEnroll {
                public_key: KEY_TESS.into(),
                label: "A device".into(),
                client_id: "phone-late".into(),
                scope: Scope::Control as i32,
                shell_access: false,
                node_key: String::new(),
            };
            crate::enrollment::enroll_as(&svc, &request, Some(by_bob)).await
        })
    };
    tokio::task::spawn_blocking(move || recorded_rx.recv().unwrap()).await.unwrap();
    assert!(enrolled(&svc).await.iter().all(|(id, _)| id != "phone-late"), "the row is in; the line isn't");

    let removal = {
        let svc = svc.clone();
        tokio::spawn(async move { call(&factory(&svc, None, Scope::HostAdmin), remove("ned")).await })
    };
    // Long enough for a removal that reads the lines without the lock to have
    // read them; the revoke waits on the lock the enroll holds.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    go_tx.send(()).unwrap();
    late.await.unwrap().expect("the late enroll finished");
    removal.await.unwrap().expect("Ned removed");

    let left: Vec<String> = enrolled(&svc).await.into_iter().map(|(id, _)| id).collect();
    assert!(!left.contains(&"phone-late".to_string()), "a late line outlived Ned: {left:?}");
}

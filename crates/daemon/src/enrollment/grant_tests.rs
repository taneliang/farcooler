//! The owner's phone adds a browser (ov-828): through `Rpc`, where the phone
//! reaches it, from a phone enrolled at `control` as the Mac's ceremony
//! leaves every phone.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, response};
use farcooler_transport::Peer;

use crate::rpc::RpcFactory;
use crate::service::Service;

const KEY_OWNERS_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMj phone";
/// A teammate's phone the owner's Mac adds (S1).
const KEY_TESS: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi phone";
const KEY_BOBS_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER phone";
/// The browser's key. Ed25519 rather than its passkey: whether this runner's
/// sshd takes passkeys is process-wide (`sshd_version`'s test owns it), and
/// the scope a phone grants at doesn't depend on the algorithm.
const KEY_BROWSER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser";
/// The browser's Tailcat node key, as its offer carries it.
const NODE_BROWSER: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// The log of each enrollment is `tests/an_enrollment_is_logged.rs`: a log
/// capture races every other test reaching the same call site on another
/// thread, so it runs alone in its own binary.
async fn runner() -> (tempfile::TempDir, Arc<Service>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_OWNERS_PHONE, "phone-o", Scope::Control, false, "")).await.expect("the owner's phone");
    call(&local, enroll(KEY_BOBS_PHONE, "phone-b", Scope::Control, false, "")).await.expect("Bob's phone");
    let bind = pb::MemberSet { handle: "bob".into(), name: "Bob".into(), client_ids: vec!["phone-b".into()], ..Default::default() };
    let bind = Request { method: "member.set".into(), payload: Some(request::Payload::MemberSet(bind)), ..Default::default() };
    call(&local, bind).await.expect("Bob's phone is Bob's");
    (dir, svc)
}

fn factory(svc: &Arc<Service>, device: Option<&str>, scope: Scope) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope };
    RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(factory: &RpcFactory, req: Request) -> std::result::Result<pb::result::Value, String> {
    use farcooler_transport::Handler as _;
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => Ok(r.value.expect("a value")),
        Some(response::Outcome::Error(e)) => Err(format!("{}:{}", farcooler_core::error::word_for(e.code), e.what)),
        other => panic!("no outcome: {other:?}"),
    }
}

fn enroll(key: &str, client_id: &str, scope: Scope, shell_access: bool, node_key: &str) -> Request {
    let p = pb::ClientEnroll {
        public_key: key.into(),
        label: "Chrome on Mac".into(),
        client_id: client_id.into(),
        scope: scope as i32,
        shell_access,
        node_key: node_key.into(),
        for_teammate: false,
        step_up_key: None,
        teammate: None,
    };
    Request { method: "client.enroll".into(), payload: Some(request::Payload::ClientEnroll(p)), ..Default::default() }
}

async fn enrolled(svc: &Service) -> Vec<String> {
    crate::enrollment::list(svc).await.unwrap().items.into_iter().map(|e| e.client_id).collect()
}

/// The owner's report: their phone, at `control` on Cosmo, couldn't add the
/// browser. It was refused `scope_denied` before anything ran, and the reply
/// said pending. Now the browser's line is written, with its node key, so the
/// runner's tunnel admits it.
#[tokio::test]
async fn the_owners_control_phone_adds_a_browser() {
    let (_dir, svc) = runner().await;
    let phone = factory(&svc, Some("phone-o"), Scope::Control);
    let Ok(pb::result::Value::ClientEnroll(done)) =
        call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::Control, false, NODE_BROWSER)).await
    else {
        panic!("the owner's phone enrolls the browser")
    };
    assert!(!done.already_enrolled);
    assert!(enrolled(&svc).await.contains(&"browser-1".to_string()));
    let line = crate::enrollment::list(&svc).await.unwrap().items.into_iter().find(|e| e.client_id == "browser-1").unwrap();
    assert_eq!(line.scope, Scope::Control as i32);
    assert!(!line.shell_access);
}

/// What a `control` device may not grant: anything wider than it holds.
#[tokio::test]
async fn a_control_phone_grants_nothing_wider_than_its_own() {
    let (_dir, svc) = runner().await;
    let phone = factory(&svc, Some("phone-o"), Scope::Control);
    let admin = call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::HostAdmin, false, "")).await;
    assert_eq!(admin.err().as_deref(), Some("scope-denied:"), "host_admin from a control phone");
    let shell = call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::HostAdmin, true, "")).await;
    assert_eq!(shell.err().as_deref(), Some("scope-denied:"), "a shell line from a control phone");
    // Refused at the grant, not left to the fence's own shape check.
    let shell = call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::Control, true, "")).await;
    assert_eq!(shell.err().as_deref(), Some("scope-denied:"), "a shell line asked for at control");
    let read = factory(&svc, Some("phone-o"), Scope::Read);
    let looked = call(&read, enroll(KEY_BROWSER, "browser-1", Scope::Read, false, "")).await;
    assert_eq!(looked.err().as_deref(), Some("scope-denied:"), "a read phone enrolls nothing");
    assert!(!enrolled(&svc).await.contains(&"browser-1".to_string()), "nothing was written");
    // A read grant from a control phone is within its own.
    call(&phone, enroll(KEY_BROWSER, "browser-1", Scope::Read, false, "")).await.expect("read is narrower");
}

/// ov-838 (R-88): a teammate's phone adds their own browser, at their
/// scope. The line is theirs from its write: bound to them, never the
/// owner's, never released, and removing them takes it. Nothing wider than
/// the phone holds, no shell, and never `for_teammate`, the owner's Add
/// Teammate.
#[tokio::test]
async fn a_members_control_phone_adds_their_own_browser() {
    let (_dir, svc) = runner().await;
    let bobs = factory(&svc, Some("phone-b"), Scope::Control);
    let admin = call(&bobs, enroll(KEY_BROWSER, "browser-b", Scope::HostAdmin, false, "")).await;
    assert_eq!(admin.err().as_deref(), Some("scope-denied:"), "host_admin from a member's control phone");
    let shell = call(&bobs, enroll(KEY_BROWSER, "browser-b", Scope::HostAdmin, true, "")).await;
    assert!(shell.is_err(), "a shell line from a member's phone");
    let mut teammate = enroll(KEY_BROWSER, "browser-b", Scope::Control, false, "");
    let Some(request::Payload::ClientEnroll(p)) = teammate.payload.as_mut() else { unreachable!() };
    p.for_teammate = true;
    let teammate = call(&bobs, teammate).await;
    assert_eq!(teammate.err().as_deref(), Some("not-yours:not_owners_device"), "a member adding for a teammate");
    assert!(!enrolled(&svc).await.contains(&"browser-b".to_string()), "a refused grant wrote");

    call(&bobs, enroll(KEY_BROWSER, "browser-b", Scope::Control, false, NODE_BROWSER)).await.expect("Bob's phone adds Bob's browser");
    let browser = Peer { client_id: Some("browser-b".into()), scope: Scope::Control };
    assert_eq!(crate::people::person_of(&svc.store, &browser).unwrap(), crate::people::Person::Member("bob".into()), "the browser isn't Bob's");
    let line = crate::enrollment::list(&svc).await.unwrap().items.into_iter().find(|e| e.client_id == "browser-b").unwrap();
    assert_eq!(line.scope, Scope::Control as i32);
    assert_eq!(line.added_by, "phone-b", "the browser's enroller wasn't kept");

    svc.store.remove_member("bob").unwrap();
    crate::release_sweep::sweep(&svc).await.unwrap();
    assert!(!enrolled(&svc).await.contains(&"browser-b".to_string()), "Bob's browser outlived his removal");
}

/// The review's S1 (ov-828): the Mac's Add Teammate writes the teammate's
/// key, then binds it in a second call. Between the two, a device bound to
/// nobody read as the owner's, and could grant devices that stayed the
/// owner's after the teammate was removed. Written `for_teammate`, it is
/// nobody's from the write: refused, and left for the bind by the sweep.
#[tokio::test]
async fn a_teammates_device_waiting_on_its_bind_grants_nothing() {
    let (_dir, svc) = runner().await;
    let local = factory(&svc, None, Scope::HostAdmin);
    let mut teammate = enroll(KEY_TESS, "phone-t", Scope::Control, false, "");
    let Some(request::Payload::ClientEnroll(p)) = teammate.payload.as_mut() else { unreachable!() };
    p.for_teammate = true;
    call(&local, teammate).await.expect("the owner's Mac writes the teammate's key");
    assert!(enrolled(&svc).await.contains(&"phone-t".to_string()));

    let tess = factory(&svc, Some("phone-t"), Scope::Control);
    let refused = call(&tess, enroll(KEY_BROWSER, "browser-t", Scope::Control, false, NODE_BROWSER)).await;
    assert!(refused.as_ref().err().is_some_and(|e| e.starts_with("not-yours:")), "{refused:?}");
    assert!(!enrolled(&svc).await.contains(&"browser-t".to_string()), "an unbound teammate granted a device");
    assert!(!svc.store.released_to_sweep().unwrap().contains(&"phone-t".to_string()), "the sweep would take it before the bind");

    // Bound, it is Tess's, and adds her own browser, hers (ov-838).
    let bind = pb::MemberSet { handle: "tess".into(), name: "Tess".into(), client_ids: vec!["phone-t".into()], ..Default::default() };
    let bind = Request { method: "member.set".into(), payload: Some(request::Payload::MemberSet(bind)), ..Default::default() };
    call(&local, bind).await.expect("bound");
    call(&tess, enroll(KEY_BROWSER, "browser-t", Scope::Control, false, NODE_BROWSER)).await.expect("Tess's phone adds her browser");
    assert_eq!(svc.store.member_of_device("browser-t").unwrap().as_deref(), Some("tess"));
}

/// The re-review's R1 (ov-828): a member's own enroll is recorded as theirs
/// even when it says `for_teammate`, so removing the member revokes what it
/// added. Taken as a pending teammate instead, the device would be nobody's
/// with no enroller, and the sweep, which spares pending devices, would leave
/// its line after the member was gone.
#[tokio::test]
async fn a_members_enroll_is_theirs_even_when_it_says_for_teammate() {
    let (_dir, svc) = runner().await;
    let local = factory(&svc, None, Scope::HostAdmin);
    call(&local, enroll(KEY_TESS, "mac-b", Scope::HostAdmin, false, "")).await.expect("Bob's Mac");
    let bind = pb::MemberSet { handle: "bob".into(), name: "Bob".into(), client_ids: vec!["phone-b".into(), "mac-b".into()], ..Default::default() };
    let bind = Request { method: "member.set".into(), payload: Some(request::Payload::MemberSet(bind)), ..Default::default() };
    call(&local, bind).await.expect("Bob's Mac is Bob's");

    let bobs_mac = factory(&svc, Some("mac-b"), Scope::HostAdmin);
    let mut teammate = enroll(KEY_BROWSER, "browser-m", Scope::Control, false, "");
    let Some(request::Payload::ClientEnroll(p)) = teammate.payload.as_mut() else { unreachable!() };
    p.for_teammate = true;
    call(&bobs_mac, teammate).await.expect("a member admin's Mac adds a device");

    let enrolled_by_bob = svc.store.remove_member_with_enrolled("bob").unwrap();
    assert_eq!(enrolled_by_bob, vec!["browser-m".to_string()], "Bob's enroll wasn't recorded as his");
}

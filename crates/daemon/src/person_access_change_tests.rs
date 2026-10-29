//! A phone's Access menu and Limit to Viewer (ov-856): the change is the
//! owner's or an Admin's, signed for exactly that request, and decided again
//! under the lock.

use farcooler_protocol::v1::{self as pb, Request, Scope, request};
use farcooler_store::access::{Access, Cap};

use super::*;
use crate::rpc_people::tests::call;
use crate::step_up::tests::{Device, Runner, nonce, runner, word};

fn set_access_req(handle: &str, access: &str) -> Request {
    Request {
        method: "member.set_access".into(),
        payload: Some(request::Payload::MemberSetAccess(pb::MemberSetAccess { handle: handle.into(), access: access.into() })),
        ..Default::default()
    }
}

fn set_cap_req(device: &str, cap: &str) -> Request {
    Request {
        method: "client.set_cap".into(),
        payload: Some(request::Payload::ClientSetCap(pb::ClientSetCap { client_id: device.into(), cap: cap.into() })),
        ..Default::default()
    }
}

/// `device` signs `req` on a fresh connection, and sends it.
async fn send(r: &Runner, device: &Device, req: Request) -> std::result::Result<pb::result::Value, String> {
    let conn = r.conn(device.id);
    let n = nonce(&conn).await;
    call(&conn, device.step(&r.pin, req, &n)).await
}

fn trusted(r: &Runner, id: &'static str, n: u8) -> Device {
    let device = Device::new(id, n);
    r.trust(&device, crate::step_up::ALG_DEVICE);
    device
}

/// The owner's phone, signed for it, changes a person's access and limits a
/// device. Dan is an Admin; Carol a Viewer; Bob a Member.
#[tokio::test]
async fn the_owners_phone_changes_access_and_limits_a_device_with_a_signature() {
    let r = runner().await;
    let phone = trusted(&r, "phone-o", 11);
    send(&r, &phone, set_access_req("bob", "viewer")).await.expect("Access");
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Access::Viewer));
    send(&r, &phone, set_access_req("carol", "admin")).await.expect("the owner gives Admin");
    assert_eq!(r.svc.store.member_access("carol").unwrap(), Some(Access::Admin));
    send(&r, &phone, set_cap_req("phone-d", "viewer")).await.expect("Limit to Viewer");
    assert_eq!(r.svc.store.device_cap("phone-d").unwrap(), Some(Cap::Viewer));
}

/// Without the signature the same call is refused, whatever the device holds:
/// a trusted key must sign, and a line that reads `control` is not a step-up.
#[tokio::test]
async fn a_device_that_did_not_step_up_changes_nothing() {
    let r = runner().await;
    let _ = trusted(&r, "phone-o", 11);
    for req in [set_access_req("bob", "viewer"), set_cap_req("phone-b", "viewer")] {
        let keyed = word(call(&r.conn("phone-o"), req.clone()).await);
        assert!(keyed.ends_with("step_up_needed"), "{keyed}");
        // No key at all: today's rule, which is the socket's alone.
        let keyless = word(call(&r.conn("ipad-o"), req).await);
        assert!(keyless.starts_with("scope-denied"), "{keyless}");
    }
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Access::Member));
    assert_eq!(r.svc.store.device_cap("phone-b").unwrap(), None);
}

/// A signature for one change is not one for another.
#[tokio::test]
async fn a_signature_is_for_that_person_and_that_access() {
    let r = runner().await;
    let phone = trusted(&r, "phone-o", 11);
    let conn = r.conn("phone-o");
    let n = nonce(&conn).await;
    let mut swapped = phone.step(&r.pin, set_access_req("carol", "member"), &n);
    swapped.payload = set_access_req("carol", "admin").payload;
    assert!(word(call(&conn, swapped).await).ends_with("step_up_failed"));
    let n = nonce(&conn).await;
    let mut other = phone.step(&r.pin, set_cap_req("phone-b", "viewer"), &n);
    other.payload = set_cap_req("phone-o", "viewer").payload;
    assert!(word(call(&conn, other).await).ends_with("step_up_failed"));
    assert_eq!(r.svc.store.member_access("carol").unwrap(), Some(Access::Viewer));
    assert_eq!(r.svc.store.device_cap("phone-o").unwrap(), Some(Cap::Member), "the owner phone's cap from its control line, unchanged");
}

/// An Admin works within their reach: Members and Viewers, never Admin, never
/// another Admin or the owner's devices, and themselves only downward.
#[tokio::test]
async fn an_admin_manages_members_and_viewers_and_lowers_only_themselves() {
    let r = runner().await;
    crate::person_access::set_access(&r.svc, "bob", Access::Admin, || ()).await.unwrap();
    let dan = trusted(&r, "phone-d", 12);

    send(&r, &dan, set_access_req("carol", "member")).await.expect("a Viewer to Member");
    send(&r, &dan, set_cap_req("tv-c", "viewer")).await.expect("a Member's device held at Viewer");
    assert_eq!(r.svc.store.member_access("carol").unwrap(), Some(Access::Member));

    for (what, req) in [
        ("giving Admin", set_access_req("carol", "admin")),
        ("another Admin", set_access_req("bob", "viewer")),
        ("another Admin's device", set_cap_req("phone-b", "viewer")),
        ("the owner's device", set_cap_req("phone-o", "viewer")),
        ("raising themselves", set_access_req("dan", "admin")),
        ("freeing their own device", set_cap_req("pad-d", "")),
    ] {
        let refused = word(send(&r, &dan, req).await);
        assert!(refused.ends_with("not_yours"), "{what}: {refused}");
    }
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Access::Admin));
    assert_eq!(r.svc.store.member_access("dan").unwrap(), Some(Access::Admin));

    send(&r, &dan, set_cap_req("pad-d", "viewer")).await.expect("their own device lowered");
    let raised = word(send(&r, &dan, set_cap_req("pad-d", "member")).await);
    assert!(raised.ends_with("not_yours"), "raising their own device: {raised}");
    send(&r, &dan, set_access_req("dan", "member")).await.expect("themselves lowered");
    assert_eq!(r.svc.store.member_access("dan").unwrap(), Some(Access::Member));
}

/// A Member signs nothing that changes access, and a demotion that lands
/// after the request was decided refuses it under the lock.
#[tokio::test]
async fn a_member_changes_nothing_and_a_demotion_in_between_refuses() {
    let r = runner().await;
    let bob = trusted(&r, "phone-b", 13);
    let refused = word(send(&r, &bob, set_access_req("carol", "member")).await);
    assert!(refused.contains("not_admin") || refused.contains("not_yours"), "{refused}");

    // Decided for Dan as an Admin; demoted before the write.
    let dan = trusted(&r, "phone-d", 12);
    let conn = r.conn("phone-d");
    let n = nonce(&conn).await;
    let _ = (&dan, &n);
    let peer = farcooler_transport::Peer { client_id: Some("phone-d".into()), scope: Scope::Control };
    let stepped = crate::step_up::Stepped { method: "member.set_access".into(), digest: [0; 32], device: "phone-d".into(), browser: false };
    let guard = authorize(
        &r.svc,
        &peer,
        None,
        Some(&stepped),
        "member.set_access",
        Change::Access { handle: "carol", access: Access::Member },
    )
    .await
    .expect("decided for an Admin");
    crate::person_access::set_access(&r.svc, "dan", Access::Member, || ()).await.unwrap();
    let late = crate::person_access::set_access_guarded(&r.svc, "carol", Access::Member, guard, || ()).await;
    assert_eq!(late.err().map(|e| e.what().to_string()).as_deref(), Some("access_changed"));
    assert_eq!(r.svc.store.member_access("carol").unwrap(), Some(Access::Viewer), "nothing was written");
}

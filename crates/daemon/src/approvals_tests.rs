//! Phone approval (ov-858): Dan is an Admin whose iPad (`pad-d`, standing in
//! for a Mac that isn't the runner) holds no step-up key, and whose phone
//! (`phone-d`) holds a trusted one. Every test goes through `Rpc` as each
//! device's connection would.

use farcooler_protocol::v1::{self as pb, Request, request, result};
use sha2::{Digest, Sha256};

use super::*;
use crate::rpc_people::tests::call;
use crate::step_up::tests::{Device, Runner, lines, revoke_req, runner, word};

const MAC: &str = "pad-d";

fn phone() -> Device {
    Device::new("phone-d", 21)
}

fn ask_req(req: &Request) -> Request {
    let payload = request::Payload::ApprovalAsk(pb::ApprovalAsk { action_bytes: step_up::action_bytes(req).into() });
    Request { method: "approval.ask".into(), payload: Some(payload), ..Default::default() }
}

fn id_req(method: &str, id: &str) -> Request {
    Request { method: method.into(), payload: Some(request::Payload::ApprovalId(pb::ApprovalId { id: id.into() })), ..Default::default() }
}

async fn asked(r: &Runner, mac: &str, req: &Request) -> pb::ApprovalState {
    match call(&r.conn(mac), ask_req(req)).await.expect("an approval") {
        result::Value::ApprovalState(s) => s,
        other => panic!("{other:?}"),
    }
}

/// The grant `by` makes for `state`, signing the statement for `device`.
fn grant_for(by: &Device, pin: &str, state: &pb::ApprovalState, device: &str, nonce: &[u8]) -> Request {
    let digest: [u8; 32] = Sha256::digest(&state.action_bytes).into();
    let statement = farcooler_protocol::step_up_statement(pin, device, &digest, nonce);
    let payload = request::Payload::ApprovalGrant(pb::ApprovalGrant { id: state.id.clone(), host_pin: pin.into(), signature: by.sign(&statement).into() });
    Request { method: "approval.grant".into(), payload: Some(payload), ..Default::default() }
}

/// What a phone does with what it was shown: sign for the asking Mac.
fn grant(by: &Device, pin: &str, state: &pb::ApprovalState) -> Request {
    grant_for(by, pin, state, &state.device_id, &state.nonce)
}

/// The Mac's request, carrying the approval and the bytes it asked about.
fn approved(req: Request, id: &str) -> Request {
    let step_up = pb::StepUp { action_bytes: step_up::action_bytes(&req).into(), alg: ALG_APPROVAL.into(), approval_id: id.into(), ..Default::default() };
    Request { step_up: Some(step_up), ..req }
}

fn state_of(value: result::Value) -> pb::ApprovalState {
    match value {
        result::Value::ApprovalState(s) => s,
        other => panic!("{other:?}"),
    }
}

async fn ready() -> Runner {
    let r = runner().await;
    r.trust(&phone(), ALG_DEVICE);
    r
}

/// The whole round trip: the Mac asks, the phone is shown what and for which
/// Mac, signs, and the Mac's request runs. Nothing runs before the grant.
#[tokio::test]
async fn a_phone_approves_the_macs_exact_action() {
    let r = ready().await;
    let req = revoke_req("pad-b", false);
    let state = asked(&r, MAC, &req).await;
    assert_eq!((state.state.as_str(), state.method.as_str(), state.device_id.as_str()), ("pending", "client.revoke", MAC));
    assert_eq!(state.action_bytes.to_vec(), step_up::action_bytes(&req));

    let listed = match call(&r.conn("phone-d"), Request { method: "approval.list".into(), ..Default::default() }).await.unwrap() {
        result::Value::ApprovalList(l) => l.items,
        other => panic!("{other:?}"),
    };
    assert_eq!(listed.len(), 1, "the phone is shown it");
    assert!(listed[0].device_name.contains("pad") || !listed[0].device_name.is_empty(), "and which Mac asked");
    assert_eq!(listed[0].id, state.id);

    let early = word(call(&r.conn(MAC), approved(req.clone(), &state.id)).await);
    assert!(early.ends_with("approval_expired"), "{early}: not granted yet");
    assert!(lines(&r).contains(&"pad-b".to_string()));

    assert_eq!(state_of(call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await.unwrap()).state, "granted");
    let waiting = state_of(call(&r.conn(MAC), id_req("approval.status", &state.id)).await.unwrap());
    assert_eq!(waiting.state, "granted");
    call(&r.conn(MAC), approved(req, &state.id)).await.expect("the Mac's request runs");
    assert!(!lines(&r).contains(&"pad-b".to_string()), "the line is gone");
}

/// The approval is for the Mac's exact bytes: another action with the same
/// approval id, or the right bytes under another request, runs nothing.
#[tokio::test]
async fn an_approval_covers_only_the_action_it_was_granted_for() {
    let r = ready().await;
    let state = asked(&r, MAC, &revoke_req("pad-b", false)).await;
    call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await.unwrap();

    let other = word(call(&r.conn(MAC), approved(revoke_req("phone-b", false), &state.id)).await);
    assert!(other.ends_with("approval_expired"), "{other}: a different target");
    let flag = word(call(&r.conn(MAC), approved(revoke_req("pad-b", true), &state.id)).await);
    assert!(flag.ends_with("approval_expired"), "{flag}: a different flag");
    assert!(lines(&r).contains(&"pad-b".to_string()) && lines(&r).contains(&"phone-b".to_string()), "nothing ran");
    call(&r.conn(MAC), approved(revoke_req("pad-b", false), &state.id)).await.expect("the approved action still runs");

    // The bytes the approval covers, under a different request: spent, and nothing runs.
    let state = asked(&r, MAC, &revoke_req("tv-c", false)).await;
    call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await.unwrap();
    let mixed = Request { step_up: approved(revoke_req("tv-c", false), &state.id).step_up, ..revoke_req("phone-b", false) };
    let mixed = word(call(&r.conn(MAC), mixed).await);
    assert!(mixed.ends_with("step_up_failed"), "{mixed}: the bytes signed are not the request sent");
    assert!(lines(&r).contains(&"tv-c".to_string()) && lines(&r).contains(&"phone-b".to_string()), "nothing ran");
}

/// An approval is spent by the request that uses it, and it is the asking
/// Mac's alone.
#[tokio::test]
async fn a_replayed_approval_is_refused_and_so_is_another_device() {
    let r = ready().await;
    let req = revoke_req("pad-b", false);
    let state = asked(&r, MAC, &req).await;
    call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await.unwrap();

    let stolen = word(call(&r.conn("phone-d"), approved(req.clone(), &state.id)).await);
    assert!(stolen.ends_with("approval_expired"), "{stolen}: not the asking Mac");
    call(&r.conn(MAC), approved(req.clone(), &state.id)).await.expect("the first use");
    let again = word(call(&r.conn(MAC), approved(req, &state.id)).await);
    assert!(again.ends_with("approval_expired"), "{again}");
}

/// What the phone signs must be the Mac, this action and this challenge.
#[tokio::test]
async fn a_signature_for_anything_else_is_not_a_grant() {
    let r = ready().await;
    let state = asked(&r, MAC, &revoke_req("pad-b", false)).await;
    let phone = phone();
    let wrong_nonce = vec![7u8; 32];
    for (what, grant) in [
        ("the phone's own id", grant_for(&phone, &r.pin, &state, "phone-d", &state.nonce)),
        ("another challenge", grant_for(&phone, &r.pin, &state, MAC, &wrong_nonce)),
        ("another host", grant_for(&phone, "SHA256:elsewhere", &state, MAC, &state.nonce)),
    ] {
        let refused = word(call(&r.conn("phone-d"), grant).await);
        assert!(refused.contains("step_up_failed") || refused.contains("host_pin_mismatch"), "{what}: {refused}");
    }
    // The signature for another action, sent under this approval's id.
    let other = asked(&r, MAC, &revoke_req("phone-b", false)).await;
    let signed_other = grant(&phone, &r.pin, &other);
    let under_this_id = match signed_other.payload {
        Some(request::Payload::ApprovalGrant(g)) => pb::ApprovalGrant { id: state.id.clone(), ..g },
        other => panic!("{other:?}"),
    };
    let swapped = word(call(&r.conn("phone-d"), Request { payload: Some(request::Payload::ApprovalGrant(under_this_id)), ..signed_other }).await);
    assert!(swapped.contains("step_up_failed"), "{swapped}: a signature for the other action");
    let still = state_of(call(&r.conn(MAC), id_req("approval.status", &state.id)).await.unwrap());
    assert_eq!(still.state, "pending");
    assert!(word(call(&r.conn(MAC), approved(revoke_req("pad-b", false), &state.id)).await).ends_with("approval_expired"));
}

/// Only another device of the same person can approve; the Mac cannot approve itself.
#[tokio::test]
async fn only_the_same_persons_other_device_can_approve() {
    let r = ready().await;
    let bob = Device::new("phone-b", 22);
    r.trust(&bob, ALG_DEVICE);
    r.trust(&Device::new(MAC, 23), ALG_DEVICE);
    let state = asked(&r, MAC, &revoke_req("pad-b", false)).await;
    let theirs = word(call(&r.conn("phone-b"), grant(&bob, &r.pin, &state)).await);
    assert!(theirs.ends_with("not_yours"), "{theirs}: Bob's phone");
    let itself = word(call(&r.conn(MAC), grant(&Device::new(MAC, 23), &r.pin, &state)).await);
    assert!(itself.ends_with("not_yours"), "{itself}: the Mac itself");
    let listed = call(&r.conn("phone-b"), Request { method: "approval.list".into(), ..Default::default() }).await.unwrap();
    assert!(matches!(listed, result::Value::ApprovalList(l) if l.items.is_empty()), "Bob is shown nothing of Dan's");
}

/// Denying or letting it lapse signs nothing; and the asks that make no sense are refused.
#[tokio::test]
async fn a_denied_or_expired_approval_runs_nothing_and_a_bad_ask_is_refused() {
    let r = ready().await;
    let req = revoke_req("pad-b", false);
    let state = asked(&r, MAC, &req).await;
    call(&r.conn("phone-d"), id_req("approval.deny", &state.id)).await.unwrap();
    assert!(word(call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await).ends_with("approval_expired"), "denied stays denied");
    assert!(word(call(&r.conn(MAC), approved(req.clone(), &state.id)).await).ends_with("approval_expired"));

    let state = asked(&r, MAC, &req).await;
    call(&r.conn("phone-d"), grant(&phone(), &r.pin, &state)).await.unwrap();
    let digest: [u8; 32] = Sha256::digest(step_up::action_bytes(&req)).into();
    assert!(!r.svc.approvals.spend(&state.id, MAC, &digest, crate::step_up_rpc::now_ms() + TTL_MS + 1), "a lapsed approval is spent on nothing");

    let member = word(call(&r.conn("phone-b"), ask_req(&req)).await);
    assert!(member.ends_with("not_admin"), "{member}: a Member");
    let plain = Request { method: "daemon.shutdown".into(), ..Default::default() };
    assert!(word(call(&r.conn(MAC), ask_req(&plain)).await).ends_with("step_up_unneeded"), "a method that takes no step-up");
    assert!(word(call(&r.socket(), ask_req(&req)).await).ends_with("step_up_unneeded"), "the runner's own socket needs none");
}

/// With no phone holding a trusted key, there is nobody to ask.
#[tokio::test]
async fn a_mac_with_no_phone_key_has_nobody_to_ask() {
    let r = runner().await;
    let none = word(call(&r.conn(MAC), ask_req(&revoke_req("pad-b", false))).await);
    assert!(none.ends_with("no_approver"), "{none}");
}

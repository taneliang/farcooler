//! Add Teammate with a person and an access, the Access menu and Limit to
//! Viewer, signed by a phone (ov-856): the whole way an app sends them, to the
//! daemon's own verifier.

use farcooler_core::base64;
use serde_json::{Value, json};

use super::super::dispatch;
use super::{Key, a_runner_with_an_owners_phone, begin, refusal, trust};

const TEAMMATE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINroX514r9TsikukQR9fbshZfnFY4OWynj6f9rVcAZAd x";

fn add_args(access: &str) -> Value {
    json!({
        "publicKey": TEAMMATE_KEY, "label": "Erin's iPhone", "clientId": "erin-x", "scope": if access == "viewer" { "read" } else { "control" }, "forTeammate": true,
        "teammate": { "handle": "erin", "name": "Erin", "account": "acct-erin", "email": "", "access": access },
    })
}

async fn signed(session: &crate::session::Session, key: &Key, method: &str, args: Value, pin: &str) -> Result<Value, crate::session::SessionError> {
    let prepared = begin(session, method, args, pin).await;
    let signature = key.sign(&base64::decode(prepared["statement"].as_str().unwrap()).unwrap());
    dispatch(session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await
}

/// One signature adds the line and binds Erin with the access, on the runner.
/// Red if the person or the access is dropped from what `begin` builds.
#[tokio::test]
async fn one_signature_adds_a_teammate_with_an_access() {
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(21);
    trust(&runner, &key);
    signed(&session, &key, "client.enroll", add_args("viewer"), &pin).await.expect("a signed Add Teammate");
    let erin = runner.service.store.member("erin").expect("Erin is here");
    assert_eq!(erin.devices, ["erin-x"]);
    assert_eq!(runner.service.store.member_access("erin").unwrap(), Some(farcooler_store::access::Access::Viewer));
}

/// The same request is refused unsigned, and the runner writes nothing.
#[tokio::test]
async fn the_plain_call_is_refused() {
    let (runner, session, _pin) = a_runner_with_an_owners_phone().await;
    trust(&runner, &Key::new(22));
    let args = add_args("member");
    let said = refusal(dispatch(&session, "client.enroll", &args).await);
    assert!(said.contains("step_up_needed"), "{said}");
    assert!(runner.service.store.member("erin").is_err());
}

/// The Access menu and Limit to Viewer, signed.
#[tokio::test]
async fn the_access_menu_and_limit_to_viewer_are_signed() {
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(23);
    trust(&runner, &key);
    runner.service.store.set_member("bob", "Bob", &["pad-x".to_string()]).unwrap();
    runner.service.store.ensure_member_access("bob", farcooler_store::access::Access::Member).unwrap();
    signed(&session, &key, "member.set_access", json!({ "handle": "bob", "access": "viewer" }), &pin).await.expect("Access");
    assert_eq!(runner.service.store.member_access("bob").unwrap(), Some(farcooler_store::access::Access::Viewer));
    signed(&session, &key, "client.set_cap", json!({ "clientId": "pad-x", "cap": "viewer" }), &pin).await.expect("Limit to Viewer");
    assert_eq!(runner.service.store.device_cap("pad-x").unwrap(), Some(farcooler_store::access::Cap::Viewer));
}

/// Unsigned, the same two calls are refused by a runner that holds this phone's
/// key, and change nothing.
#[tokio::test]
async fn the_plain_access_calls_are_refused_for_a_phone_with_a_key() {
    let (runner, session, _pin) = a_runner_with_an_owners_phone().await;
    trust(&runner, &Key::new(24));
    runner.service.store.set_member("bob", "Bob", &["pad-x".to_string()]).unwrap();
    runner.service.store.ensure_member_access("bob", farcooler_store::access::Access::Member).unwrap();
    for (method, args) in [
        ("member.set_access", json!({ "handle": "bob", "access": "viewer" })),
        ("client.set_cap", json!({ "clientId": "pad-x", "cap": "viewer" })),
    ] {
        let said = refusal(dispatch(&session, method, &args).await);
        assert!(said.contains("step_up_needed"), "{method}: {said}");
    }
    assert_eq!(runner.service.store.member_access("bob").unwrap(), Some(farcooler_store::access::Access::Member));
    assert_eq!(runner.service.store.device_cap("pad-x").unwrap(), None);
}

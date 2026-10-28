//! `teammate.enroll` as an app passes it (ov-854): what is refused before the
//! wire, and that the arm is there.

use farcooler_protocol::v1::Scope;
use serde_json::json;

use super::phone_path_tests::a_runner;
use super::{dispatch, route::route};
use crate::session::Session;

fn args(scope: &str) -> serde_json::Value {
    json!({
        "publicKey": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob-phone",
        "label": "Bob’s iPhone", "clientId": "phone-bob", "scope": scope, "nodeKey": "",
        "handle": "bob", "name": "Bob Smith", "account": "user_bob", "email": "bob@example.com",
    })
}

/// **A teammate is never given more than Control**, and the ask never leaves
/// the phone: `host_admin` and a made-up word are refused here. Goes red when
/// the scope check is dropped.
#[tokio::test]
async fn a_teammate_is_never_enrolled_above_control() {
    let runner = a_runner(Scope::HostAdmin).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    for scope in ["host_admin", "admin", ""] {
        let refused = dispatch(&session, "teammate.enroll", &args(scope)).await.unwrap_err();
        assert!(refused.to_string().contains("scope") || refused.to_string().contains("read or control"), "{scope}: {refused}");
    }
    let listed = dispatch(&session, "client.list", &json!({})).await.unwrap();
    assert!(listed["clients"].as_array().unwrap().is_empty(), "nothing was written: {listed}");
}

/// The arm is routed, answers an outcome word, and a missing field is named
/// before the wire.
#[tokio::test]
async fn the_arm_answers_an_outcome_and_names_what_is_missing() {
    let runner = a_runner(Scope::HostAdmin).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let done = dispatch(&session, "teammate.enroll", &args("control")).await.expect("teammate.enroll");
    assert_eq!(done["outcome"], "written", "{done}");

    let mut missing = args("control");
    missing.as_object_mut().unwrap().remove("account");
    let refused = dispatch(&session, "teammate.enroll", &missing).await.unwrap_err();
    assert!(refused.to_string().contains("needs a account"), "{refused}");
    // Not a wire method: the phones can't name `member.set` or `client.revoke` by it.
    assert_eq!(route(farcooler_protocol::method::Method::MemberSet), None);
}

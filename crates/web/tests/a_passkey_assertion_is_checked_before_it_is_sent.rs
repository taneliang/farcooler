//! What the browser checks before a passkey's signature leaves it (ov-767).
//!
//! sshd checks every one of these too, so none of them is the runner's only
//! defense. They are here so this browser never offers a runner a signature
//! that isn't plainly its own login: for this build's RP ID, touched, over
//! exactly the data SSH asked to have signed, from this build's own origin in
//! a top-level page. Each test goes red if its check in
//! `passkey::signature_blob_for` is removed.

mod soft_passkey;

use std::sync::atomic::Ordering;

use farcooler_web::passkey::{self, PasskeySigner};
use russh::keys::agent::AgentIdentity;
use russh::keys::PublicKey;
use russh::Signer as _;
use soft_passkey::{SoftPasskey, Tamper};

const RP: &str = "app.canary.farcooler.com";
const DATA: &[u8] = b"the SSH user-auth data russh asks to have signed";

fn refusal(tamper: Tamper) -> String {
    let assertion = SoftPasskey::with(tamper).assertion(RP, DATA);
    passkey::signature_blob_for(RP, DATA, &assertion).expect_err("a wrong assertion was encoded")
}

#[test]
fn a_good_assertion_encodes() {
    let assertion = SoftPasskey::new().assertion(RP, DATA);
    let blob = passkey::signature_blob_for(RP, DATA, &assertion).unwrap();
    assert!(blob.starts_with(b"\0\0\0\x2bwebauthn-sk-ecdsa-sha2-nistp256@openssh.com"));
}

/// Extension outputs (authenticatorData after byte 37, present when the ED
/// flag is set) travel as the last string, as PROTOCOL.u2f lays them out. The
/// relay's fixture has none, so this is what holds them in place.
#[test]
fn extension_outputs_are_the_last_field() {
    let mut assertion = SoftPasskey::new().assertion(RP, DATA);
    let extensions = [0xa1, 0x63, b'f', b'o', b'o', 0xf5];
    assertion.authenticator_data.extend_from_slice(&extensions);
    let blob = passkey::signature_blob_for(RP, DATA, &assertion).unwrap();
    let mut tail = (extensions.len() as u32).to_be_bytes().to_vec();
    tail.extend_from_slice(&extensions);
    assert!(blob.ends_with(&tail));
    let mut client = (assertion.client_data_json.len() as u32).to_be_bytes().to_vec();
    client.extend_from_slice(&assertion.client_data_json);
    client.extend_from_slice(&tail);
    assert!(blob.ends_with(&client), "the client data is not just before the extensions");
}

#[test]
fn an_assertion_for_another_rp_id_is_refused() {
    assert!(refusal(Tamper::ParentRpId).contains("not for app.canary.farcooler.com"));
}

/// R-89: user presence is required (user verification is not).
#[test]
fn an_assertion_nobody_touched_is_refused() {
    assert!(refusal(Tamper::NoTouch).contains("no touch"));
}

#[test]
fn an_assertion_over_other_data_is_refused() {
    assert!(refusal(Tamper::OtherChallenge).contains("other than this login"));
}

#[test]
fn an_assertion_from_another_origin_is_refused() {
    assert!(refusal(Tamper::OtherOrigin).contains("came from https://canary.farcooler.com"));
}

#[test]
fn an_assertion_from_a_framed_page_is_refused() {
    assert!(refusal(Tamper::CrossOrigin).contains("inside a frame"));
    assert!(refusal(Tamper::TopOrigin).contains("inside a frame"));
}

#[test]
fn a_create_ceremonys_client_data_is_refused() {
    assert!(refusal(Tamper::Create).contains("not a webauthn.get"));
}

#[test]
fn a_short_authenticator_data_or_a_bad_der_signature_is_refused() {
    assert!(refusal(Tamper::Short).contains("shorter than 37"));
    assert!(refusal(Tamper::BadDer).contains("not a DER ECDSA signature"));
}

/// The key line is this build's RP ID's, from an uncompressed P-256 SPKI and
/// nothing else.
#[test]
fn a_public_key_becomes_an_sk_line_for_this_builds_host_or_is_refused() {
    let passkey = SoftPasskey::new();
    let line = passkey::public_line(&passkey.spki(), "farcooler-web").unwrap();
    let key = PublicKey::from_openssh(&line).unwrap();
    assert_eq!(key.key_data().sk_ecdsa_p256().unwrap().application(), passkey::rp_id());
    assert_eq!(passkey::rp_id(), farcooler_protocol::CHANNEL.app_host());

    let mut compressed = passkey.spki();
    compressed[26] = 0x02;
    assert!(passkey::public_line(&compressed, "x").is_err());
    let mut other_curve = passkey.spki();
    other_curve[22] = 0x08;
    assert!(passkey::public_line(&other_curve, "x").is_err());
    assert!(passkey::public_line(&passkey.spki()[..90], "x").is_err());
}

/// A key for any other host is never shown to the authenticator: no prompt,
/// no touch, and a refusal.
#[tokio::test(flavor = "current_thread")]
async fn a_key_for_another_host_never_reaches_the_authenticator() {
    let passkey = SoftPasskey::new();
    let line = passkey::public_line_for("farcooler.com", &passkey.spki(), "x").unwrap();
    let identity = AgentIdentity::PublicKey { key: PublicKey::from_openssh(&line).unwrap(), comment: String::new() };
    let mut signer = PasskeySigner(passkey.authenticator());
    assert!(signer.auth_sign(&identity, None, DATA.to_vec()).await.is_err());
    assert_eq!(passkey.touches.load(Ordering::SeqCst), 0);

    let ours = passkey::public_line(&passkey.spki(), "x").unwrap();
    let identity = AgentIdentity::PublicKey { key: PublicKey::from_openssh(&ours).unwrap(), comment: String::new() };
    assert!(signer.auth_sign(&identity, None, DATA.to_vec()).await.is_ok());
    assert_eq!(passkey.touches.load(Ordering::SeqCst), 1);
}

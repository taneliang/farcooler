//! A phone's step-up (ov-856, slice 4), the whole way it sends it: `dispatch`
//! with the JSON an app passes, a `Session`, and the daemon's own verifier. A
//! software P-256 key stands where the Secure Enclave or the Keystore would;
//! what the apps add is the key and the face, and nothing else is signed
//! anywhere but here.

use farcooler_core::base64;
use farcooler_protocol::v1::{ClientEnroll, Scope};
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::dispatch;
use super::phone_path_tests::{Runner, a_runner_with};
use crate::session::Session;

const HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBzxEtM7v44iP1iwf+YAjcmFtGDt61RQYGY3zmCTN0Ke x";
const VICTIM_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINqoX514r9TsikukQR9fbshZfnFY4OWynj6f9rVcAZAd x";

/// A device key in software, signing as an enclave would: DER over the
/// SHA-256 of the statement.
pub(super) struct Key(SigningKey);

impl Key {
    pub(super) fn new(n: u8) -> Key {
        Key(SigningKey::from_bytes(&[n; 32].into()).unwrap())
    }

    pub(super) fn public(&self) -> Vec<u8> {
        self.0.verifying_key().to_sec1_point(false).as_bytes().to_vec()
    }

    pub(super) fn sign(&self, statement: &[u8]) -> String {
        let signature: Signature = self.0.sign_prehash(&Sha256::digest(statement)).unwrap();
        base64::encode(signature.to_der().as_bytes())
    }
}

/// A runner whose connection is `phone-o`, the owner's `control` phone, with
/// a host key to pin and a second device (`pad-x`) to act on.
pub(super) async fn a_runner_with_an_owners_phone() -> (Runner, Session, String) {
    // In the runner's own scratch directory, which is removed with it.
    let runner = a_runner_with(Scope::Control, Some("phone-o"), |s, scratch| {
        let hostkeys = scratch.join("etc-ssh");
        std::fs::create_dir(&hostkeys).unwrap();
        std::fs::write(hostkeys.join("ssh_host_ed25519_key.pub"), HOST_KEY).unwrap();
        s.with_ssh_host_dir(hostkeys)
    })
    .await;
    farcooler_daemon::access_migration::migrate_at_start(&runner.service).await;
    runner.service.store.bind_owner_device("phone-o", None).unwrap();
    let victim = ClientEnroll {
        public_key: VICTIM_KEY.into(),
        label: "pad-x".into(),
        client_id: "pad-x".into(),
        scope: Scope::Read as i32,
        ..Default::default()
    };
    farcooler_daemon::enrollment::enroll(&runner.service, &victim).await.expect("a device to act on");
    let pin = farcooler_daemon::step_up::host_pins(runner.service.ssh_host_dir()).remove(0);
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    (runner, session, pin)
}

pub(super) fn trust(runner: &Runner, key: &Key) {
    runner.service.store.put_step_up_key("phone-o", "p256-sha256", &key.public(), true, 1).unwrap();
}

pub(super) async fn begin(session: &Session, method: &str, args: Value, pin: &str) -> Value {
    dispatch(session, "step_up.begin", &json!({ "method": method, "args": args, "clientId": "phone-o", "hostPin": pin })).await.expect("begin")
}

/// What a refused call said, or a failure that it was served.
pub(super) fn refusal(result: Result<Value, crate::session::SessionError>) -> String {
    match result {
        Ok(served) => panic!("served: {served}"),
        // The word the runner named, which is what an app reads.
        Err(crate::session::SessionError::Refused { what, .. }) => what,
        Err(other) => other.to_string(),
    }
}

fn clients(list: &Value) -> Vec<String> {
    list["clients"].as_array().unwrap().iter().map(|c| c["clientId"].as_str().unwrap().to_string()).collect()
}

/// A revoke from a phone that holds a trusted key is refused plain and
/// served signed, by the statement `begin` built; a used token is spent.
/// Red if `begin` signs other bytes than it sends, drops the client id from
/// the statement, or `finish` keeps a spent token.
#[tokio::test]
async fn a_signed_revoke_goes_through_the_bridge() {
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(11);
    trust(&runner, &key);

    let plain = dispatch(&session, "client.revoke", &json!({ "clientId": "pad-x" })).await;
    { let said = refusal(plain); assert!(said.contains("step_up_needed"), "{said}"); }

    let prepared = begin(&session, "client.revoke", json!({ "clientId": "pad-x" }), &pin).await;
    let statement = base64::decode(prepared["statement"].as_str().unwrap()).unwrap();
    let signature = key.sign(&statement);
    let left = dispatch(&session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await.expect("a signed revoke");
    assert!(!clients(&left).contains(&"pad-x".to_string()), "{left}");

    let again = dispatch(&session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await;
    { let said = refusal(again); assert!(said.contains("already used"), "{said}"); }
}

/// One prompt signs one action: a signature made for one revoke does not
/// carry another, and a statement for another device's id is refused.
#[tokio::test]
async fn a_signature_carries_only_its_own_action() {
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(12);
    trust(&runner, &key);
    let first = begin(&session, "client.revoke", json!({ "clientId": "pad-x" }), &pin).await;
    let second = begin(&session, "client.revoke", json!({ "clientId": "phone-o" }), &pin).await;
    // The first prompt's signature on the second action.
    let crossed = key.sign(&base64::decode(first["statement"].as_str().unwrap()).unwrap());
    let refused = dispatch(&session, "step_up.finish", &json!({ "token": second["token"], "signature": crossed })).await;
    { let said = refusal(refused); assert!(said.contains("step_up_failed"), "{said}"); }
    let other_device = dispatch(&session, "step_up.begin", &json!({ "method": "client.revoke", "args": { "clientId": "pad-x" }, "clientId": "pad-y", "hostPin": pin })).await.unwrap();
    let signed = key.sign(&base64::decode(other_device["statement"].as_str().unwrap()).unwrap());
    let refused = dispatch(&session, "step_up.finish", &json!({ "token": other_device["token"], "signature": signed })).await;
    { let said = refusal(refused); assert!(said.contains("step_up_failed"), "{said}"); }
    let wrong_pin = begin(&session, "client.revoke", json!({ "clientId": "pad-x" }), "SHA256:not-the-runner").await;
    let signed = key.sign(&base64::decode(wrong_pin["statement"].as_str().unwrap()).unwrap());
    let refused = dispatch(&session, "step_up.finish", &json!({ "token": wrong_pin["token"], "signature": signed })).await;
    { let said = refusal(refused); assert!(said.contains("host_pin_mismatch"), "{said}"); }
    assert!(farcooler_daemon::step_up::host_pins(runner.service.ssh_host_dir()).len() == 1);
}

/// A phone proposes its key and gets the digits to show; the runner's own
/// code is the one it answers. A method that takes no step-up never leaves
/// the phone.
#[tokio::test]
async fn a_phone_proposes_its_key_and_is_told_its_code() {
    let (runner, session, _pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(13);
    let proposed = dispatch(&session, "step_up.propose", &json!({ "publicKey": base64::encode(&key.public()) })).await.expect("a proposal");
    assert_eq!((proposed["clientId"].as_str(), proposed["state"].as_str()), (Some("phone-o"), Some("proposed")));
    let code = proposed["code"].as_str().unwrap();
    assert_eq!(code.len(), 6, "{proposed}");
    assert_eq!(code, farcooler_daemon::step_up::code_of("p256-sha256", &key.public()));
    assert!(!runner.service.store.step_up_key("phone-o").unwrap().unwrap().trusted, "a proposal signs nothing");
    let none = dispatch(&session, "step_up.begin", &json!({ "method": "terminal.write", "args": {}, "clientId": "phone-o", "hostPin": "SHA256:x" })).await;
    { let said = refusal(none); assert!(said.contains("takes no step-up"), "{said}"); }
    let garbled = dispatch(&session, "step_up.propose", &json!({ "publicKey": "not base64!" })).await;
    assert!(garbled.is_err());
}

/// A trusted device approves its own person's other device by the code that
/// device shows, signed, through the bridge.
#[tokio::test]
async fn a_trusted_device_approves_the_others_code() {
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(14);
    trust(&runner, &key);
    runner.service.store.bind_owner_device("pad-x", None).ok();
    let other = Key::new(15);
    runner.service.store.put_step_up_key("pad-x", "p256-sha256", &other.public(), false, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64).unwrap();
    let code = farcooler_daemon::step_up::code_of("p256-sha256", &other.public());
    let wrong = begin(&session, "client.approve_step_up", json!({ "clientId": "pad-x", "code": "000000" }), &pin).await;
    let signed = key.sign(&base64::decode(wrong["statement"].as_str().unwrap()).unwrap());
    let refused = dispatch(&session, "step_up.finish", &json!({ "token": wrong["token"], "signature": signed })).await;
    { let said = refusal(refused); assert!(said.contains("wrong_code"), "{said}"); }
    let right = begin(&session, "client.approve_step_up", json!({ "clientId": "pad-x", "code": code }), &pin).await;
    let signed = key.sign(&base64::decode(right["statement"].as_str().unwrap()).unwrap());
    let state = dispatch(&session, "step_up.finish", &json!({ "token": right["token"], "signature": signed })).await.expect("approved");
    assert_eq!(state["state"], "trusted", "{state}");
}

#[path = "step_up_access_tests.rs"]
mod access;

/// The owner's "Allow cutover once" from their phone (ov-974, ov-977): built
/// and signed through the bridge as the apps do, it reaches the runner as the
/// owner's answer said from their device, and grants. The same answer plain
/// is still an answer, and grants nothing; a signed one is good once.
#[tokio::test]
async fn the_owners_signed_cutover_answer_goes_through_the_bridge() {
    use farcooler_daemon::cutover_grant::{ALLOW, SAID_FROM, find};
    use farcooler_store::models::{Actor, NoteKind};
    let (runner, session, pin) = a_runner_with_an_owners_phone().await;
    let key = Key::new(15);
    trust(&runner, &key);
    let store = &runner.service.store;
    let root = store.create_repository_root(runner.service.host_id, "/r", 0).unwrap();
    let repo = store.create_repository(runner.service.host_id, root.id, "demo", "/r/demo/.git", "").unwrap();
    let workspace = store.ensure_main_workspace(repo.id).unwrap().id;
    let task = store.create_task(workspace, "Data: the engine cutover", Actor::Manager).unwrap().id;
    store.add_note(task, NoteKind::Question, Actor::Manager, "Run it once?", json!({ "options": [ALLOW, "Wait for me"] })).unwrap();
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let args = json!({ "task": task.to_string(), "kind": "answer", "body": ALLOW });

    dispatch(&session, "task.note", &args).await.expect("a plain answer is still an answer");
    assert_eq!(find(store, workspace, now()).unwrap(), None, "unsigned, it grants nothing");

    // Trusted on the walk alone, the key signs and the grant is refused,
    // until the owner vouches for it with presence (B1 of the second review).
    let prepared = begin(&session, "task.note", args.clone(), &pin).await;
    let signature = key.sign(&base64::decode(prepared["statement"].as_str().unwrap()).unwrap());
    let unvouched = dispatch(&session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await;
    { let said = refusal(unvouched); assert!(said.contains("key_unvouched"), "{said}"); }
    store.vouch_step_up_key("phone-o", "local:app").unwrap();

    let prepared = begin(&session, "task.note", args.clone(), &pin).await;
    let signature = key.sign(&base64::decode(prepared["statement"].as_str().unwrap()).unwrap());
    let said = dispatch(&session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await.expect("a signed answer");
    assert_eq!(said["body"].as_str(), Some(ALLOW), "{said}");
    let written = store.notes_for(task, Some(NoteKind::Answer)).unwrap().pop().unwrap();
    assert_eq!((written.actor, written.extra[SAID_FROM].as_str()), (Actor::User, Some("device")));
    assert!(find(store, workspace, now()).unwrap().is_some(), "the owner's phone, by its key");

    let again = dispatch(&session, "step_up.finish", &json!({ "token": prepared["token"], "signature": signature })).await;
    { let said = refusal(again); assert!(said.contains("already used"), "{said}"); }
    let other = dispatch(&session, "step_up.begin", &json!({ "method": "task.note", "args": { "task": task.to_string(), "kind": "comment", "body": "x" }, "clientId": "phone-o", "hostPin": pin })).await;
    assert!(other.is_err(), "a signed note is an answer and only an answer");
}

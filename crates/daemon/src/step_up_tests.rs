//! Step-up on the runner (ov-856, slice 3): a signature that covers exactly
//! one request, once, from a key a person approved. Every test goes through
//! `Rpc` the way a phone's connection does, except the clock tests, which
//! call `admit` with the time they want.

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};

use super::*;
use crate::step_up_rpc::now_ms;
use farcooler_store::access::Access;
use crate::rpc_people::tests::{call, factory, set};

fn key(n: u8) -> String {
    use ssh_key::public::{Ed25519PublicKey, KeyData};
    ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([n; 32])), "x").to_openssh().unwrap()
}

fn asking(n: u8, id: &str, scope: Scope) -> pb::ClientEnroll {
    pb::ClientEnroll {
        public_key: key(n),
        label: id.into(),
        client_id: id.into(),
        scope: scope as i32,
        shell_access: false,
        node_key: String::new(),
        for_teammate: false,
        step_up_key: None,
    }
}

/// A device's key, in software, as a Secure Enclave's would sign.
struct Device {
    id: &'static str,
    key: SigningKey,
}

impl Device {
    fn new(id: &'static str, n: u8) -> Device {
        Device { id, key: SigningKey::from_bytes(&[n; 32].into()).unwrap() }
    }

    fn public(&self) -> Vec<u8> {
        self.key.verifying_key().to_sec1_point(false).as_bytes().to_vec()
    }

    fn wire_key(&self, alg: &str) -> pb::StepUpKey {
        pb::StepUpKey { alg: alg.into(), public_key: self.public().into() }
    }

    fn sign(&self, statement: &[u8]) -> Vec<u8> {
        let signature: Signature = self.key.sign_prehash(&Sha256::digest(statement)).unwrap();
        signature.to_der().as_bytes().to_vec()
    }

    /// `req`, carrying a `p256-sha256` step-up made with `nonce`.
    fn step(&self, pin: &str, req: Request, nonce: &[u8]) -> Request {
        let bytes = action_bytes(&req);
        let statement = farcooler_protocol::step_up_statement(pin, self.id, &Sha256::digest(&bytes).into(), nonce);
        let step_up = pb::StepUp {
            action_bytes: bytes.into(),
            alg: ALG_DEVICE.into(),
            nonce: nonce.to_vec().into(),
            host_pin: pin.into(),
            signature: self.sign(&statement).into(),
            ..Default::default()
        };
        Request { step_up: Some(step_up), ..req }
    }

    /// `req`, carrying a passkey's assertion: `flags` the authenticator's,
    /// `count` its counter, `origin` the page's.
    fn assert(&self, pin: &str, req: Request, nonce: &[u8], flags: u8, count: u32, origin: &str) -> Request {
        let bytes = action_bytes(&req);
        let statement = farcooler_protocol::step_up_statement(pin, self.id, &Sha256::digest(&bytes).into(), nonce);
        let challenge = Base64UrlUnpadded::encode_string(&Sha256::digest(&statement));
        let client_data = format!(r#"{{"type":"webauthn.get","challenge":"{challenge}","origin":"{origin}"}}"#).into_bytes();
        let mut auth = Sha256::digest(farcooler_fence::app_host().as_bytes()).to_vec();
        auth.push(flags);
        auth.extend_from_slice(&count.to_be_bytes());
        let mut signed = auth.clone();
        signed.extend_from_slice(&Sha256::digest(&client_data));
        let step_up = pb::StepUp {
            action_bytes: bytes.into(),
            alg: ALG_WEBAUTHN.into(),
            nonce: nonce.to_vec().into(),
            host_pin: pin.into(),
            signature: self.sign(&signed).into(),
            authenticator_data: auth.into(),
            client_data_json: client_data.into(),
        };
        Request { step_up: Some(step_up), ..req }
    }
}

const UP_UV: u8 = 0x05;

struct Runner {
    _dir: tempfile::TempDir,
    svc: Arc<Service>,
    pin: String,
}

impl Runner {
    fn socket(&self) -> crate::rpc::RpcFactory {
        factory(&self.svc, None, Scope::HostAdmin)
    }

    /// A connection as `id`'s line makes it: `control`.
    fn conn(&self, id: &str) -> crate::rpc::RpcFactory {
        factory(&self.svc, Some(id), Scope::Control)
    }

    /// The device's key, trusted as the socket would.
    fn trust(&self, device: &Device, alg: &str) {
        self.svc.store.put_step_up_key(device.id, alg, &device.public(), true, now_ms()).unwrap();
    }
}

async fn nonce(conn: &crate::rpc::RpcFactory) -> Vec<u8> {
    match call(conn, Request { method: "auth.challenge".into(), ..Default::default() }).await.expect("a challenge") {
        result::Value::AuthChallenge(c) => c.nonce.to_vec(),
        other => panic!("{other:?}"),
    }
}

/// The owner has a control phone and an iPad and a browser; Bob is a Member
/// with a phone and a pad; Carol a Viewer; Dan an Admin with a control phone.
async fn runner() -> Runner {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let hostkeys = dir.path().join("etc-ssh");
    std::fs::create_dir_all(&hostkeys).unwrap();
    std::fs::write(hostkeys.join("ssh_host_ed25519_key.pub"), key(90)).unwrap();
    let mut svc = Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys"));
    svc.ssh_host_dir = hostkeys;
    let svc = Arc::new(svc);
    crate::access_migration::migrate_at_start(&svc).await;
    for (n, id, scope) in [
        (1, "phone-o", Scope::Control),
        (2, "ipad-o", Scope::Read),
        (3, "web-o", Scope::Control),
        (4, "phone-b", Scope::Control),
        (5, "pad-b", Scope::Read),
        (6, "tv-c", Scope::Read),
        (7, "phone-d", Scope::Control),
        (8, "pad-d", Scope::Control),
    ] {
        crate::enrollment::enroll(&svc, &asking(n, id, scope)).await.unwrap_or_else(|e| panic!("{id}: {e:?}"));
    }
    let socket = factory(&svc, None, Scope::HostAdmin);
    for (handle, devices) in [("bob", &["phone-b", "pad-b"][..]), ("carol", &["tv-c"]), ("dan", &["phone-d", "pad-d"])] {
        call(&socket, set(handle, devices, "", "")).await.unwrap_or_else(|e| panic!("{handle}: {e}"));
    }
    crate::person_access::set_access(&svc, "carol", Access::Viewer, || ()).await.unwrap();
    crate::person_access::set_access(&svc, "dan", Access::Admin, || ()).await.unwrap();
    let pin = host_pins(&svc.ssh_host_dir).remove(0);
    Runner { _dir: dir, svc, pin }
}

fn revoke_req(device: &str, with_added: bool) -> Request {
    Request {
        method: "client.revoke".into(),
        payload: Some(request::Payload::ClientRevoke(pb::ClientRevoke { client_id: device.into(), with_added })),
        ..Default::default()
    }
}

fn remove_req(handle: &str) -> Request {
    Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    }
}

fn enroll_req(n: u8, id: &str, scope: Scope, shell: bool, key: Option<pb::StepUpKey>) -> Request {
    let p = pb::ClientEnroll { shell_access: shell, step_up_key: key, ..asking(n, id, scope) };
    Request { method: "client.enroll".into(), payload: Some(request::Payload::ClientEnroll(p)), ..Default::default() }
}

fn lines(r: &Runner) -> Vec<String> {
    farcooler_fence::read(r.svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS).unwrap().into_iter().map(|e| e.client_id).collect()
}

fn word(result: std::result::Result<result::Value, String>) -> String {
    result.err().unwrap_or_else(|| "(served)".into())
}

// MARK: - The happy path, and what it does not turn into

/// The owner's `control` phone, with a trusted key, revokes another device
/// with one signed request. The line it holds is `control` throughout: the
/// same phone's unsigned `daemon.shutdown` is refused, and its signed one is
/// refused for taking no step-up (B2: step-up is not `host_admin`).
#[tokio::test]
async fn a_signed_revoke_from_a_control_phone_is_served_and_widens_nothing() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let conn = r.conn("phone-o");
    let nonce = nonce(&conn).await;
    call(&conn, phone.step(&r.pin, revoke_req("pad-b", false), &nonce)).await.expect("a signed revoke");
    assert!(!lines(&r).contains(&"pad-b".to_string()), "the line is gone");

    let shutdown = Request { method: "daemon.shutdown".into(), ..Default::default() };
    assert!(word(call(&conn, shutdown.clone()).await).starts_with("scope-denied"), "unsigned, still a control line");
    let nonce = self::nonce(&conn).await;
    let stepped = word(call(&conn, phone.step(&r.pin, shutdown, &nonce)).await);
    assert!(stepped.ends_with("step_up_unneeded"), "{stepped}: step-up on a method that takes none");
}

/// A device with a trusted key that doesn't step up for an admin method is
/// refused `step_up_needed`; an owner's device with none keeps what it had.
#[tokio::test]
async fn a_device_with_a_trusted_key_must_step_up() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let with_key = word(call(&r.conn("phone-o"), revoke_req("pad-b", false)).await);
    assert!(with_key.ends_with("step_up_needed"), "{with_key}");
    // Without a key it is today's rule: revoke is `host_admin`'s, and a
    // control line is refused at the gate.
    let without = word(call(&r.conn("ipad-o"), revoke_req("pad-b", false)).await);
    assert!(without.starts_with("scope-denied"), "{without}");
    assert!(lines(&r).contains(&"pad-b".to_string()));
}

// MARK: - One assertion, one action

/// An assertion is spent by the request it came with: the same request again
/// is refused, and the line it removed stays removed.
#[tokio::test]
async fn a_replayed_assertion_is_refused() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let conn = r.conn("phone-o");
    let nonce = nonce(&conn).await;
    let signed = phone.step(&r.pin, revoke_req("pad-b", false), &nonce);
    call(&conn, signed.clone()).await.expect("the first");
    // The same bytes to the same connection, and to another one.
    assert!(word(call(&conn, signed.clone()).await).ends_with("challenge_expired"));
    assert!(word(call(&r.conn("phone-o"), signed).await).ends_with("challenge_expired"));
}

/// A signature made for one action doesn't carry another: a different
/// target, a different flag, and a different envelope field are each refused
/// and nothing runs.
#[tokio::test]
async fn an_assertion_is_for_that_action_and_those_arguments() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let conn = r.conn("phone-o");

    // Signed for pad-b, sent as tv-c.
    let n = nonce(&conn).await;
    let mut swapped = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    swapped.payload = revoke_req("tv-c", false).payload;
    assert!(word(call(&conn, swapped).await).ends_with("step_up_failed"), "wrong action target");

    // Signed without `with_added`, sent with it.
    let n = nonce(&conn).await;
    let mut wider = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    wider.payload = revoke_req("pad-b", true).payload;
    assert!(word(call(&conn, wider).await).ends_with("step_up_failed"), "wrong arguments");

    // Signed for another request's bytes under this one's statement.
    let n = nonce(&conn).await;
    let mut other_bytes = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    let theirs = action_bytes(&revoke_req("tv-c", false));
    other_bytes.step_up.as_mut().unwrap().action_bytes = theirs.into();
    other_bytes.payload = revoke_req("tv-c", false).payload;
    assert!(word(call(&conn, other_bytes).await).ends_with("step_up_failed"), "a signature over other bytes");

    // The envelope is signed too: a precondition added afterwards.
    let n = nonce(&conn).await;
    let mut envelope = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    envelope.idempotency_key = Some("another".into());
    assert!(word(call(&conn, envelope).await).ends_with("step_up_failed"), "the envelope");

    // A field this runner doesn't know, appended to the signed bytes.
    let n = nonce(&conn).await;
    let mut unknown = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    let mut bytes = action_bytes(&revoke_req("pad-b", false));
    bytes.extend_from_slice(&[0x78, 0x01]);
    let statement = farcooler_protocol::step_up_statement(&r.pin, "phone-o", &Sha256::digest(&bytes).into(), &n);
    let step_up = unknown.step_up.as_mut().unwrap();
    step_up.signature = phone.sign(&statement).into();
    step_up.action_bytes = bytes.into();
    assert!(word(call(&conn, unknown).await).ends_with("step_up_failed"), "an unknown field");

    for line in ["pad-b", "tv-c"] {
        assert!(lines(&r).contains(&line.to_string()), "{line} was revoked");
    }
}

/// The host-key pin must be one of this runner's, a nonce must be this
/// session's, and a signature by another device's key is no signature.
#[tokio::test]
async fn the_pin_the_session_and_the_signer_must_all_be_right() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let conn = r.conn("phone-o");

    let n = nonce(&conn).await;
    let elsewhere = phone.step("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", revoke_req("pad-b", false), &n);
    assert!(word(call(&conn, elsewhere).await).ends_with("host_pin_mismatch"), "another runner's pin");

    let theirs = nonce(&r.conn("phone-o")).await;
    let borrowed = phone.step(&r.pin, revoke_req("pad-b", false), &theirs);
    assert!(word(call(&conn, borrowed).await).ends_with("challenge_expired"), "a nonce another session asked for");

    let n = nonce(&conn).await;
    let thief = Device::new("phone-o", 12);
    assert!(word(call(&conn, thief.step(&r.pin, revoke_req("pad-b", false), &n)).await).ends_with("step_up_failed"), "another key");

    // A statement for another device id, signed by the right key.
    let n = nonce(&conn).await;
    let other_id = Device { id: "phone-b", key: phone.key.clone() };
    assert!(word(call(&conn, other_id.step(&r.pin, revoke_req("pad-b", false), &n)).await).ends_with("step_up_failed"), "another client id");
    assert!(lines(&r).contains(&"pad-b".to_string()));
}

/// A challenge lives 60 seconds, is spent when taken, and a session holds
/// four: the oldest goes when a fifth is asked.
#[tokio::test]
async fn a_challenge_expires_and_a_session_holds_four() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let peer = Peer { client_id: Some("phone-o".into()), scope: Scope::Control };
    let session = r.svc.sessions().open(Some("phone-o".into()));
    let t0 = 1_000_000_000_i64;

    let n = session.challenges().issue(t0).unwrap();
    let stale = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    let late = admit(&r.svc, &peer, &session, stale, t0 + CHALLENGE_MS + 1).await;
    assert!(matches!(late, Err(DomainError::Conflict { what: "challenge_expired" })), "{late:?}");

    let n = session.challenges().issue(t0).unwrap();
    let on_time = phone.step(&r.pin, revoke_req("pad-b", false), &n);
    let (executed, stepped) = admit(&r.svc, &peer, &session, on_time, t0 + CHALLENGE_MS).await.expect("at the last moment");
    assert_eq!(executed.method, "client.revoke");
    assert_eq!(stepped.unwrap().device, "phone-o");

    let first = session.challenges().issue(t0).unwrap();
    for _ in 0..LIVE_CHALLENGES {
        session.challenges().issue(t0).unwrap();
    }
    assert!(!session.challenges().take(&first, t0), "the fifth ask dropped the oldest");
}

// MARK: - Keys: trusted, proposed, approved

/// A device proposes its own key and it signs nothing until a person
/// approves it with the six digits the device shows.
#[tokio::test]
async fn a_proposed_key_signs_nothing_until_it_is_approved() {
    let r = runner().await;
    let dan = Device::new("phone-d", 21);
    let conn = r.conn("phone-d");
    let propose = Request {
        method: "client.set_step_up_key".into(),
        payload: Some(request::Payload::ClientSetStepUpKey(pb::ClientSetStepUpKey { client_id: String::new(), key: Some(dan.wire_key(ALG_DEVICE)) })),
        ..Default::default()
    };
    let code = match call(&conn, propose.clone()).await.expect("a proposal") {
        result::Value::StepUpKeyState(s) => {
            assert_eq!(s.state, "proposed");
            s.code
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(code, code_of(ALG_DEVICE, &dan.public()), "the code the device shows is the key's");
    assert!(word(call(&conn, propose).await).ends_with("key_exists"), "one key per device");

    let n = nonce(&conn).await;
    let proposed = word(call(&conn, dan.step(&r.pin, remove_req("carol"), &n)).await);
    assert!(proposed.ends_with("key_untrusted"), "{proposed}");
    assert!(r.svc.store.member("carol").is_ok());

    let approve = |code: &str| Request {
        method: "client.approve_step_up".into(),
        payload: Some(request::Payload::ClientApproveStepUp(pb::ClientApproveStepUp { client_id: "phone-d".into(), code: code.into() })),
        ..Default::default()
    };
    assert!(word(call(&r.socket(), approve("000000")).await).ends_with("wrong_code"), "the wrong digits");
    assert!(!r.svc.store.step_up_key("phone-d").unwrap().unwrap().trusted);
    call(&r.socket(), approve(&code)).await.expect("the runner's own socket approves");

    let n = nonce(&conn).await;
    call(&conn, dan.step(&r.pin, remove_req("carol"), &n)).await.expect("an Admin removes a Viewer with a trusted key");
    assert!(r.svc.store.member("carol").is_err());
}

/// An approval comes from the person's own other trusted device and from no
/// one else's, and a proposal is good for ten minutes.
#[tokio::test]
async fn only_the_same_persons_trusted_device_approves_and_a_proposal_expires() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let ipad = Device::new("ipad-o", 13);
    let bob = Device::new("pad-b", 14);
    let now = now_ms();
    r.svc.store.put_step_up_key("ipad-o", ALG_DEVICE, &ipad.public(), false, now).unwrap();
    r.svc.store.put_step_up_key("pad-b", ALG_DEVICE, &bob.public(), false, now).unwrap();
    let conn = r.conn("phone-o");
    let approve = |device: &str, key: &Device| Request {
        method: "client.approve_step_up".into(),
        payload: Some(request::Payload::ClientApproveStepUp(pb::ClientApproveStepUp {
            client_id: device.into(),
            code: code_of(ALG_DEVICE, &key.public()),
        })),
        ..Default::default()
    };

    // Unsigned: the owner's phone has a trusted key, so it must step up.
    let unsigned = word(call(&conn, approve("ipad-o", &ipad)).await);
    assert!(unsigned.ends_with("step_up_needed"), "{unsigned}");
    // Bob's pad is another person's.
    let n = nonce(&conn).await;
    let theirs = word(call(&conn, phone.step(&r.pin, approve("pad-b", &bob), &n)).await);
    assert!(theirs.contains("not_yours"), "{theirs}");
    assert!(!r.svc.store.step_up_key("pad-b").unwrap().unwrap().trusted);
    // The owner's iPad is the owner's own.
    let n = nonce(&conn).await;
    call(&conn, phone.step(&r.pin, approve("ipad-o", &ipad), &n)).await.expect("the same person's");
    assert!(r.svc.store.step_up_key("ipad-o").unwrap().unwrap().trusted);

    // Eleven minutes old.
    let web = Device::new("web-o", 15);
    r.svc.store.put_step_up_key("web-o", ALG_DEVICE, &web.public(), false, now - 11 * 60 * 1000).unwrap();
    let n = nonce(&conn).await;
    let old = word(call(&conn, phone.step(&r.pin, approve("web-o", &web), &n)).await);
    assert!(old.ends_with("proposal_expired"), "{old}");
}

/// The runner's own socket from inside a pane is an agent, not a person: it
/// proposes and approves no key.
#[tokio::test]
async fn a_pane_writes_and_approves_no_key() {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let agent = svc.create_terminal(worktree.id, "Agent 2", "claude").await.expect("an agent");
    let snapshot = svc.inventory.refresh().await;
    let pid = snapshot.claimants(agent.id).into_iter().next().expect("the agent's pane").stamp.pid as i32;
    let device = Device::new("phone-x", 31);
    let in_pane = factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(pid));
    let set_key = Request {
        method: "client.set_step_up_key".into(),
        payload: Some(request::Payload::ClientSetStepUpKey(pb::ClientSetStepUpKey { client_id: "phone-x".into(), key: Some(device.wire_key(ALG_DEVICE)) })),
        ..Default::default()
    };
    assert!(word(call(&in_pane, set_key.clone()).await).ends_with("person_only"), "a key from a pane");
    assert!(svc.store.step_up_key("phone-x").unwrap().is_none());
    let person = crate::test_support::Outside::a_person(dir.path(), 6592);
    let outside = factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    call(&outside, set_key).await.expect("a person at the socket");
    assert!(svc.store.step_up_key("phone-x").unwrap().unwrap().trusted, "the socket's key is trusted");
    let approve = Request {
        method: "client.approve_step_up".into(),
        payload: Some(request::Payload::ClientApproveStepUp(pb::ClientApproveStepUp { client_id: "phone-x".into(), code: code_of(ALG_DEVICE, &device.public()) })),
        ..Default::default()
    };
    assert!(word(call(&in_pane, approve).await).ends_with("person_only"), "an approval from a pane");
}

// MARK: - Who may step up

/// An Admin steps up for what they manage and no further; a Member's key,
/// however trusted, signs no admin action.
#[tokio::test]
async fn an_admin_manages_members_and_a_member_is_no_admin() {
    let r = runner().await;
    let dan = Device::new("phone-d", 21);
    r.trust(&dan, ALG_DEVICE);
    let conn = r.conn("phone-d");
    let n = nonce(&conn).await;
    let owners = word(call(&conn, dan.step(&r.pin, revoke_req("phone-o", false), &n)).await);
    assert!(owners.contains("not_yours"), "{owners}");
    assert!(lines(&r).contains(&"phone-o".to_string()));
    let n = nonce(&conn).await;
    call(&conn, dan.step(&r.pin, revoke_req("pad-b", false), &n)).await.expect("Bob's pad");

    let bob = Device::new("phone-b", 22);
    r.trust(&bob, ALG_DEVICE);
    let conn = r.conn("phone-b");
    let n = nonce(&conn).await;
    let member = word(call(&conn, bob.step(&r.pin, remove_req("carol"), &n)).await);
    assert!(member.contains("not_admin"), "{member}");
    assert!(r.svc.store.member("carol").is_ok());
}

/// B2: a remote device writes no shell line and no `host_admin` line, stepped
/// up or not; its stepped-up enroll of a `control` line trusts the new
/// device's key, and an unsigned one stores it as a proposal (B3).
#[tokio::test]
async fn a_remote_device_writes_no_shell_line_and_trusts_only_what_it_signed_for() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, ALG_DEVICE);
    let conn = r.conn("phone-o");
    let fresh = Device::new("new-o", 41);

    for (what, req) in [
        ("a shell line", enroll_req(50, "shell-o", Scope::HostAdmin, true, None)),
        ("a host_admin line", enroll_req(51, "admin-x", Scope::HostAdmin, false, None)),
    ] {
        let n = nonce(&conn).await;
        let signed = word(call(&conn, phone.step(&r.pin, req.clone(), &n)).await);
        assert!(signed.starts_with("scope-denied"), "{what} signed, from a control line: {signed}");
    }
    // An older Admin phone whose line is `host_admin` is refused too, signed
    // or not (the owner's, in the socket's words: `people_enrolled_tests`).
    let legacy = Device::new("phone-d", 21);
    r.trust(&legacy, ALG_DEVICE);
    let old = factory(&r.svc, Some("phone-d"), Scope::HostAdmin);
    for req in [enroll_req(55, "shell-d", Scope::HostAdmin, true, None), enroll_req(56, "admin-d", Scope::HostAdmin, false, None)] {
        let n = nonce(&old).await;
        let signed = word(call(&old, legacy.step(&r.pin, req, &n)).await);
        assert!(signed.starts_with("not-yours"), "{signed}");
    }
    assert!(!lines(&r).iter().any(|l| ["shell-o", "admin-x", "shell-d", "admin-d"].contains(&l.as_str())));
    // The socket still writes them.
    call(&r.socket(), enroll_req(52, "mac-o", Scope::HostAdmin, true, None)).await.expect("the socket's shell line");

    let n = nonce(&conn).await;
    call(&conn, phone.step(&r.pin, enroll_req(53, "new-o", Scope::Control, false, Some(fresh.wire_key(ALG_DEVICE))), &n)).await.expect("a stepped-up enroll");
    assert!(r.svc.store.step_up_key("new-o").unwrap().unwrap().trusted, "signed for, by the same person");

    // A Member's own enroll needs no step-up, and its key waits.
    let member = factory(&r.svc, Some("phone-b"), Scope::Control);
    let bobs = Device::new("new-b", 42);
    call(&member, enroll_req(54, "new-b", Scope::Control, false, Some(bobs.wire_key(ALG_DEVICE)))).await.expect("a member's own device");
    let row = r.svc.store.step_up_key("new-b").unwrap().unwrap();
    assert!(!row.trusted, "an un-stepped enroll proposes");
    let n = nonce(&member).await;
    let signed = word(call(&member, bobs.step(&r.pin, remove_req("carol"), &n)).await);
    assert!(signed.ends_with("key_untrusted"), "{signed}");
}

/// The runner's own socket takes no step-up.
#[tokio::test]
async fn the_socket_needs_and_takes_no_step_up() {
    let r = runner().await;
    let phone = Device::new("phone-o", 11);
    let socket = r.socket();
    let n = nonce(&socket).await;
    assert!(word(call(&socket, phone.step(&r.pin, revoke_req("pad-b", false), &n)).await).ends_with("step_up_unneeded"));
    call(&socket, revoke_req("pad-b", false)).await.expect("the socket revokes");
}

// MARK: - A browser's passkey

/// A passkey's assertion needs this app's origin and RP ID, a challenge that
/// is the statement's hash, and user presence and verification; a counter may
/// not go back.
#[tokio::test]
async fn a_passkey_assertion_must_be_this_apps_and_verified() {
    let r = runner().await;
    let web = Device::new("web-o", 15);
    r.trust(&web, ALG_WEBAUTHN);
    let conn = r.conn("web-o");
    let host = farcooler_fence::app_host();
    let origin = format!("https://{host}");

    for (what, flags, origin) in [
        ("no user verification", 0x01, origin.as_str()),
        ("no user presence", 0x04, origin.as_str()),
        ("another origin", UP_UV, "https://evil.example"),
    ] {
        let n = nonce(&conn).await;
        let refused = word(call(&conn, web.assert(&r.pin, revoke_req("pad-b", false), &n, flags, 1, origin)).await);
        assert!(refused.ends_with("step_up_failed"), "{what}: {refused}");
    }
    assert!(lines(&r).contains(&"pad-b".to_string()));

    let n = nonce(&conn).await;
    call(&conn, web.assert(&r.pin, revoke_req("pad-b", false), &n, UP_UV, 7, &origin)).await.expect("a verified assertion");
    // The counter went back, and stood still.
    for count in [3, 7] {
        let n = nonce(&conn).await;
        let cloned = word(call(&conn, web.assert(&r.pin, revoke_req("tv-c", false), &n, UP_UV, count, &origin)).await);
        assert!(cloned.ends_with("step_up_replayed"), "counter {count}: {cloned}");
    }
    assert!(lines(&r).contains(&"tv-c".to_string()));
}

/// Decision 2: a browser's passkey takes access away and adds none. It
/// revokes and removes; it enrolls, approves and proposes nothing, and its
/// refusal leaves everything where it was.
#[tokio::test]
async fn a_browser_attempts_only_defensive_actions() {
    let r = runner().await;
    let web = Device::new("web-o", 15);
    r.trust(&web, ALG_WEBAUTHN);
    let conn = r.conn("web-o");
    let origin = format!("https://{}", farcooler_fence::app_host());
    let before = lines(&r);
    let fresh = Device::new("evil-o", 43);
    let approve = Request {
        method: "client.approve_step_up".into(),
        payload: Some(request::Payload::ClientApproveStepUp(pb::ClientApproveStepUp { client_id: "ipad-o".into(), code: "000000".into() })),
        ..Default::default()
    };
    let mut count = 0;
    for (what, req) in [
        ("an enroll", enroll_req(60, "evil-o", Scope::Control, false, Some(fresh.wire_key(ALG_DEVICE)))),
        ("an approval", approve),
        ("a member's change", set("dan", &["pad-b"], "", "")),
    ] {
        count += 1;
        let n = nonce(&conn).await;
        let refused = word(call(&conn, web.assert(&r.pin, req, &n, UP_UV, count, &origin)).await);
        assert!(refused.ends_with("defensive_only"), "{what}: {refused}");
    }
    assert_eq!(lines(&r), before, "nothing was written");
    assert!(r.svc.store.step_up_key("evil-o").unwrap().is_none());

    // And it can't propose a key either.
    let propose = Request {
        method: "client.set_step_up_key".into(),
        payload: Some(request::Payload::ClientSetStepUpKey(pb::ClientSetStepUpKey { client_id: String::new(), key: Some(web.wire_key(ALG_WEBAUTHN)) })),
        ..Default::default()
    };
    let refused = word(call(&conn, propose).await);
    assert!(refused.ends_with("defensive_only") || refused.ends_with("key_exists"), "{refused}");

    // What it may do.
    let n = nonce(&conn).await;
    call(&conn, web.assert(&r.pin, remove_req("carol"), &n, UP_UV, 99, &origin)).await.expect("a browser removes a person");
    assert!(r.svc.store.member("carol").is_err());
}

/// Makes `id`'s line a browser's passkey line on a `host_admin` Key A line, as
/// `fence::render` writes it.
fn make_browser(r: &Runner, id: &str) {
    let blob = if farcooler_fence::app_host() == "app.farcooler.com" {
        "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAARYXBwLmZhcmNvb2xlci5jb20="
    } else {
        "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAAYYXBwLmNhbmFyeS5mYXJjb29sZXIuY29t"
    };
    let passkey = farcooler_fence::render(&format!("sk-ecdsa-sha2-nistp256@openssh.com {blob} webauthn@browser"), "Browser", id, Scope::HostAdmin, farcooler_fence::Grant::FarCooler, None).expect("a passkey line");
    let path = r.svc.authorized_keys();
    let kept: Vec<String> = farcooler_fence::read(path, farcooler_fence::AUTHORIZED_KEYS).unwrap().into_iter().map(|e| if e.client_id == id { passkey.clone() } else { e.line }).collect();
    farcooler_fence::write(path, farcooler_fence::AUTHORIZED_KEYS, &kept, &[], farcooler_fence::Placement::Last).unwrap();
    r.svc.store.set_device_cap(id, None).unwrap(); // the owner's line, held to nothing below `host_admin`
}

/// A browser has no key to ask for and can never register one, so it has no
/// transition: with no step-up it is refused every admin action that isn't
/// defensive, and allowed the ones that are. A phone with no key is not.
#[tokio::test]
async fn a_keyless_browser_may_only_take_access_away() {
    let r = runner().await;
    make_browser(&r, "web-o");
    // A `host_admin` line is where a keyless device does admin today.
    let conn = factory(&r.svc, Some("web-o"), Scope::HostAdmin);
    let before = lines(&r);
    let refused = word(call(&conn, enroll_req(61, "new-o", Scope::Control, false, None)).await);
    assert!(refused.ends_with("defensive_only"), "{refused}");
    let refused = word(call(&conn, set("dan", &["pad-b"], "", "")).await);
    assert!(refused.ends_with("defensive_only"), "{refused}");
    assert_eq!(lines(&r), before, "nothing was written");

    call(&conn, revoke_req("pad-b", false)).await.expect("a keyless browser revokes");
    assert!(!lines(&r).contains(&"pad-b".to_string()));
    call(&conn, remove_req("carol")).await.expect("a keyless browser removes a person");
}

/// A browser with a valid passkey step-up for the exact action is served.
#[tokio::test]
async fn a_browser_with_a_step_up_for_that_action_is_served() {
    let r = runner().await;
    make_browser(&r, "web-o");
    let web = Device::new("web-o", 15);
    r.trust(&web, ALG_WEBAUTHN);
    let conn = r.conn("web-o");
    let origin = format!("https://{}", farcooler_fence::app_host());
    let n = nonce(&conn).await;
    call(&conn, web.assert(&r.pin, revoke_req("pad-b", false), &n, UP_UV, 1, &origin)).await.expect("a stepped-up browser");
    assert!(!lines(&r).contains(&"pad-b".to_string()));
}

// MARK: - The verifiers alone

/// A challenge is 32 random bytes, and two are never alike.
#[test]
fn challenges_are_random() {
    let c = Challenges::default();
    let (a, b) = (c.issue(0).unwrap(), c.issue(0).unwrap());
    assert_ne!(a, b);
    assert_ne!(a, [0; 32]);
}

/// A key is a P-256 point in a known algorithm and nothing else.
#[test]
fn only_a_p256_point_in_a_known_algorithm_is_a_key() {
    let good = Device::new("x", 5).public();
    assert!(check_key(ALG_DEVICE, &good).is_ok());
    assert!(check_key(ALG_WEBAUTHN, &good).is_ok());
    assert!(check_key("rsa", &good).is_err());
    assert!(check_key(ALG_DEVICE, &good[..64]).is_err());
    assert!(check_key(ALG_DEVICE, &[4; 65]).is_err(), "not on the curve");
    assert_eq!(code_of(ALG_DEVICE, &good).len(), 6);
    assert_ne!(code_of(ALG_DEVICE, &good), code_of(ALG_WEBAUTHN, &good), "the algorithm is part of the key");
}

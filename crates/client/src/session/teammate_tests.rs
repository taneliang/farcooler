//! Add Teammate from a phone (ov-854), against a scripted runner that records
//! what it is sent, and against the daemon itself.
//!
//! The scripted runner is for the call site: what the phone asks for, in what
//! order, and what it does when a step fails. The daemon is for what that
//! leaves behind: whose the device is, and what removing the member does.

use std::sync::{Arc, Mutex};

use farcooler_protocol::v1::{self as pb, Scope, request, response, result, wire_envelope};
use farcooler_transport::codec::{FrameReader, FrameWriter};
use serde_json::json;

use super::*;
use crate::ffi::phone_path_tests::a_runner;

/// A real ed25519 key, as a phone's offer carries one.
const BOBS_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 bob-phone";

fn bobs_phone() -> TeammateDevice<'static> {
    TeammateDevice { public_key: BOBS_KEY, label: "Bob’s iPhone", client_id: "phone-bob", scope: "control", node_key: "" }
}

fn bob() -> TeammateMember<'static> {
    TeammateMember { handle: "bob", name: "Bob Smith", account: "user_bob", email: "bob@example.com" }
}

/// What a scripted runner answers one request with.
type Script = Box<dyn Fn(&pb::Request) -> response::Outcome + Send + Sync>;

struct Scripted {
    socket: std::path::PathBuf,
    sent: Arc<Mutex<Vec<pb::Request>>>,
    _dir: tempfile::TempDir,
}

impl Scripted {
    fn methods(&self) -> Vec<String> {
        self.sent.lock().unwrap().iter().map(|r| r.method.clone()).collect()
    }

    fn payload(&self, method: &str) -> Option<request::Payload> {
        self.sent.lock().unwrap().iter().find(|r| r.method == method).and_then(|r| r.payload.clone())
    }

    async fn session(&self) -> Session {
        Session::connect_local(&self.socket).await.expect("connect")
    }
}

fn ok(value: result::Value) -> response::Outcome {
    response::Outcome::Result(pb::Result { value: Some(value) })
}

fn refused(word: pb::ErrorCode, what: &str) -> response::Outcome {
    response::Outcome::Error(pb::Error { code: word as i32, what: what.into(), message: "refused".into(), ..Default::default() })
}

fn enrolled(already: bool) -> response::Outcome {
    ok(result::Value::ClientEnroll(pb::ClientEnrollResult {
        client: Some(pb::EnrolledClient { client_id: "phone-bob".into(), ..Default::default() }),
        already_enrolled: already,
        conn_blob: "token".into(),
    }))
}

fn people(members: Vec<pb::Member>) -> response::Outcome {
    ok(result::Value::MemberList(pb::MemberList { members, ..Default::default() }))
}

fn member(handle: &str, account: &str, devices: &[&str]) -> pb::Member {
    pb::Member {
        handle: handle.into(),
        name: handle.into(),
        account: account.into(),
        client_ids: devices.iter().map(|d| d.to_string()).collect(),
        ..Default::default()
    }
}

/// A runner at `scope` that answers each request by `script`, in its own
/// scratch directory, and keeps what it was sent.
async fn a_scripted_runner(scope: Scope, capabilities: Vec<String>, script: Script) -> Scripted {
    let dir = tempfile::Builder::new().prefix("t").tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("s.sock");
    let sent = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let (record, script) = (sent.clone(), Arc::new(script));
    tokio::spawn(async move {
        let envelope = |body| pb::WireEnvelope {
            protocol_version: farcooler_protocol::PROTOCOL_VERSION,
            message_id: farcooler_protocol::ids::new_id(),
            body: Some(body),
        };
        while let Ok((stream, _)) = listener.accept().await {
            let (read, write) = stream.into_split();
            let (mut reader, mut writer) = (FrameReader::new(read), FrameWriter::new(write));
            let Ok(Some(_hello)) = reader.read_frame().await else { continue };
            let hello = envelope(wire_envelope::Body::ServerHello(pb::ServerHello {
                selected_protocol_version: farcooler_protocol::PROTOCOL_VERSION,
                daemon_version: "scripted".into(),
                max_control_envelope_bytes: farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES as u32,
                max_terminal_payload_bytes: farcooler_protocol::MAX_TERMINAL_PAYLOAD_BYTES as u32,
                granted_scope: scope as i32,
                capabilities: capabilities.clone(),
            }));
            if writer.write_frame(&hello).await.is_err() {
                continue;
            }
            while let Ok(Some(frame)) = reader.read_frame().await {
                let Some(wire_envelope::Body::Request(req)) = frame.body else { continue };
                let outcome = script(&req);
                record.lock().unwrap().push(req.clone());
                let reply = envelope(wire_envelope::Body::Response(pb::Response { request_id: req.request_id, outcome: Some(outcome) }));
                if writer.write_frame(&reply).await.is_err() {
                    break;
                }
            }
        }
    });
    Scripted { socket, sent, _dir: dir }
}

fn every_capability() -> Vec<String> {
    farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect()
}

/// A runner at `host_admin` that takes everything, with `members` on it.
async fn a_runner_that_takes_it(members: Vec<pb::Member>) -> Scripted {
    a_scripted_runner(
        Scope::HostAdmin,
        every_capability(),
        Box::new(move |r| match r.method.as_str() {
            "member.list" => people(members.clone()),
            "client.enroll" => enrolled(false),
            "member.set" => people(vec![]),
            "client.revoke" => ok(result::Value::ClientList(pb::ClientList::default())),
            other => panic!("unexpected {other}"),
        }),
    )
    .await
}

/// **The call site.** The phone writes the key `for_teammate`, at the scope
/// chosen, never a shell line, and only then binds it to the member, with what
/// it read. Goes red on `for_teammate: false`, on a `true` shell flag, on a
/// bind before the write, and on a bind that sends no `devices_read`.
#[tokio::test]
async fn a_teammate_is_written_for_them_and_then_bound() {
    let runner = a_runner_that_takes_it(vec![]).await;
    let session = runner.session().await;

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;

    assert_eq!(outcome, TeammateOutcome::Written { tunnel: "token".into() });
    assert_eq!(runner.methods(), ["member.list", "client.enroll", "member.set"]);
    let Some(request::Payload::ClientEnroll(write)) = runner.payload("client.enroll") else { panic!("no enroll") };
    assert!(write.for_teammate, "the runner would keep the key the owner's until bound");
    assert!(!write.shell_access, "a teammate never has a shell line");
    assert_eq!((write.scope, write.client_id.as_str()), (Scope::Control as i32, "phone-bob"));
    let Some(request::Payload::MemberSet(bind)) = runner.payload("member.set") else { panic!("no bind") };
    assert_eq!((bind.handle.as_str(), bind.name.as_str()), ("bob", "Bob Smith"));
    assert_eq!(bind.client_ids, ["phone-bob"]);
    assert_eq!((bind.account.as_str(), bind.email.as_str()), ("user_bob", "bob@example.com"));
    assert_eq!(bind.devices_read.map(|d| d.client_ids), Some(vec![]), "a bind says what it read");
}

/// The teammate's access is what was chosen: Read stays Read.
#[tokio::test]
async fn a_read_teammate_is_written_at_read() {
    let runner = a_runner_that_takes_it(vec![]).await;
    let session = runner.session().await;
    let device = TeammateDevice { scope: "read", ..bobs_phone() };

    session.enroll_teammate(&device, &bob()).await;

    let Some(request::Payload::ClientEnroll(write)) = runner.payload("client.enroll") else { panic!("no enroll") };
    assert_eq!(write.scope, Scope::Read as i32);
}

/// **A phone below `host_admin` writes nothing.** It could write the key and
/// not bind it, or write it and not take it off, so it is told before any
/// request is made. Goes red without the gate.
#[tokio::test]
async fn a_control_phone_is_told_it_cant_and_sends_nothing() {
    let runner = a_scripted_runner(Scope::Control, every_capability(), Box::new(|r| panic!("sent {}", r.method))).await;
    let session = runner.session().await;

    assert_eq!(session.enroll_teammate(&bobs_phone(), &bob()).await, TeammateOutcome::NeedsAdmin);
    assert!(runner.methods().is_empty(), "{:?}", runner.methods());
}

/// A runner that can't keep a teammate's account is refused before a key is
/// written: the bind would drop the account, and the key would be unbound.
#[tokio::test]
async fn a_runner_that_cant_keep_the_account_gets_no_key() {
    let without: Vec<String> =
        every_capability().into_iter().filter(|c| c != farcooler_protocol::capability::MEMBER_ACCOUNTS).collect();
    let runner = a_scripted_runner(Scope::HostAdmin, without, Box::new(|r| panic!("sent {}", r.method))).await;
    let session = runner.session().await;

    assert_eq!(session.enroll_teammate(&bobs_phone(), &bob()).await, TeammateOutcome::TooOld);
    assert!(runner.methods().is_empty());
}

/// **A bind the runner refuses takes the key off again**, and the devices it
/// added with it. Goes red when the take-off is skipped or drops `with_added`.
#[tokio::test]
async fn a_bind_that_fails_takes_the_key_off_again() {
    let runner = a_scripted_runner(
        Scope::HostAdmin,
        every_capability(),
        Box::new(|r| match r.method.as_str() {
            "member.list" => people(vec![]),
            "client.enroll" => enrolled(false),
            "member.set" => refused(pb::ErrorCode::InvalidArgument, "handle"),
            "client.revoke" => ok(result::Value::ClientList(pb::ClientList::default())),
            other => panic!("unexpected {other}"),
        }),
    )
    .await;
    let session = runner.session().await;

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;

    assert_eq!(outcome, TeammateOutcome::NotRecorded { still_there: false });
    assert_eq!(runner.methods(), ["member.list", "client.enroll", "member.set", "client.revoke"]);
    let Some(request::Payload::ClientRevoke(off)) = runner.payload("client.revoke") else { panic!("no revoke") };
    assert_eq!(off.client_id, "phone-bob");
    assert!(off.with_added, "what the key added goes with it");
}

/// A take-off that fails too is said, never swallowed.
#[tokio::test]
async fn a_key_that_could_not_be_taken_off_is_said_to_be_still_there() {
    let runner = a_scripted_runner(
        Scope::HostAdmin,
        every_capability(),
        Box::new(|r| match r.method.as_str() {
            "member.list" => people(vec![]),
            "client.enroll" => refused(pb::ErrorCode::OperationFailed, ""),
            "client.revoke" => refused(pb::ErrorCode::OperationFailed, ""),
            other => panic!("unexpected {other}"),
        }),
    )
    .await;
    let session = runner.session().await;

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;

    assert_eq!(outcome, TeammateOutcome::NotAdded { still_there: true });
}

/// A device that is another member's there is left exactly as it was: nothing
/// is written and nothing is taken off.
#[tokio::test]
async fn a_device_that_is_someone_elses_is_left_alone() {
    let runner = a_runner_that_takes_it(vec![member("carol", "user_carol", &["phone-bob"])]).await;
    let session = runner.session().await;

    assert_eq!(session.enroll_teammate(&bobs_phone(), &bob()).await, TeammateOutcome::SomeoneElses);
    assert_eq!(runner.methods(), ["member.list"]);
}

/// A teammate already on the runner (by account) keeps their handle, name and
/// other devices, and gains this one: `member.set` replaces a member's
/// devices, so the list sent is the old devices and the new.
#[tokio::test]
async fn a_teammate_already_there_gains_the_device_and_keeps_the_rest() {
    let runner = a_runner_that_takes_it(vec![member("bobby", "user_bob", &["mac-bob"])]).await;
    let session = runner.session().await;

    session.enroll_teammate(&bobs_phone(), &bob()).await;

    let Some(request::Payload::MemberSet(bind)) = runner.payload("member.set") else { panic!("no bind") };
    assert_eq!(bind.handle, "bobby");
    assert_eq!(bind.client_ids, ["mac-bob", "phone-bob"]);
    assert_eq!(bind.devices_read.map(|d| d.client_ids), Some(vec!["mac-bob".to_string()]));
}

/// A new teammate whose suggested handle is taken is suffixed; the runner's
/// own list decides.
#[tokio::test]
async fn a_taken_handle_is_suffixed() {
    let runner = a_runner_that_takes_it(vec![member("bob", "user_other", &[])]).await;
    let session = runner.session().await;

    session.enroll_teammate(&bobs_phone(), &bob()).await;

    let Some(request::Payload::MemberSet(bind)) = runner.payload("member.set") else { panic!("no bind") };
    assert_eq!(bind.handle, "bob-2");
}

#[test]
fn a_handle_is_made_unique_within_the_runners_length_and_never_reserved() {
    assert_eq!(unique_handle("bob", ["alice"]), "bob");
    assert_eq!(unique_handle("bob", ["bob", "bob-2"]), "bob-3");
    assert_eq!(unique_handle("owner", []), "owner-2");
    let long = "a".repeat(32);
    let made = unique_handle(&long, [long.as_str()]);
    assert_eq!(made.len(), 32);
    assert!(made.ends_with("-2"));
}

/// A key already there keeps its lines, which may be wider than what was
/// chosen, so it is taken off (both lines) and written again.
#[tokio::test]
async fn a_key_already_there_is_taken_off_and_written_again() {
    let writes = Arc::new(Mutex::new(0));
    let counted = writes.clone();
    let runner = a_scripted_runner(
        Scope::HostAdmin,
        every_capability(),
        Box::new(move |r| match r.method.as_str() {
            "member.list" => people(vec![]),
            "client.enroll" => {
                let mut n = counted.lock().unwrap();
                *n += 1;
                enrolled(*n == 1)
            }
            "client.revoke" => ok(result::Value::ClientList(pb::ClientList::default())),
            "member.set" => people(vec![]),
            other => panic!("unexpected {other}"),
        }),
    )
    .await;
    let session = runner.session().await;

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;

    assert!(matches!(outcome, TeammateOutcome::Written { .. }));
    assert_eq!(runner.methods(), ["member.list", "client.enroll", "client.revoke", "client.enroll", "member.set"]);
}

/// Their devices changed between the read and the bind (ov-786): read again
/// and bind once more from what is there now.
#[tokio::test]
async fn a_bind_over_a_stale_read_is_read_again_and_retried() {
    let binds = Arc::new(Mutex::new(0));
    let counted = binds.clone();
    let runner = a_scripted_runner(
        Scope::HostAdmin,
        every_capability(),
        Box::new(move |r| match r.method.as_str() {
            "member.list" => people(vec![]),
            "client.enroll" => enrolled(false),
            "member.set" => {
                let mut n = counted.lock().unwrap();
                *n += 1;
                if *n == 1 { refused(pb::ErrorCode::ResourceConflict, "devices_changed") } else { people(vec![]) }
            }
            other => panic!("unexpected {other}"),
        }),
    )
    .await;
    let session = runner.session().await;

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;

    assert!(matches!(outcome, TeammateOutcome::Written { .. }));
    assert_eq!(runner.methods(), ["member.list", "client.enroll", "member.set", "member.list", "member.set"]);
}

/// **The daemon's side of it.** On a `host_admin` connection the key lands in
/// the runner's file, the device is the member's (their rep, their scope) and
/// not released, and removing the member takes the key off the file. Goes red
/// when the bind is skipped (the device is nobody's) or the key is written for
/// the owner.
#[tokio::test]
async fn the_bound_device_is_the_members_and_removing_them_removes_it() {
    let runner = a_runner(Scope::HostAdmin).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let outcome = session.enroll_teammate(&bobs_phone(), &bob()).await;
    assert!(matches!(outcome, TeammateOutcome::Written { .. }), "{outcome:?}");

    let store = &runner.service.store;
    assert_eq!(store.member_of_device("phone-bob").unwrap().as_deref(), Some("bob"), "bound to the member");
    assert!(!store.device_released("phone-bob").unwrap(), "bound, so no longer held");
    let bound = store.member("bob").unwrap();
    assert_eq!((bound.account.as_deref(), bound.email.as_deref(), bound.name.as_str()), (Some("user_bob"), Some("bob@example.com"), "Bob Smith"));
    let peer = farcooler_transport::Peer { client_id: Some("phone-bob".into()), scope: Scope::Control };
    assert_eq!(
        farcooler_daemon::people::person_of(store, &peer).unwrap(),
        farcooler_daemon::people::Person::Member("bob".into()),
        "the device runs as the member"
    );
    let listed = session.enrolled_clients().await.unwrap();
    let line = listed["clients"].as_array().unwrap().iter().find(|c| c["clientId"] == "phone-bob").expect("on the runner").clone();
    assert_eq!(line["scope"], "control", "{line}");
    assert_eq!(line["shellAccess"], false, "{line}");

    let payload = request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() });
    session.value("member.remove", None, Some(payload)).await.expect("member.remove");
    let after = session.enrolled_clients().await.unwrap();
    assert!(
        !after["clients"].as_array().unwrap().iter().any(|c| c["clientId"] == "phone-bob"),
        "removing the member removes the device: {after}"
    );
    assert!(!std::fs::read_to_string(&runner.authorized_keys).unwrap().contains("AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4"));
}

/// On the daemon, a `control` phone is refused before anything is written, so
/// nothing is left on the runner.
#[tokio::test]
async fn a_control_phone_leaves_the_real_runner_untouched() {
    let runner = a_runner(Scope::Control).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    assert_eq!(session.enroll_teammate(&bobs_phone(), &bob()).await, TeammateOutcome::NeedsAdmin);

    let listed = session.enrolled_clients().await.unwrap();
    assert!(listed["clients"].as_array().unwrap().is_empty(), "{listed}");
    assert_eq!(runner.service.store.member_of_device("phone-bob").unwrap(), None);
}

#[test]
fn the_outcomes_cross_the_ffi_as_words() {
    assert_eq!(TeammateOutcome::Written { tunnel: "t".into() }.json(), json!({ "outcome": "written", "tunnel": "t" }));
    assert_eq!(TeammateOutcome::NotAdded { still_there: true }.json(), json!({ "outcome": "not_added", "stillThere": true }));
    assert_eq!(TeammateOutcome::NeedsAdmin.json(), json!({ "outcome": "needs_admin" }));
}

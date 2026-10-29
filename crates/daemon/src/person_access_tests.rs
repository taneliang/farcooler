//! Access follows the person, enforced live (ov-856, slice 2): insert-only
//! binds, the rows a revoke clears, `access_generation` rechecked under the
//! lock, scope-word rebuilds in the order their direction asks, admin
//! requests refused from a pane, and the two calls only the runner's socket
//! makes. Each race is parked at a hook in the blocking closure, so the
//! order is the one the test says and nothing sleeps.

use std::collections::BTreeMap;
use std::sync::mpsc;

use farcooler_core::DomainError;
use farcooler_protocol::v1::{self as pb, Request, Scope, request};
use farcooler_store::access::{Access, Cap};

use super::*;
use crate::rpc_people::tests::{call, factory, set};

fn key(n: u8) -> String {
    use ssh_key::public::{Ed25519PublicKey, KeyData};
    ssh_key::PublicKey::new(KeyData::Ed25519(Ed25519PublicKey([n; 32])), "x").to_openssh().unwrap()
}

const NODE_A: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const NODE_E: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";

fn asking(n: u8, id: &str, scope: Scope, node: &str) -> pb::ClientEnroll {
    pb::ClientEnroll {
        public_key: key(n),
        label: id.into(),
        client_id: id.into(),
        scope: scope as i32,
        shell_access: false,
        node_key: node.into(),
        for_teammate: false,
        step_up_key: None,
    }
}

/// The runner's own socket enrolls `id`.
async fn enroll(svc: &Service, n: u8, id: &str, scope: Scope) {
    crate::enrollment::enroll(svc, &asking(n, id, scope, "")).await.unwrap_or_else(|e| panic!("{id}: {e:?}"));
}

struct Runner {
    dir: tempfile::TempDir,
    svc: Arc<Service>,
}

impl Runner {
    fn ssh(&self) -> std::path::PathBuf {
        self.dir.path().join("home").join(".ssh")
    }

    fn lines(&self) -> BTreeMap<String, Scope> {
        entries(&self.svc).into_iter().filter(|e| !e.shell_access).map(|e| (e.client_id, e.scope)).collect()
    }

    fn file(&self) -> Vec<u8> {
        std::fs::read(self.svc.authorized_keys()).unwrap()
    }

    fn owner(&self) -> crate::rpc::RpcFactory {
        factory(&self.svc, None, Scope::HostAdmin)
    }

    fn lock_the_directory(&self, locked: bool) {
        let mode = if locked { 0o500 } else { 0o700 };
        std::fs::set_permissions(self.ssh(), std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
    }
}

fn entries(svc: &Service) -> Vec<farcooler_fence::Entry> {
    farcooler_fence::read(svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS).unwrap()
}

/// A migrated runner shaped like the real ones:
///
/// - the owner has a control phone, a read-only iPad and an admin Mac;
/// - Bob is a Member: a control phone, and a read pad;
/// - Carol is a Viewer, with a read display;
/// - Dan and Fay are Admins, whose phones are `host_admin`.
async fn runner() -> Runner {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    // Nothing yet to move: this marks the migration done, as a start does.
    crate::access_migration::migrate_at_start(&svc).await;
    for (n, id, scope) in [
        (1, "phone-o", Scope::Control),
        (2, "ipad-o", Scope::Read),
        (3, "phone-b", Scope::Control),
        (4, "pad-b", Scope::Read),
        (5, "tv-c", Scope::Read),
        (6, "phone-d", Scope::HostAdmin),
        (7, "phone-f", Scope::HostAdmin),
        (8, "admin-o", Scope::HostAdmin),
    ] {
        enroll(&svc, n, id, scope).await;
    }
    let runner = Runner { dir, svc };
    for (handle, devices) in [("bob", &["phone-b", "pad-b"][..]), ("carol", &["tv-c"]), ("dan", &["phone-d"]), ("fay", &["phone-f"])] {
        call(&runner.owner(), set(handle, devices, "", "")).await.unwrap_or_else(|e| panic!("{handle}: {e}"));
    }
    runner
}

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

fn revoke_req(device: &str, with_added: bool) -> Request {
    Request {
        method: "client.revoke".into(),
        payload: Some(request::Payload::ClientRevoke(pb::ClientRevoke { client_id: device.into(), with_added })),
        ..Default::default()
    }
}

fn enroll_req(n: u8, id: &str, scope: Scope) -> Request {
    Request { method: "client.enroll".into(), payload: Some(request::Payload::ClientEnroll(asking(n, id, scope, ""))), ..Default::default() }
}

use Access::{Admin, Member, Viewer};

/// A socket enroll binds its line to the owner in the lock that writes it,
/// capped where it is below the owner's ceiling, and a member's binding
/// takes the owner's away.
#[tokio::test]
async fn an_enroll_binds_the_new_line_to_the_person_it_belongs_to() {
    let r = runner().await;
    let stored = r.svc.store.stored_access();
    let stored = stored.unwrap();
    let owners: Vec<&str> = stored.owner_devices.iter().map(|(d, _)| d.as_str()).collect();
    assert_eq!(owners, ["admin-o", "ipad-o", "phone-o"], "the owner's own, and not the devices bound to Bob, Carol, Dan or Fay since");
    assert_eq!(stored.member_access, vec![("bob".into(), Member), ("carol".into(), Viewer), ("dan".into(), Admin), ("fay".into(), Admin)]);
    let caps: Vec<(&str, Cap)> = stored.caps.iter().map(|(d, c)| (d.as_str(), *c)).collect();
    assert_eq!(
        caps,
        [("ipad-o", Cap::Viewer), ("pad-b", Cap::Viewer), ("phone-o", Cap::Member)],
        "a cap wherever a line is below its person's ceiling"
    );
}

/// S5: a bind is insert-only. An id with a row of any kind left is refused
/// `device_taken`, even from the socket, and nothing is written; once a
/// revoke has cleared the rows it owns, the same id enrolls. An older mark (a
/// member's binding, a release) stays after a revoke and keeps the id taken.
#[tokio::test]
async fn a_new_line_under_an_id_with_a_row_left_is_refused_even_from_the_socket() {
    let r = runner().await;
    let store = &r.svc.store;
    store.set_device_cap("left-cap", Some(Cap::Viewer)).unwrap();
    store.record_teammate_pending("left-pending").unwrap();
    store.bind_owner_device("left-owner", None).unwrap();
    store.set_member("zed", "Zed", &["left-bound".into()]).unwrap();
    for (n, id, clears) in [(20, "left-cap", true), (21, "left-pending", false), (22, "left-owner", true), (23, "left-bound", false)] {
        let before = r.file();
        let refused = crate::enrollment::enroll(&r.svc, &asking(n, id, Scope::Control, "")).await;
        assert!(matches!(refused, Err(DomainError::Conflict { what: "device_taken" })), "{id}: {refused:?}");
        assert_eq!(r.file(), before, "{id}: a line was written");
        store.clear_device_rows(id).unwrap();
        let again = crate::enrollment::enroll(&r.svc, &asking(n, id, Scope::Control, "")).await;
        assert_eq!(again.is_ok(), clears, "{id} after a revoke's rows were cleared: {again:?}");
    }
}

/// A member's enroll replaces the older marks itself, as it did, so a
/// teammate's id revoked and enrolled again is Tess's no longer; the tables
/// this slice added are left over for it as for anyone.
#[tokio::test]
async fn a_members_enroll_supersedes_the_older_marks_and_not_the_new_rows() {
    let r = runner().await;
    let bob = factory(&r.svc, Some("phone-d"), Scope::HostAdmin);
    call(&bob, enroll_req(80, "mate-t", Scope::Control)).await.unwrap();
    call(&r.owner(), set("tess", &["mate-t"], "", "")).await.unwrap();
    call(&r.owner(), revoke_req("mate-t", false)).await.unwrap();
    call(&bob, enroll_req(80, "mate-t", Scope::Control)).await.expect("the member's own enroll takes the id back");
    assert!(r.svc.store.member_of_device("mate-t").unwrap().is_none(), "and it is nobody's until bound");
    r.svc.store.set_device_cap("mate-u", Some(Cap::Viewer)).unwrap();
    let refused = call(&bob, enroll_req(81, "mate-u", Scope::Control)).await;
    assert_eq!(refused.err().as_deref(), Some("resource-conflict:device_taken"), "a cap row left over");
}

/// S5: a revoke clears the rows it owns for the id in the lock that removes
/// the line (the owner's binding, the cap, a key, a pending mark), so an id
/// enrolled again inherits none of them. A member's binding is an older mark
/// and stays until their list is edited, so a revoked member's id is not
/// enrolled again under someone else.
#[tokio::test]
async fn a_revoke_clears_the_rows_it_owns_for_the_device() {
    let r = runner().await;
    r.svc.store.set_device_cap("ipad-o", Some(Cap::Viewer)).unwrap();
    call(&r.owner(), revoke_req("ipad-o", false)).await.unwrap();
    assert!(!r.svc.store.device_has_rows("ipad-o").unwrap(), "a row about the iPad stayed");
    assert!(!r.lines().contains_key("ipad-o"));
    call(&r.owner(), enroll_req(2, "ipad-o", Scope::Read)).await.unwrap();
    let stored = r.svc.store.stored_access().unwrap();
    assert!(stored.owner_devices.iter().any(|(d, _)| d == "ipad-o"), "enrolled again, it is the owner's: {stored:?}");
    assert_eq!(stored.caps.iter().filter(|(d, _)| d == "ipad-o").count(), 1, "one cap, the new line's");

    call(&r.owner(), revoke_req("phone-b", false)).await.unwrap();
    assert!(!r.lines().contains_key("phone-b"));
    assert_eq!(r.svc.store.member_of_device("phone-b").unwrap().as_deref(), Some("bob"), "Bob's list keeps it, as it did");
    let again = call(&r.owner(), enroll_req(3, "phone-b", Scope::Control)).await;
    assert_eq!(again.err().as_deref(), Some("resource-conflict:device_taken"), "a revoked member's id enrolled again");
}

/// A revoke whose file write fails clears nothing: a line with no rows would
/// read as the owner's.
#[tokio::test]
async fn a_revoke_that_cannot_write_keeps_the_rows() {
    let r = runner().await;
    r.lock_the_directory(true);
    let refused = call(&r.owner(), revoke_req("ipad-o", false)).await;
    r.lock_the_directory(false);
    assert!(refused.is_err(), "the revoke wrote into a locked directory");
    assert!(r.svc.store.device_has_rows("ipad-o").unwrap(), "the rows went with a line that stayed");
    assert!(r.lines().contains_key("ipad-o"));
}

/// A demotion rewrites the lines of all their devices to the new ceiling
/// before it is recorded, rewrites only the scope word of those and no other
/// line, and closes their sessions without a farewell.
#[tokio::test]
async fn a_demotion_lowers_their_lines_and_reconnects_their_sessions() {
    let r = runner().await;
    let before = entries(&r.svc);
    let phone = r.svc.sessions().open(Some("phone-b".into()));
    let pad = r.svc.sessions().open(Some("pad-b".into()));
    let other = r.svc.sessions().open(Some("tv-c".into()));
    call(&r.owner(), set_access_req("bob", "viewer")).await.unwrap();

    assert_eq!(r.lines()["phone-b"], Scope::Read, "Bob's control phone is a viewer's now");
    assert_eq!(r.lines()["pad-b"], Scope::Read);
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Viewer));
    for (session, closed) in [(&phone, true), (&pad, true), (&other, false)] {
        assert_eq!(session.is_closed(), closed);
        assert_eq!(session.closed_quietly(), closed, "closed for an access change, not as a revoke");
    }
    // Only the scope word: every other line is byte for byte what it was, and
    // Bob's lines differ in that word alone.
    let now = entries(&r.svc);
    for (was, is) in before.iter().zip(&now) {
        match was.client_id.as_str() {
            "phone-b" => assert_eq!(is.line, was.line.replace("--scope control", "--scope read")),
            _ => assert_eq!(is.line, was.line, "{} changed", was.client_id),
        }
    }
    assert_eq!(r.svc.store.access_generation("bob").unwrap(), 1);
}

/// A promotion raises the lines of the person it names, as far as `control`
/// and no further, and never a cap or another person's line (B1).
#[tokio::test]
async fn a_promotion_raises_only_the_named_person_and_only_to_control() {
    let r = runner().await;
    call(&r.owner(), set_access_req("carol", "member")).await.unwrap();
    let lines = r.lines();
    assert_eq!(lines["tv-c"], Scope::Control, "Carol's display is a member's now");
    assert_eq!(lines["pad-b"], Scope::Read, "Bob's capped pad stays read");
    assert_eq!(lines["ipad-o"], Scope::Read, "the owner's read-only iPad stays read");
    call(&r.owner(), set_access_req("bob", "admin")).await.unwrap();
    let lines = r.lines();
    assert_eq!(lines["phone-b"], Scope::Control, "an Admin's line is raised no further than control until step-up");
    assert_eq!(lines["pad-b"], Scope::Read);
    assert_eq!(lines["phone-d"], Scope::HostAdmin, "Dan's line stayed as it was");
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Admin));
}

/// A cap lowers the line first and a cleared one raises it only for that
/// device: the same direction rule, on a device.
#[tokio::test]
async fn a_cap_lowers_the_line_and_clearing_it_raises_that_device_alone() {
    let r = runner().await;
    call(&r.owner(), set_cap_req("phone-o", "viewer")).await.unwrap();
    assert_eq!(r.lines()["phone-o"], Scope::Read);
    assert_eq!(r.svc.store.device_cap("phone-o").unwrap(), Some(Cap::Viewer));
    call(&r.owner(), set_cap_req("phone-o", "")).await.unwrap();
    assert_eq!(r.lines()["phone-o"], Scope::Control, "uncapped, it rises to the control it had, not to host_admin");
    assert_eq!(r.lines()["ipad-o"], Scope::Read, "no other device rose");
    assert!(call(&r.owner(), set_cap_req("nobody", "viewer")).await.is_err(), "a device nobody has");
    assert!(call(&r.owner(), set_cap_req("phone-o", "owner")).await.is_err());
}

/// S3: a demotion whose lines can't be written changes nothing, so no
/// rollback can serve a stale, higher line; a promotion is recorded first, so
/// the device keeps its lower line.
#[tokio::test]
async fn a_demotion_that_cannot_be_written_changes_nothing_and_a_promotion_is_recorded_first() {
    let r = runner().await;
    r.lock_the_directory(true);
    let demote = call(&r.owner(), set_access_req("bob", "viewer")).await;
    let cap = call(&r.owner(), set_cap_req("phone-o", "viewer")).await;
    let promote = call(&r.owner(), set_access_req("carol", "member")).await;
    r.lock_the_directory(false);
    assert!(demote.is_err() && cap.is_err() && promote.is_err(), "a write into a locked directory succeeded");
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Member), "a demotion nobody could write was recorded");
    assert_eq!(r.svc.store.device_cap("phone-o").unwrap(), Some(Cap::Member));
    assert_eq!(r.svc.store.access_generation("bob").unwrap(), 0);
    assert_eq!(r.svc.store.member_access("carol").unwrap(), Some(Member), "a promotion is recorded before its lines");
    assert_eq!(r.lines()["tv-c"], Scope::Read, "and the line stays the lower one");
}

/// Each request is checked against the store, not the line: a demoted
/// Bob's open connection, whose line is still `control`, is refused at once.
#[tokio::test]
async fn a_request_is_served_at_the_lowest_of_its_line_its_person_and_its_cap() {
    let r = runner().await;
    let ask = || Request { method: "terminal.create".into(), ..Default::default() };
    let bob = || factory(&r.svc, Some("phone-b"), Scope::Control);
    let denied = |outcome: std::result::Result<pb::result::Value, String>| outcome.err().is_some_and(|e| e.starts_with("scope-denied"));
    assert!(!denied(call(&bob(), ask()).await), "a Member's control phone is served at control");
    r.svc.store.set_member_access("bob", Viewer).unwrap();
    assert!(denied(call(&bob(), ask()).await), "Bob was made a Viewer, and his open connection still holds a control line");
    r.svc.store.set_member_access("bob", Member).unwrap();
    r.svc.store.set_device_cap("phone-b", Some(Cap::Viewer)).unwrap();
    assert!(denied(call(&bob(), ask()).await), "his phone was capped");
    let peer = Peer { client_id: Some("phone-d".into()), scope: Scope::HostAdmin };
    assert_eq!(effective_scope(&r.svc.store, &peer), Scope::HostAdmin, "an Admin's host_admin line");
    let owners = Peer { client_id: Some("phone-o".into()), scope: Scope::HostAdmin };
    assert_eq!(effective_scope(&r.svc.store, &owners), Scope::Control, "the owner's phone is capped at member");
    r.svc.store.set_member_access("dan", Member).unwrap();
    assert_eq!(effective_scope(&r.svc.store, &peer), Scope::Control, "a Member's ceiling");
    let unknown = Peer { client_id: Some("phone-x".into()), scope: Scope::Read };
    assert_eq!(effective_scope(&r.svc.store, &unknown), Scope::Read, "a device the tables know nothing of is its line");
    let socket = Peer { client_id: None, scope: Scope::HostAdmin };
    assert_eq!(effective_scope(&r.svc.store, &socket), Scope::HostAdmin);
}

/// S4: a remote device never sets access or a cap, whatever its scope, and
/// nor does a caller the runner can't tell is a person.
#[tokio::test]
async fn access_and_caps_are_set_from_the_runner_socket_alone() {
    let r = runner().await;
    // The owner's own phone passes the gate and is refused here; an Admin's
    // is the owner's alone by the gate.
    for (device, word) in [("phone-d", "not-yours:not_yours"), ("admin-o", "not-yours:socket_only")] {
        let remote = factory(&r.svc, Some(device), Scope::HostAdmin);
        for req in [set_access_req("bob", "viewer"), set_cap_req("phone-b", "viewer")] {
            assert_eq!(call(&remote, req).await.err().as_deref(), Some(word), "{device}");
        }
    }
    let unknown = factory(&r.svc, None, Scope::HostAdmin).with_caller_pid(Some(0));
    for req in [set_access_req("bob", "viewer"), set_cap_req("phone-b", "viewer")] {
        assert_eq!(call(&unknown, req).await.err().as_deref(), Some("resource-conflict:person_only"));
    }
    assert_eq!(r.svc.store.member_access("bob").unwrap(), Some(Member));
    assert_eq!(r.svc.store.device_cap("phone-b").unwrap(), None);
    assert!(call(&r.owner(), set_access_req("bob", "owner")).await.is_err(), "nobody is given the owner's access");
    assert!(call(&r.owner(), set_access_req("nobody", "viewer")).await.is_err());
}

/// S4: the admin requests are refused `person_only` from inside a pane
/// (an agent's prompt-injected call), and still work outside one.
#[tokio::test]
async fn admin_requests_from_inside_a_pane_are_refused() {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let agent = svc.create_terminal(worktree.id, "Agent 2", "claude").await.expect("an agent");
    svc.store.set_member("bob", "Bob", &["phone-b".into()]).unwrap();
    let snapshot = svc.inventory.refresh().await;
    let pid = snapshot.claimants(agent.id).into_iter().next().expect("the agent's pane").stamp.pid as i32;
    let in_pane = factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(pid));
    let member_remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "bob".into() })),
        ..Default::default()
    };
    for (what, req) in [
        ("member.set", set("carol", &[], "", "")),
        ("member.remove", member_remove),
        ("client.enroll", enroll_req(9, "phone-x", Scope::Control)),
        ("member.set_access", set_access_req("bob", "viewer")),
        ("client.set_cap", set_cap_req("phone-b", "viewer")),
    ] {
        assert_eq!(call(&in_pane, req).await.err().as_deref(), Some("resource-conflict:person_only"), "{what}");
    }
    assert!(svc.store.member("bob").is_ok(), "Bob was removed from a pane");
    assert!(svc.store.member("carol").is_err(), "a member was added from a pane");
    let person = crate::test_support::Outside::a_person(dir.path(), 6591);
    let outside = factory(&svc, None, Scope::HostAdmin).with_caller_pid(Some(person.pid));
    assert!(call(&outside, set("carol", &[], "", "")).await.is_ok(), "outside a pane, the same call is served");
}

// MARK: - Authority, rechecked under the lock

/// A request's task parked on its way into the fence lock: `go` lets it on.
fn parked<T: Send + 'static>(
    work: impl FnOnce(Box<dyn FnOnce() + Send>) -> tokio::task::JoinHandle<T>,
) -> (mpsc::Receiver<()>, mpsc::Sender<()>, tokio::task::JoinHandle<T>) {
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let held: Box<dyn FnOnce() + Send> = Box::new(move || {
        reached_tx.send(()).unwrap();
        go_rx.recv().unwrap();
    });
    (reached_rx, go_tx, work(held))
}

async fn reached(rx: mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || rx.recv().unwrap()).await.unwrap();
}

/// S2: Bob's enroll was decided while he was a Member, and waits on the lock
/// while he is made a Viewer. It is refused `access_changed` and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enroll_decided_before_a_demotion_is_refused_under_the_lock() {
    let r = runner().await;
    let session = r.svc.sessions().open(Some("phone-b".into()));
    let authority = Authority::read(&r.svc.store, Some("phone-b")).unwrap();
    let caller = crate::enrollment::Caller::new(session, Scope::Control).with_authority(authority);
    let (reached_rx, go, enrolling) = parked(|held| {
        let svc = r.svc.clone();
        tokio::spawn(async move {
            let asked = asking(30, "browser-b", Scope::Control, "");
            crate::enrollment::enroll_held_for_tests(&svc, &asked, caller, held).await
        })
    });
    reached(reached_rx).await;
    set_access(&r.svc, "bob", Viewer, || ()).await.unwrap();
    let before = r.file();
    go.send(()).unwrap();
    let answer = enrolling.await.unwrap();
    assert!(matches!(answer, Err(DomainError::Conflict { what: "access_changed" })), "{answer:?}");
    assert_eq!(r.file(), before, "a line was written for a demoted Member");
    assert!(!r.lines().contains_key("browser-b"));
}

/// The same enroll with nothing changed in between is served and bound to
/// Bob, over the wire, and a line below his ceiling is capped.
#[tokio::test]
async fn an_enroll_by_a_member_is_bound_to_them_and_capped_below_their_ceiling() {
    let r = runner().await;
    let bob = factory(&r.svc, Some("phone-b"), Scope::Control);
    call(&bob, enroll_req(30, "browser-b", Scope::Control)).await.expect("served");
    call(&bob, enroll_req(31, "glance-b", Scope::Read)).await.expect("served");
    assert_eq!(r.svc.store.member_of_device("browser-b").unwrap().as_deref(), Some("bob"));
    assert_eq!(r.svc.store.device_cap("browser-b").unwrap(), None, "a control line is at Bob's ceiling");
    assert_eq!(r.svc.store.device_cap("glance-b").unwrap(), Some(Cap::Viewer));
    assert!(r.svc.store.stored_access().unwrap().owner_devices.iter().all(|(d, _)| d != "browser-b"), "the owner's by mistake");
    call(&r.owner(), set_access_req("bob", "viewer")).await.unwrap();
    let demoted = call(&factory(&r.svc, Some("phone-b"), Scope::Control), enroll_req(32, "again-b", Scope::Read)).await;
    assert!(demoted.is_err(), "a Viewer's phone enrolled a device: {demoted:?}");
}

/// S2: an Admin's revoke decided before they were made a Member is refused
/// under the lock, and the device keeps its line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoke_decided_before_a_demotion_is_refused_under_the_lock() {
    let r = runner().await;
    let (reached_rx, go, revoking) = parked(|held| {
        let svc = r.svc.clone();
        tokio::spawn(async move { revoke_held(&svc, Some("phone-d"), &pb::ClientRevoke { client_id: "tv-c".into(), with_added: false }, held).await })
    });
    reached(reached_rx).await;
    set_access(&r.svc, "dan", Member, || ()).await.unwrap();
    go.send(()).unwrap();
    let answer = revoking.await.unwrap();
    assert!(matches!(answer, Err(DomainError::Conflict { what: "access_changed" })), "{answer:?}");
    assert!(r.lines().contains_key("tv-c"), "Carol's display was revoked by a demoted Admin");
    assert_eq!(r.svc.store.member_of_device("tv-c").unwrap().as_deref(), Some("carol"));
}

/// S2: `member.set` takes the fence lock and checks its caller there. Dan's
/// is decided as an Admin and lands after his phone was capped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn member_set_decided_before_a_cap_is_refused_under_the_lock() {
    let r = runner().await;
    enroll(&r.svc, 40, "pad-x", Scope::Read).await;
    let authority = Authority::read(&r.svc.store, Some("phone-d")).unwrap();
    let (reached_rx, go, setting) = parked(|held| {
        let svc = r.svc.clone();
        let asked = set("bob", &["phone-b", "pad-b", "pad-x"], "", "");
        let Some(request::Payload::MemberSet(asked)) = asked.payload else { unreachable!() };
        tokio::spawn(async move { member_set(&svc, Some("phone-d"), authority, asked, held).await })
    });
    reached(reached_rx).await;
    set_cap(&r.svc, "phone-d", Some(Cap::Member), || ()).await.unwrap();
    go.send(()).unwrap();
    let answer = setting.await.unwrap();
    assert!(matches!(answer, Err(DomainError::Conflict { what: "access_changed" })), "{:?}", answer.map(|w| w.member.devices));
    assert_eq!(r.svc.store.member("bob").unwrap().devices, ["pad-b", "phone-b"], "Bob's devices changed");
}

/// `member.set` is decided where the write is: it names only devices that
/// have lines there, binds them with an access and a cap, and writes no line.
#[tokio::test]
async fn member_set_binds_a_new_members_devices_with_their_access_and_caps() {
    let r = runner().await;
    enroll(&r.svc, 41, "tablet-g", Scope::Read).await;
    enroll(&r.svc, 42, "phone-g", Scope::Control).await;
    let before = r.file();
    assert!(call(&r.owner(), set("gus", &["phone-nowhere"], "", "")).await.is_err(), "a device with no line");
    call(&r.owner(), set("gus", &["phone-g", "tablet-g"], "", "")).await.unwrap();
    assert_eq!(r.file(), before, "binding writes no line");
    assert_eq!(r.svc.store.member_access("gus").unwrap(), Some(Member), "the highest of their lines");
    assert_eq!(r.svc.store.device_cap("tablet-g").unwrap(), Some(Cap::Viewer));
    assert_eq!(r.svc.store.device_cap("phone-g").unwrap(), None);
    let owners = r.svc.store.stored_access().unwrap().owner_devices;
    assert!(owners.iter().all(|(d, _)| d != "phone-g" && d != "tablet-g"), "no longer the owner's: {owners:?}");
}

/// Slice 2 review N1: `member.remove` looks at its caller again before it
/// stops or revokes anything. An Admin's removal decided before they were made
/// a Member is refused `access_changed`; a demoted one's fresh call is refused
/// at the gate; and Carol is still a member afterwards.
#[tokio::test]
async fn a_removal_decided_before_a_demotion_is_refused() {
    let r = runner().await;
    let authority = Authority::read(&r.svc.store, Some("phone-d")).unwrap();
    recheck_removal(&r.svc.store, &authority, Some("phone-d"), "carol").expect("nothing changed yet");
    set_access(&r.svc, "dan", Member, || ()).await.unwrap();
    let late = recheck_removal(&r.svc.store, &authority, Some("phone-d"), "carol");
    assert!(matches!(late, Err(DomainError::Conflict { what: "access_changed" })), "{late:?}");
    let other = Authority::read(&r.svc.store, Some("phone-d")).unwrap();
    let admin = recheck_removal(&r.svc.store, &other, Some("phone-d"), "fay");
    assert!(matches!(admin, Err(DomainError::NotYours { what: "not_yours" })), "an Admin is not a Member's to remove: {admin:?}");
    let remove = Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: "carol".into() })),
        ..Default::default()
    };
    assert!(call(&factory(&r.svc, Some("phone-d"), Scope::HostAdmin), remove).await.is_err(), "a demoted Admin's removal");
    assert!(r.svc.store.member("carol").is_ok());
}

/// Slice 2 review N3: one line this build can't rewrite blocks only the
/// change that names it. Another person's access change is served and leaves
/// that line as it was; a change to the odd line's own device is refused
/// `repair_device`, and nothing is written.
#[tokio::test]
async fn a_line_that_cant_be_rewritten_blocks_only_its_own_device() {
    let r = runner().await;
    enroll(&r.svc, 70, "odd-o", Scope::Control).await;
    let text = std::fs::read_to_string(r.svc.authorized_keys()).unwrap();
    let odd = text.lines().find(|l| l.contains("odd-o")).unwrap().to_string();
    assert!(odd.starts_with("restrict,command="), "{odd}");
    std::fs::write(r.svc.authorized_keys(), text.replace(&odd, &odd.replacen("restrict,", "restrict,pty,", 1))).unwrap();
    // The store says this device is a Viewer's; its line says `control`.
    r.svc.store.set_device_cap("odd-o", Some(Cap::Viewer)).unwrap();
    let before = r.file();

    set_access(&r.svc, "bob", Viewer, || ()).await.expect("Bob's change is not blocked by someone else's odd line");
    assert_eq!(r.lines()["phone-b"], Scope::Read, "Bob's phone was lowered");
    let after = String::from_utf8(r.file()).unwrap();
    assert!(after.contains(&odd.replacen("restrict,", "restrict,pty,", 1)), "the odd line was left as it was");
    assert_ne!(r.file(), before);

    let before = r.file();
    let refused = set_cap(&r.svc, "odd-o", Some(Cap::Viewer), || ()).await;
    assert!(matches!(refused, Err(DomainError::Conflict { what: "repair_device" })), "{refused:?}");
    assert_eq!(r.file(), before, "a refused change wrote a line");
}

// MARK: - What an Admin manages

/// An Admin manages Members, Viewers and themselves: not another Admin and
/// not what another Admin added; `member.set` too.
#[tokio::test]
async fn an_admin_manages_members_viewers_and_themselves() {
    let r = runner().await;
    enroll(&r.svc, 50, "kid-f", Scope::Control).await;
    call(&r.owner(), set("fay", &["phone-f", "kid-f"], "", "")).await.unwrap();
    r.svc.store.record_added_by("kid-f", "pad-b").unwrap();
    let dan = factory(&r.svc, Some("phone-d"), Scope::HostAdmin);

    assert_eq!(call(&dan, revoke_req("phone-f", false)).await.err().as_deref(), Some("not-yours:not_yours"), "another Admin's phone");
    assert!(r.lines().contains_key("phone-f"));
    // Bob's pad, and with it what it added: Fay's kid-f is spared.
    call(&dan, revoke_req("pad-b", true)).await.unwrap();
    assert!(!r.lines().contains_key("pad-b"));
    assert!(r.lines().contains_key("kid-f"), "the cascade took an Admin's device");
    // The owner's devices are never an Admin's.
    assert!(call(&dan, revoke_req("phone-o", false)).await.is_err());
    assert!(r.lines().contains_key("phone-o"));
    // Viewers' devices are.
    call(&dan, revoke_req("tv-c", false)).await.unwrap();
    assert!(!r.lines().contains_key("tv-c"));

    let remove = |handle: &str| Request {
        method: "member.remove".into(),
        payload: Some(request::Payload::MemberRemove(pb::MemberRemove { handle: handle.into() })),
        ..Default::default()
    };
    assert_eq!(call(&dan, remove("fay")).await.err().as_deref(), Some("not-yours:not_yours"), "removing another Admin");
    assert!(r.svc.store.member("fay").is_ok());
    let name_fays = set("bob", &["phone-b", "kid-f"], "", "");
    assert_eq!(call(&dan, name_fays).await.err().as_deref(), Some("not-yours:not_yours"), "binding another Admin's device to Bob");
    assert_eq!(call(&dan, set("fay", &["phone-f", "kid-f"], "", "")).await.err().as_deref(), Some("not-yours:not_yours"), "editing another Admin");
    assert_eq!(r.svc.store.member("bob").unwrap().devices, ["pad-b", "phone-b"], "Bob's list is as it was");
    assert!(call(&dan, set("bob", &["phone-b"], "", "")).await.is_ok(), "editing a Member");
    call(&dan, remove("bob")).await.expect("removing a Member");
    assert!(r.svc.store.member("bob").is_err());
}

// MARK: - A rebuild keeps the tunnel

const KEY_ONE: u8 = 60;

/// S8: an access change lands while an enroll is parked on its way to the
/// tunnel. The rebuild changes the scope word and nothing else, so the node
/// key stays on the line and the tunnel's allowlist is what it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cap_rebuild_keeps_the_node_key_and_the_tunnels_allowlist() {
    let r = runner().await;
    let tunnel = Arc::new(crate::tunnel_gate::recording::Recording::default());
    r.svc.tunnel.use_backend(tunnel.clone());
    let laptop = asking(KEY_ONE, "laptop", Scope::Control, NODE_A);
    crate::enrollment::enroll(&r.svc, &laptop).await.unwrap();
    assert_eq!(tunnel.admits(), Some(vec![NODE_A.to_string()]));

    let (reached_rx, go, enrolling) = parked(|pause| {
        let svc = r.svc.clone();
        tokio::spawn(async move {
            let asked = asking(KEY_ONE + 1, "tablet", Scope::Control, NODE_E);
            crate::enrollment::enroll_at_tunnel_for_tests(&svc, &asked, pause).await
        })
    });
    reached(reached_rx).await;
    set_cap(&r.svc, "laptop", Some(Cap::Viewer), || ()).await.unwrap();
    go.send(()).unwrap();
    assert_eq!(enrolling.await.unwrap().unwrap().conn_blob, "token");

    let laptop_line = entries(&r.svc).into_iter().find(|e| e.client_id == "laptop").unwrap();
    assert_eq!((laptop_line.scope, laptop_line.node_key.as_str()), (Scope::Read, NODE_A), "the line lost its node key or kept its scope");
    assert_eq!(tunnel.admits(), Some(vec![NODE_A.to_string(), NODE_E.to_string()]), "the allowlist changed");
}

// MARK: - The start-up refresh

/// SF1: a later start adds the rows a member added or a device an older
/// build enrolled lacks, drops rows no line backs, and lowers a line above
/// what the tables say. It never raises one.
#[tokio::test]
async fn a_later_start_refreshes_the_tables_and_only_ever_lowers_a_line() {
    let r = runner().await;
    let store = &r.svc.store;
    // Written behind the runner's back, as an older build would have.
    enroll_without_rows(&r, 70, "phone-h", Scope::Control);
    store.set_member("hal", "Hal", &["phone-h".into()]).unwrap();
    store.bind_owner_device("ghost-o", None).unwrap();
    store.set_device_cap("ghost-c", Some(Cap::Viewer)).unwrap();
    // Stored lower than the line says, and higher than it.
    store.set_device_cap("phone-b", Some(Cap::Viewer)).unwrap();
    store.set_member_access("carol", Member).unwrap();
    store.set_device_cap("tv-c", None).unwrap();

    crate::access_migration::migrate_at_start(&r.svc).await;
    let stored = store.stored_access().unwrap();
    assert_eq!(store.member_access("hal").unwrap(), Some(Member), "a member the tables lacked");
    assert!(stored.owner_devices.iter().all(|(d, _)| d != "ghost-o"), "a row no line backs");
    assert!(stored.caps.iter().all(|(d, _)| d != "ghost-c"));
    assert_eq!(r.lines()["phone-b"], Scope::Read, "a line above its cap is lowered");
    assert_eq!(r.lines()["tv-c"], Scope::Read, "and one below its person's access is not raised");
    assert_eq!(store.member_access("carol").unwrap(), Some(Member), "a recorded access has its say");
}

/// A line written without going through `client.enroll`, so with no rows.
fn enroll_without_rows(r: &Runner, n: u8, id: &str, scope: Scope) {
    let line = farcooler_fence::render(&key(n), id, id, scope, farcooler_fence::Grant::FarCooler, None).unwrap();
    let mut keep: Vec<String> = entries(&r.svc).into_iter().map(|e| e.line).collect();
    keep.push(line);
    farcooler_fence::write(r.svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS, &keep, &[], farcooler_fence::Placement::Last).unwrap();
}

/// SF2: a stale owner row for an id that is released now (a teammate
/// enrolled under it) is nobody's, not the owner's.
#[tokio::test]
async fn a_released_device_with_a_stale_owner_row_is_nobodys() {
    let r = runner().await;
    r.svc.store.record_teammate_pending("stale-o").unwrap();
    let view = {
        let mut view = View::from_store(&r.svc.store).unwrap();
        view.owners_insert_for_tests("stale-o");
        view
    };
    assert_eq!(view.access_of("stale-o"), None);
    assert_eq!(view.access_of("phone-o"), Some(Access::Owner));
}

/// SF1 for the start: the first run still writes no line.
#[tokio::test]
async fn the_first_start_writes_no_line() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    enroll(&svc, 1, "phone-o", Scope::Control).await;
    enroll(&svc, 2, "ipad-o", Scope::Read).await;
    let before = std::fs::read(svc.authorized_keys()).unwrap();
    crate::access_migration::migrate_at_start(&svc).await;
    assert_eq!(std::fs::read(svc.authorized_keys()).unwrap(), before);
    assert!(svc.store.access_migrated().unwrap());
    assert_eq!(svc.store.device_cap("ipad-o").unwrap(), Some(Cap::Viewer));
}

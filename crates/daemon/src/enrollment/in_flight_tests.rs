//! An enroll already running on a device's own session when that device is
//! revoked (ov-826, the ov-820 re-review's F1).
//!
//! A revoke closes the device's sessions only after its write lands, and a
//! connection finishes a request it was already running. So an enroll that
//! was waiting on the fence lock got it after the revoke, wrote the device's
//! line back, and readmitted it. Each test here parks the enroll on its way
//! into the lock (`enroll_held`), runs the revoke to completion, then lets
//! the enroll go: no sleeps, and the order is the one the test says.

use std::sync::Arc;
use std::sync::mpsc;

use farcooler_core::DomainError;
use farcooler_protocol::v1::{self as pb, ClientEnroll, ClientRevoke, Request, Scope, request, response};
use farcooler_transport::{Handler as _, Peer};

use crate::service::Service;
use crate::sessions::Session;

const KEY_PHONE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMj phone";
const KEY_PHONE_NEW: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk phone";
const KEY_MAC_SHELL: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUlJSUl mac";

/// A runner on a scratch root with the phone enrolled at `host_admin`, by
/// the local socket.
async fn runner() -> (tempfile::TempDir, Arc<Service>) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let ssh = dir.path().join("home").join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(ssh.join("authorized_keys")));
    super::enroll(&svc, &asking(KEY_PHONE, "phone", false)).await.expect("the phone enrolls");
    (dir, svc)
}

fn asking(key: &str, client_id: &str, shell_access: bool) -> ClientEnroll {
    ClientEnroll {
        public_key: key.into(),
        label: "A device".into(),
        client_id: client_id.into(),
        scope: Scope::HostAdmin as i32,
        shell_access,
        node_key: String::new(),
        for_teammate: false,
    }
}

/// The phone's lines, as `(key fingerprint, shell)`.
async fn phone_lines(svc: &Service) -> Vec<(String, bool)> {
    super::list(svc)
        .await
        .unwrap()
        .items
        .into_iter()
        .filter(|e| e.client_id == "phone")
        .map(|e| (e.fingerprint, e.shell_access))
        .collect()
}

fn fingerprint(key: &str) -> String {
    let line = farcooler_fence::render(key, "x", "x", Scope::HostAdmin, farcooler_fence::Grant::FarCooler, None).unwrap();
    super::read_back(&line).unwrap().fingerprint
}

async fn revoke(svc: &Service) {
    super::revoke(svc, &ClientRevoke { client_id: "phone".into() }).await.expect("the phone is revoked");
}

/// An enroll of `asked` on `caller`, started and parked on its way into the
/// fence lock. `go` lets it on; awaiting the handle answers what it did.
async fn parked(
    svc: &Arc<Service>,
    asked: ClientEnroll,
    caller: Arc<Session>,
) -> (mpsc::Sender<()>, tokio::task::JoinHandle<farcooler_core::Result<pb::ClientEnrollResult>>) {
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let enrolling = {
        let svc = svc.clone();
        tokio::spawn(async move {
            let held = move || {
                reached_tx.send(()).unwrap();
                go_rx.recv().unwrap();
            };
            super::enroll_held(&svc, &asked, None, Some(caller), held, None).await
        })
    };
    tokio::task::spawn_blocking(move || reached_rx.recv().unwrap()).await.unwrap();
    (go_tx, enrolling)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enroll_in_flight_when_its_device_is_revoked_writes_nothing() {
    let (_dir, svc) = runner().await;
    let session = svc.sessions().open(Some("phone".into()));
    // The phone's own line again, which is what an attacker holding it
    // would send: after the revoke there's no line, so it would be written.
    let (go, enrolling) = parked(&svc, asking(KEY_PHONE, "phone", false), session.clone()).await;

    revoke(&svc).await;
    assert!(session.is_closed(), "the revoke didn't close the phone's session");
    assert!(phone_lines(&svc).await.is_empty(), "the revoke left a line");
    go.send(()).unwrap();

    let answer = enrolling.await.unwrap();
    assert!(matches!(answer, Err(DomainError::AuthRequired)), "the enroll wasn't refused: {answer:?}");
    assert!(phone_lines(&svc).await.is_empty(), "the revoked phone's enroll put its line back");
    assert!(svc.sessions().open(Some("phone".into())).is_closed(), "the revoked phone was readmitted");
}

/// The mark is lifted by a real re-enrollment, but the session the enroll
/// arrived on was still revoked after it opened.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enroll_in_flight_stays_refused_once_its_device_is_enrolled_again() {
    let (_dir, svc) = runner().await;
    let session = svc.sessions().open(Some("phone".into()));
    let (go, enrolling) = parked(&svc, asking(KEY_PHONE, "phone", false), session).await;

    revoke(&svc).await;
    super::enroll(&svc, &asking(KEY_PHONE_NEW, "phone", false)).await.expect("the phone is enrolled again, with a new key");
    go.send(()).unwrap();

    let answer = enrolling.await.unwrap();
    assert!(matches!(answer, Err(DomainError::AuthRequired)), "the enroll wasn't refused: {answer:?}");
    assert_eq!(phone_lines(&svc).await, vec![(fingerprint(KEY_PHONE_NEW), false)], "the old session's key came back");
}

/// A normal enroll on a device's session, and a readmitted device's new
/// session enrolling its second key, both write.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enroll_on_a_session_nobody_revoked_writes() {
    let (_dir, svc) = runner().await;
    let session = svc.sessions().open(Some("phone".into()));
    let (go, enrolling) = parked(&svc, asking(KEY_MAC_SHELL, "phone", true), session).await;
    go.send(()).unwrap();
    let answer = enrolling.await.unwrap().expect("a normal enroll");
    assert!(!answer.already_enrolled);
    assert!(phone_lines(&svc).await.contains(&(fingerprint(KEY_MAC_SHELL), true)));

    revoke(&svc).await;
    super::enroll(&svc, &asking(KEY_PHONE_NEW, "phone", false)).await.expect("enrolled again");
    let fresh = svc.sessions().open(Some("phone".into()));
    assert!(!fresh.is_closed(), "the readmitted phone is shut out");
    super::enroll_as(&svc, &asking(KEY_MAC_SHELL, "phone", true), None, Some(fresh)).await.expect("the readmitted phone's own enroll");
    assert_eq!(phone_lines(&svc).await.len(), 2, "both of the readmitted phone's keys");
}

/// The wire: `client.enroll` hands the connection's session to the enroll.
/// Staged in the window between a revoke's write and its close: the device
/// is marked, its session isn't closed yet, so only the check stops it.
#[tokio::test]
async fn client_enroll_over_a_connection_carries_its_session() {
    let (_dir, svc) = runner().await;
    let peer = Peer { client_id: Some("phone".into()), scope: Scope::HostAdmin };
    let phone = crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), Arc::new(tokio::sync::Notify::new()), peer);
    svc.sessions().mark_revoked("phone");
    let req = Request {
        method: "client.enroll".into(),
        payload: Some(request::Payload::ClientEnroll(asking(KEY_MAC_SHELL, "phone", true))),
        ..Default::default()
    };
    let answer = phone.handle(req).await.outcome;
    let Some(response::Outcome::Error(error)) = answer else { panic!("the marked phone's enroll was served: {answer:?}") };
    assert_eq!(farcooler_core::error::word_for(error.code), "auth-required");
    assert_eq!(phone_lines(&svc).await, vec![(fingerprint(KEY_PHONE), false)], "a line was written");
}

/// Two node keys that decode, and differ (A and E: see
/// `tests/a_sweep_rebuilds_the_tunnel_once.rs`).
const NODE_PHONE: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const NODE_LAPTOP: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";
const KEY_LAPTOP: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz laptop";
const KEY_BROWSER: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER browser";

/// [`runner`], serving a recorded tunnel that admits the laptop, which a
/// revoke's rebuild keeps.
async fn tunneled() -> (tempfile::TempDir, Arc<Service>, Arc<crate::tunnel_gate::recording::Recording>) {
    let (dir, svc) = runner().await;
    let tunnel = Arc::new(crate::tunnel_gate::recording::Recording::default());
    svc.tunnel.use_backend(tunnel.clone());
    let laptop = ClientEnroll { node_key: NODE_LAPTOP.into(), scope: Scope::Control as i32, ..asking(KEY_LAPTOP, "laptop", false) };
    let paired = super::enroll(&svc, &laptop).await.expect("the laptop enrolls");
    assert_eq!(paired.conn_blob, "token", "the laptop's pairing started no tunnel");
    assert_eq!(tunnel.admits(), Some(vec![NODE_LAPTOP.to_string()]));
    (dir, svc, tunnel)
}

/// An enroll of `asked` that writes, then parks on its way to the tunnel.
async fn parked_at_the_tunnel(
    svc: &Arc<Service>,
    asked: ClientEnroll,
    caller: Option<Arc<Session>>,
) -> (mpsc::Sender<()>, tokio::task::JoinHandle<farcooler_core::Result<pb::ClientEnrollResult>>) {
    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let enrolling = {
        let svc = svc.clone();
        tokio::spawn(async move {
            let pause: Box<dyn FnOnce() + Send> = Box::new(move || {
                reached_tx.send(()).unwrap();
                go_rx.recv().unwrap();
            });
            super::enroll_held(&svc, &asked, None, caller, || (), Some(pause)).await
        })
    };
    tokio::task::spawn_blocking(move || reached_rx.recv().unwrap()).await.unwrap();
    (go_tx, enrolling)
}

/// S1, the device's own key (ov-832): an enroll wrote the phone's line with
/// a node key, and before it reached the tunnel a revoke took the line out
/// and rebuilt the tunnel without it. The enroll then admits nothing: the
/// key it would add is in no line any more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_written_before_a_revoke_never_reaches_the_tunnel_it_rebuilt() {
    let (_dir, svc, tunnel) = tunneled().await;
    let tablet = ClientEnroll { node_key: NODE_PHONE.into(), scope: Scope::Control as i32, ..asking(KEY_BROWSER, "tablet", false) };
    let (go, enrolling) = parked_at_the_tunnel(&svc, tablet, None).await;

    super::revoke(&svc, &ClientRevoke { client_id: "tablet".into() }).await.expect("the tablet is revoked");
    assert_eq!(tunnel.served.lock().unwrap().last(), Some(&vec![NODE_LAPTOP.to_string()]), "the revoke didn't rebuild the tunnel");
    go.send(()).unwrap();

    let answer = enrolling.await.unwrap().expect("the write had landed");
    assert_eq!(answer.conn_blob, "", "a revoked device was handed the tunnel");
    assert_eq!(tunnel.admits(), Some(vec![NODE_LAPTOP.to_string()]), "a revoked device's node key reached the rebuilt tunnel");
}

/// S1, the caller (ov-832): the phone enrolled a browser and was revoked
/// before the browser's key reached the tunnel. Its session is refused the
/// admission under the tunnel's hold, as its enroll would be under the
/// fence lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_sessions_enroll_admits_nothing_to_the_tunnel() {
    let (_dir, svc, tunnel) = tunneled().await;
    let session = svc.sessions().open(Some("phone".into()));
    let browser = ClientEnroll { node_key: NODE_PHONE.into(), scope: Scope::Control as i32, ..asking(KEY_BROWSER, "browser", false) };
    let (go, enrolling) = parked_at_the_tunnel(&svc, browser, Some(session)).await;

    revoke(&svc).await;
    go.send(()).unwrap();

    let answer = enrolling.await.unwrap().expect("the write had landed");
    assert_eq!(answer.conn_blob, "", "a revoked session was handed the tunnel");
    assert_eq!(tunnel.admits(), Some(vec![NODE_LAPTOP.to_string()]), "a revoked session's enroll reached the tunnel");
}

/// The ordinary case still admits: nobody revoked anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enroll_nobody_revoked_is_admitted_to_the_running_tunnel() {
    let (_dir, svc, tunnel) = tunneled().await;
    let session = svc.sessions().open(Some("phone".into()));
    let browser = ClientEnroll { node_key: NODE_PHONE.into(), scope: Scope::Control as i32, ..asking(KEY_BROWSER, "browser", false) };
    let (go, enrolling) = parked_at_the_tunnel(&svc, browser, Some(session)).await;
    go.send(()).unwrap();
    assert_eq!(enrolling.await.unwrap().unwrap().conn_blob, "token");
    assert_eq!(tunnel.admits(), Some(vec![NODE_LAPTOP.to_string(), NODE_PHONE.to_string()]));
}

/// N4 (ov-832): `client.set_node_key` on a session whose device was revoked
/// since it opened writes nothing, as an enroll does.
#[tokio::test]
async fn a_revoked_sessions_node_key_is_refused() {
    let (_dir, svc) = runner().await;
    let session = svc.sessions().open(Some("phone".into()));
    let peer = Peer { client_id: Some("phone".into()), scope: Scope::Read };
    let asked = pb::ClientSetNodeKey { node_key: NODE_PHONE.into(), ..Default::default() };
    svc.sessions().mark_revoked("phone");
    let answer = super::set_node_key_on(&svc, &peer, &asked, Some(session.clone())).await;
    assert!(matches!(answer, Err(DomainError::AuthRequired)), "the revoked session's node key was taken: {answer:?}");
    assert_eq!(phone_lines(&svc).await, vec![(fingerprint(KEY_PHONE), false)], "the phone's line changed");
    let entries = farcooler_fence::read(svc.authorized_keys(), farcooler_fence::AUTHORIZED_KEYS).unwrap();
    assert!(entries.iter().all(|e| e.node_key.is_empty()), "a node key was written for a revoked session");
}

/// S2 (ov-832): a revoke outlives a restart. The store is opened again, as
/// a restarted daemon opens it, and the revoked phone's new session, a new
/// channel on a still-open ssh master, starts closed and can't enroll its
/// line back. A real enrollment lifts the mark, for good.
#[tokio::test]
async fn a_revoke_outlives_a_daemon_restart() {
    let (dir, svc) = runner().await;
    revoke(&svc).await;
    drop(svc);

    let state = dir.path().join("state");
    let keys = dir.path().join("home").join(".ssh").join("authorized_keys");
    let svc = Arc::new(Service::open_in(state.clone()).await.unwrap().enrolling_into(keys.clone()));
    let session = svc.sessions().open(Some("phone".into()));
    assert!(session.is_closed(), "after a restart, the revoked phone's session was served");
    let answer = super::enroll_as(&svc, &asking(KEY_PHONE, "phone", false), None, Some(session)).await;
    assert!(matches!(answer, Err(DomainError::AuthRequired)), "after a restart, the revoked phone enrolled itself: {answer:?}");
    assert!(phone_lines(&svc).await.is_empty(), "the revoked phone's line came back");

    super::enroll(&svc, &asking(KEY_PHONE_NEW, "phone", false)).await.expect("the owner enrolls the phone again");
    drop(svc);
    let svc = Arc::new(Service::open_in(state).await.unwrap().enrolling_into(keys));
    assert!(!svc.sessions().open(Some("phone".into())).is_closed(), "a phone enrolled again was shut out after a restart");
}

/// A kept mark the store can't lift (ov-832) refuses the enroll that would
/// lift it: the line and the mark never disagree, and the device stays out.
#[tokio::test]
async fn an_enroll_whose_mark_cant_be_lifted_writes_nothing() {
    let (_dir, svc) = runner().await;
    revoke(&svc).await;
    farcooler_store::testing::break_revoked_marks(&svc.store);
    let answer = super::enroll(&svc, &asking(KEY_PHONE_NEW, "phone", false)).await;
    assert!(answer.is_err(), "an enroll whose mark couldn't be lifted wrote: {answer:?}");
    assert!(phone_lines(&svc).await.is_empty(), "a line was written while the device stayed marked");
    assert!(svc.sessions().open(Some("phone".into())).is_closed(), "the mark was lifted in memory alone");
}

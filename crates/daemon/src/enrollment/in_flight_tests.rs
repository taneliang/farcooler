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
            super::enroll_held(&svc, &asked, None, Some(caller), held).await
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

//! Revocation is containment, so it has to have happened when it answers.
//!
//! sshd reads `authorized_keys` at authentication and never again, so deleting
//! a device's line leaves the session it already holds running. A revoke that
//! returned before closing that session would report a containment that had not
//! happened — and the person reading the answer acts on it.
//!
//! These run against a real socket, the real dispatch table and the real
//! `authorized_keys` writer (pointed at a scratch file per harness, because
//! `client.enroll` writes SSH keys and a test that reached the developer's own
//! file could take away their access to their own machine).
//!
//! What a connection IS here matters as much as what it does: each one is built
//! by the daemon's own `peer_from_preamble` and `RpcFactory::new`, not by a copy
//! of them living in this file. A copy would let the very bug this covers — the
//! preamble's client id being parsed, relayed and then dropped — pass, because
//! the test would be asserting against its own wiring.

use std::sync::Arc;

use farcooler_daemon::{rpc::RpcFactory, service::Service, sessions::peer_from_preamble};
use farcooler_protocol::v1::{Scope, request, result};
use farcooler_transport::{
    Client, ClientError, HandshakeConfig, UnixListenerServer, request,
};
use tokio::io::AsyncWriteExt;

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

/// A daemon on a private socket, with a private database and its own
/// `authorized_keys`.
struct Harness {
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
    /// Held so a test can look at the session registry the daemon is keeping,
    /// which is where the ordering this file is about is actually observable.
    service: Arc<Service>,
    tmux_socket: String,
}

/// Take the tmux server down with the test that started it.
///
/// Nothing here opens a terminal, but `Service::open_in` inventories tmux on the
/// way up, and a server left behind per test run accumulates across runs until
/// new ones stop starting. The same guard `rpc_over_socket.rs` carries.
impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

async fn start() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("farcoolerd.sock");
    // The `.ssh` inside it is created by the write itself, at 0700 — what is
    // needed here is the directory ABOVE it, which the writer anchors to.
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let service = Arc::new(
        Service::open_in(dir.path().to_path_buf())
            .await
            .expect("service")
            .enrolling_into(home.join(".ssh").join("authorized_keys")),
    );
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");

    // Constructed but not run: these tests are about dispatch and session
    // lifetime, and a sampling loop would make them race a tmux that may not
    // be there.
    let watcher = farcooler_daemon::watch::Watcher::new(service.clone());

    let served = service.clone();
    tokio::spawn(async move {
        let _ = server
            .serve(move |preamble| {
                // The daemon's own answer to "who is this", called rather than
                // reproduced. See this file's header.
                let peer = peer_from_preamble(preamble.as_ref())?;
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
                    // Nothing waits on this stop signal: the test server has no
                    // process to end.
                    RpcFactory::new(
                        served.clone(),
                        watcher.clone(),
                        Arc::new(tokio::sync::Notify::new()),
                        peer,
                    ),
                ))
            })
            .await;
    });

    // The listener is bound before serve() is spawned, so a connect cannot race
    // it — but give the task a turn so the first accept is already pending.
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, service, tmux_socket }
}

/// Connect, optionally saying which device this is first.
///
/// The preamble is written by hand rather than through a helper, because it is
/// what a `farcoolerd --stdio` relay actually puts on the socket and the point
/// of these tests is that the client id in it survives the whole way to the
/// session registry.
async fn connect(h: &Harness, preamble: Option<&str>) -> SocketClient {
    let stream = tokio::net::UnixStream::connect(&h.socket).await.expect("connect");
    let (read, mut write) = stream.into_split();
    if let Some(line) = preamble {
        write.write_all(line.as_bytes()).await.expect("preamble");
    }
    Client::over(read, write, "test-client", "0.0.0").await.expect("handshake")
}

/// A valid ed25519 public key, chosen for being obviously synthetic.
const PHONE_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA phone";

/// A second one, so a test can tell two devices apart.
const LAPTOP_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER laptop";

async fn enroll(admin: &mut SocketClient, key: &str, label: &str, client_id: &str) {
    let mut req = request("client.enroll");
    req.payload = Some(request::Payload::ClientEnroll(farcooler_protocol::v1::ClientEnroll {
        public_key: key.into(),
        label: label.into(),
        client_id: client_id.into(),
        scope: Scope::Control as i32,
    }));
    admin.call(req).await.expect("client.enroll");
}

/// Revoke, and answer with the client ids still enrolled.
async fn revoke(admin: &mut SocketClient, client_id: &str) -> Vec<String> {
    let mut req = request("client.revoke");
    req.payload = Some(request::Payload::ClientRevoke(farcooler_protocol::v1::ClientRevoke {
        client_id: client_id.into(),
    }));
    let result = admin.call(req).await.expect("client.revoke");
    let Some(result::Value::ClientList(list)) = result.value else { panic!("wrong result") };
    list.items.into_iter().map(|c| c.client_id).collect()
}

/// The device's own session is gone by the time its next call goes out.
///
/// Not "the next call is refused": the connection itself ends, which is what
/// makes this containment rather than a policy check the next feature could
/// forget to run.
#[tokio::test]
async fn a_revoked_client_loses_the_session_it_already_held() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    enroll(&mut admin, PHONE_KEY, "iPhone", "phone").await;

    let mut phone = connect(&h, Some("farcooler-session control phone\n")).await;
    phone.call(request("client.list")).await.expect("the phone could call before it was revoked");

    assert!(!revoke(&mut admin, "phone").await.contains(&"phone".to_string()));

    let outcome = phone.call(request("client.list")).await;
    assert!(
        matches!(outcome, Err(ClientError::Closed | ClientError::Codec(_))),
        "a revoked device's session was still answering: {outcome:?}"
    );
}

/// Revoking one device closes one device.
///
/// The test that catches an over-broad close — "close everything and the
/// revoked one is certainly among it" passes every other assertion in this
/// file. Two live sessions belong to somebody else here: another enrolled
/// device, and the local caller doing the revoking, whose connection carries no
/// client id at all and must never be matched by one.
#[tokio::test]
async fn a_bystander_keeps_the_session_nobody_revoked() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    enroll(&mut admin, PHONE_KEY, "iPhone", "phone").await;
    enroll(&mut admin, LAPTOP_KEY, "MacBook", "laptop").await;

    let mut phone = connect(&h, Some("farcooler-session control phone\n")).await;
    let mut laptop = connect(&h, Some("farcooler-session control laptop\n")).await;
    phone.call(request("client.list")).await.expect("the phone starts connected");
    laptop.call(request("client.list")).await.expect("the laptop starts connected");

    revoke(&mut admin, "phone").await;

    laptop
        .call(request("client.list"))
        .await
        .expect("a device nobody revoked lost the session it was holding");
    admin
        .call(request("client.list"))
        .await
        .expect("the local caller's own connection was closed by its own revoke");
    // So the test cannot pass by closing nothing at all.
    assert!(phone.call(request("client.list")).await.is_err(), "the revoked device survived");
}

/// The order, asserted where nothing can run in between.
///
/// Against `enrollment::revoke` itself rather than across the socket, and that
/// is the whole design of this test. Over the wire, the answer travels through
/// a writer task and a client's `await`, so a revocation that spawned its
/// closing and returned would still have closed by the time any client-side
/// assertion could run — the test would pass on an implementation that reported
/// a containment it had not yet performed. Here there is no scheduling point
/// between `revoke` returning and the assertion, so what is being asserted is
/// the order and nothing else.
///
/// The probe is a session registered for the same device and never served:
/// `close` reaches it exactly as it reaches a real connection's, and
/// `is_closed` reads a flag with no awaiting.
#[tokio::test]
async fn revoke_answers_only_after_it_has_closed() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    enroll(&mut admin, PHONE_KEY, "iPhone", "phone").await;

    let probe = h.service.sessions().open(Some("phone".to_string()));
    assert!(!probe.is_closed(), "a session is open when it starts");

    farcooler_daemon::enrollment::revoke(
        &h.service,
        &farcooler_protocol::v1::ClientRevoke { client_id: "phone".into() },
    )
    .await
    .expect("client.revoke");

    assert!(probe.is_closed(), "revoke answered before the phone's session was closed");
}

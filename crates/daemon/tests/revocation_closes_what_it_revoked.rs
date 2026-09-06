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
        // Key A, the restricted line. A plain line is the other half of a Mac
        // and there is nothing to close on one: sshd runs a shell rather than
        // this daemon, so no session of ours ever arrives on it.
        shell_access: false,
        // No tunnel wanted. Every test in this file is about closing sessions
        // and rebuilding a tunnel, and a device admitted to one here would make
        // the fixture itself start a server — see `line` below, which is how
        // this file builds a tunneled device on purpose.
        node_key: String::new(),
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

/// The tunnel half of containment, where a revoked device's *route* lives.
///
/// Deleting a line stops the next login and closes the sessions this daemon is
/// serving. Neither of those touches the tunnel: tailcat copies the allowlist
/// at `Start` and consults it only when a client first registers, so a device
/// that already peered keeps a path to this host's sshd until the server is
/// replaced. Replacing it is what `enrollment::revoke` now does, and these are
/// the tests that say so.
///
/// **Why the helper backend rather than a plain `cargo test`.** A default build
/// links no tunnel at all, so `farcooler_tailcat::serve` refuses every call for
/// any input — which makes "was the tunnel rebuilt" unobservable there, and any
/// assertion about it vacuous. Under `tailcat-helper` the tunnel is a PROCESS,
/// and a process cannot be faked: `FARCOOLER_TUNNEL_HELPER` points at a shell
/// script that answers the line protocol and writes down every command it was
/// given. `an_empty_allowlist_starts_no_tunnel.rs` makes the same argument for
/// the same reason.
#[cfg(feature = "tailcat-helper")]
mod tunnel {
    use super::*;
    use farcooler_daemon::allowlist::{self, TunnelOutcome};
    use farcooler_fence::Grant;
    use std::os::unix::fs::PermissionsExt;

    /// `FARCOOLER_TUNNEL_HELPER` is process-wide, and so is the running helper
    /// itself — one per process, mirroring the one server a runner has. Two of
    /// these tests in flight at once would each be reading the other's tunnel.
    static TUNNEL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Two node keys that are genuinely two keys. 43 base64 characters carry
    /// 258 bits, so the last character's low two bits are padding: "...AAA"
    /// and "...AAB" decode to the same 32 bytes, and only "...AAE" moves a bit
    /// that is really there. The same pair `allowlist.rs`'s own tests use.
    const NODE_A: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const NODE_B: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";

    /// A running tunnel torn down with the test that started it.
    ///
    /// The helper is held in a process-wide static that never drops, so a test
    /// that just returned would leave a shell blocked on a pipe for as long as
    /// the test binary lives — and the next test would inherit it. `serve` with
    /// an empty allowlist is the crate's own way to say "stop", which is the
    /// same call `revoke` makes and therefore not a second mechanism invented
    /// for the tests.
    struct Tunnel(std::path::PathBuf);

    impl Drop for Tunnel {
        fn drop(&mut self) {
            let _ = farcooler_tailcat::serve(&self.0, 22, &[]);
            // SAFETY: as at the set below — the tunnel lock is still held by
            // the test whose locals are being dropped, and nothing else in
            // this binary reads this variable.
            unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
        }
    }

    /// A helper that serves nothing and writes down everything it was asked.
    ///
    /// Returns the log it writes to. Every command arrives on one line, so the
    /// log is the sequence of commands this runner's tunnel was given — which
    /// is exactly what "the server was rebuilt, without that key" is a claim
    /// about.
    fn fake_helper(h: &Harness) -> std::path::PathBuf {
        let dir = h.service.tailcat_key().parent().expect("a root").to_path_buf();
        let log = dir.join("tunnel-commands.log");
        let script = dir.join("fake-tunnel-helper");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> {log}
  case "$line" in
    blob*) echo "ok fake-blob" ;;
    *) echo "ok" ;;
  esac
done
"#,
                log = log.display()
            ),
        )
        .expect("the fake helper was written");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        // No product path creates this file yet — the device end-to-end run had
        // to write it by hand too — and `tunnel_plan` refuses a runner without
        // one, so every test here would answer `NoIdentity` instead of
        // exercising anything.
        std::fs::write(h.service.tailcat_key(), b"scratch tailcat identity, never read")
            .expect("a scratch tailcat identity");
        std::fs::set_permissions(h.service.tailcat_key(), std::fs::Permissions::from_mode(0o600))
            .unwrap();

        // SAFETY: `set_var` is sound only while no other thread reads the
        // environment. The `TUNNEL` lock keeps the other tests in this module
        // out, and nothing else in this binary touches this variable. The same
        // argument `an_empty_allowlist_starts_no_tunnel.rs` makes.
        unsafe { std::env::set_var("FARCOOLER_TUNNEL_HELPER", &script) };
        log
    }

    /// Place one Far Cooler line, with or without a node key.
    ///
    /// Not `enrollment::enroll`: that now admits the key to a tunnel and will
    /// START one for a runner with none, which is a second effect these tests
    /// are not about — they are about what REVOKING does to a running server,
    /// so the fixture places a line and the test starts the tunnel itself. This
    /// goes to `farcooler_fence`, which is the same primitive
    /// `enrollment::enroll` itself calls.
    async fn line(h: &Harness, key: &str, client_id: &str, node_key: Option<&str>) {
        let rendered = farcooler_fence::render(
            key,
            client_id,
            client_id,
            Scope::Control,
            Grant::FarCooler,
            node_key,
        )
        .expect("a synthetic key and id render");
        let path = h.service.authorized_keys().to_path_buf();
        tokio::task::spawn_blocking(move || {
            farcooler_fence::update(
                &path,
                farcooler_fence::AUTHORIZED_KEYS,
                farcooler_fence::Placement::Last,
                move |entries| {
                    let mut ours: Vec<String> = entries.iter().map(|e| e.line.clone()).collect();
                    ours.push(rendered);
                    Ok::<_, farcooler_fence::FenceError>((
                        farcooler_fence::Change::Write { entries: ours, foreign: Vec::new() },
                        (),
                    ))
                },
            )
        })
        .await
        .expect("the write task ran")
        .expect("authorized_keys accepted the line");
    }

    async fn revoke_directly(h: &Harness, client_id: &str) {
        farcooler_daemon::enrollment::revoke(
            &h.service,
            &farcooler_protocol::v1::ClientRevoke { client_id: client_id.into() },
        )
        .await
        .expect("client.revoke");
    }

    fn commands(log: &std::path::Path) -> String {
        std::fs::read_to_string(log).unwrap_or_default()
    }

    /// The tunnel is serving right now, as far as anything in this process can
    /// tell. `conn_blob` asks the running helper for its token; with no helper
    /// it answers ENOTCONN, which is the backend's word for "nothing is
    /// serving".
    fn serving() -> bool {
        farcooler_tailcat::conn_blob().is_ok()
    }

    /// Revoking one of two tunneled devices rebuilds the tunnel around the one
    /// that is left.
    #[tokio::test]
    async fn the_tunnel_is_rebuilt_without_the_revoked_device() {
        let _serial = TUNNEL.lock().await;
        let h = start().await;
        let log = fake_helper(&h);
        let _tunnel = Tunnel(h.service.tailcat_key());

        line(&h, PHONE_KEY, "phone", Some(NODE_A)).await;
        line(&h, LAPTOP_KEY, "laptop", Some(NODE_B)).await;
        let outcome = allowlist::start_tunnel(&h.service).await;
        assert!(
            matches!(outcome, TunnelOutcome::Serving(_)),
            "the fixture's own tunnel never started: {outcome:?}"
        );
        assert!(
            commands(&log).contains(&format!("serve 22 {NODE_A} {NODE_B}")),
            "both devices were not admitted to begin with: {}",
            commands(&log)
        );

        std::fs::write(&log, "").unwrap();
        revoke_directly(&h, "phone").await;

        let after = commands(&log);
        assert!(
            after.contains(&format!("serve 22 {NODE_B}")),
            "revoking a tunneled device did not rebuild the tunnel: {after:?}"
        );
        assert!(
            !after.contains(NODE_A),
            "the revoked device's node key was handed to the rebuilt tunnel: {after:?}"
        );
        assert!(serving(), "the device nobody revoked lost the tunnel entirely");
    }

    /// The one this whole change exists for: revoking the LAST tunneled device
    /// stops the server rather than leaving it running with that device still
    /// in its set.
    ///
    /// `tunnel_plan` answers `NobodyAdmitted` here, and an implementation that
    /// takes that as "nothing to start" returns without calling `serve` at all
    /// — which leaves the compromised phone peered to a tunnel that no longer
    /// appears in any file. The empty call IS the revocation: `serve` stops
    /// whatever is running before it validates anything, so handing it an
    /// allowlist that admits nobody is how a runner that admits nobody ends
    /// with no tunnel.
    #[tokio::test]
    async fn revoking_the_last_tunneled_device_stops_the_tunnel() {
        let _serial = TUNNEL.lock().await;
        let h = start().await;
        let log = fake_helper(&h);
        let _tunnel = Tunnel(h.service.tailcat_key());

        line(&h, PHONE_KEY, "phone", Some(NODE_A)).await;
        let outcome = allowlist::start_tunnel(&h.service).await;
        assert!(
            matches!(outcome, TunnelOutcome::Serving(_)),
            "the fixture's own tunnel never started: {outcome:?}"
        );
        assert!(serving(), "the fixture's own tunnel is not serving");

        revoke_directly(&h, "phone").await;

        assert!(
            !serving(),
            "the tunnel kept running after the only device it admitted was \
             revoked, so the revoked device still has a route to this sshd: {}",
            commands(&log)
        );
    }

    /// A device that was never in the allowlist costs nobody their tunnel.
    ///
    /// The absence is the assertion. Without the `node_key.is_empty()` guard in
    /// `revoke` this still contains the revoked device perfectly — it was never
    /// admitted — while dropping every other device's live tunnel to do it, and
    /// most revocations are of direct-only devices.
    #[tokio::test]
    async fn revoking_a_device_that_was_never_admitted_leaves_the_tunnel_alone() {
        let _serial = TUNNEL.lock().await;
        let h = start().await;
        let log = fake_helper(&h);
        let _tunnel = Tunnel(h.service.tailcat_key());

        line(&h, PHONE_KEY, "phone", Some(NODE_A)).await;
        line(&h, LAPTOP_KEY, "laptop", None).await;
        let outcome = allowlist::start_tunnel(&h.service).await;
        assert!(
            matches!(outcome, TunnelOutcome::Serving(_)),
            "the fixture's own tunnel never started: {outcome:?}"
        );

        std::fs::write(&log, "").unwrap();
        revoke_directly(&h, "laptop").await;

        assert_eq!(
            commands(&log),
            "",
            "revoking a device with no node key rebuilt the tunnel anyway, \
             which drops every other device's live tunnel for nothing"
        );
        assert!(serving(), "the phone's tunnel went down with a revocation that was not its own");
    }
}

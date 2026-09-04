//! A device registers its OWN node key, over the access it already holds.
//!
//! The whole safety argument for skipping the enrolment ceremony is one
//! sentence: the line written is named by what this runner's `authorized_keys`
//! says, never by anything the connection claims. Registering a node key grants
//! nothing — it adds a route to access the caller is already using to make the
//! call — and it can only ever be added to the caller's own line.
//!
//! **Two levels, because the property can be lost at either.**
//! `enrollment::set_node_key` must read `peer.client_id` and ignore
//! `request.client_id`; and the dispatch arm in `rpc.rs` must hand it the
//! CONNECTION's peer rather than one assembled out of the request. A test at
//! only the first level passes happily while the second is broken, so the last
//! test here runs over a real socket, through the real dispatch table, with a
//! peer built by the daemon's own `peer_from_preamble`.
//!
//! Nothing here touches `allowlist::tunnel_plan` or `start_tunnel`. This call
//! only ever makes an allowlist longer, and the guard that refuses to serve an
//! empty one belongs to boot — see `an_empty_allowlist_starts_no_tunnel.rs`.

use std::path::Path;
use std::sync::Arc;

use farcooler_daemon::{enrollment, rpc::RpcFactory, service::Service, sessions::peer_from_preamble};
use farcooler_fence::Entry;
use farcooler_protocol::v1::{ClientSetNodeKey, Scope, request, result};
use farcooler_transport::{Client, HandshakeConfig, Peer, UnixListenerServer, request};
use tokio::io::AsyncWriteExt;

/// 32 bytes of X25519 public key, as the 43 characters of unpadded base64
/// `usable_node_key` requires. The same synthetic constant the allowlist tests
/// use, for the same reason: obviously not a real key, and the right shape.
const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Distinct ed25519 public keys, one per device.
///
/// Two devices have to be two KEYS: `enroll` refuses a second line for a
/// fingerprint already in the file, so a fixture that reused one would enroll
/// one device and quietly report two.
const KEYS: [&str; 2] = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER one",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi two",
];

/// A third key, for the Mac's plain line in the Key B test.
const SHELL_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz shell";

/// A runner carrying a fleet enrolled BEFORE the tunnel existed.
///
/// Built with `enrollment::enroll`, which renders every line with no node key
/// at all — which is not a shortcut, it is exactly what every device already in
/// the field looks like, and the state this whole call exists to migrate out of.
///
/// A short scratch root, not a deep one: `Service::open_in` stands up a tmux
/// server keyed off this path.
async fn runner_with_devices(client_ids: &[&str]) -> (Service, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a scratch root");
    let service = open_service(dir.path()).await;
    for (index, client_id) in client_ids.iter().enumerate() {
        enroll(&service, KEYS[index], client_id, false).await;
    }
    (service, dir)
}

async fn open_service(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a scratch home");
    Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"))
}

/// One device, through the daemon's own enrollment path.
async fn enroll(service: &Service, public_key: &str, client_id: &str, shell_access: bool) {
    let request = farcooler_protocol::v1::ClientEnroll {
        public_key: public_key.into(),
        label: "device".into(),
        client_id: client_id.into(),
        scope: if shell_access { Scope::HostAdmin as i32 } else { Scope::Control as i32 },
        shell_access,
    };
    let out = enrollment::enroll(service, &request).await.expect("the fixture enrolls");
    assert!(!out.already_enrolled, "the fixture enrolled {client_id} twice");
}

/// The request as an honest client sends it: no device named, because naming
/// one changes nothing. Not `request`, which is `farcooler_transport`'s wire
/// envelope builder and is used further down this file.
fn registering(node_key: &str) -> ClientSetNodeKey {
    ClientSetNodeKey { client_id: String::new(), node_key: node_key.into() }
}

/// Every line in the runner's fence, as the parser reads it back.
///
/// Read through `farcooler_fence` rather than `enrollment::list`, and that is
/// not a preference: `EnrolledClient` — the wire type `list` answers with — has
/// no node key field at all, because nothing a client displays needs one. The
/// file is where the answer lives.
async fn entries(service: &Service) -> Vec<Entry> {
    let path = service.authorized_keys().to_path_buf();
    tokio::task::spawn_blocking(move || {
        farcooler_fence::read(&path, farcooler_fence::AUTHORIZED_KEYS)
    })
    .await
    .expect("the read task ran")
    .expect("authorized_keys parses")
}

/// One device's Key A line — the restricted one, the only shape with a forced
/// command to hold a node key.
fn key_a<'a>(entries: &'a [Entry], client_id: &str) -> &'a Entry {
    entries
        .iter()
        .find(|e| e.client_id == client_id && !e.shell_access)
        .unwrap_or_else(|| panic!("no Key A line for {client_id}"))
}

fn peer(client_id: Option<&str>) -> Peer {
    Peer { client_id: client_id.map(str::to_string), scope: Scope::Read }
}

/// The whole safety argument for skipping the ceremony: a device writes onto
/// its OWN line, named by what this runner's authorized_keys says rather than
/// by anything the connection claims. Registering a node key grants nothing —
/// it adds a route to access the caller is already using to make the call.
#[tokio::test]
async fn a_device_registers_a_node_key_on_its_own_line() {
    let (service, _home) = runner_with_devices(&["c1", "c2"]).await;
    let before = entries(&service).await;
    let (was_labelled, was_fingerprinted) =
        (key_a(&before, "c1").label.clone(), key_a(&before, "c1").fingerprint.clone());
    let c2_line = key_a(&before, "c2").line.clone();

    let out = enrollment::set_node_key(&service, &peer(Some("c1")), &registering(NODE_KEY))
        .await
        .expect("c1 registers its own key");
    // No archive is linked in a `cargo test` build, so there is no token to
    // hand back and an empty one is the honest answer. What must have happened
    // regardless is the write below: the line is what the next boot reads.
    assert!(out.conn_blob.is_empty(), "a stub build produced a tunnel token");

    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c1").node_key, NODE_KEY);
    assert_eq!(key_a(&entries, "c2").node_key, "", "wrote onto another device's line");
    // The rest of the line is the line it was: a rewrite that lost the scope or
    // the client id would take the device's access away while reporting success.
    assert_eq!(key_a(&entries, "c1").scope, Scope::Control);
    assert_eq!(key_a(&entries, "c1").client_id, "c1");
    // The key material and the name are the ones already in the file. A
    // re-render that rebuilt the comment from the label it read would wrap
    // `farcooler-` around itself and rename the device in Settings every time
    // somebody migrated it.
    assert_eq!(key_a(&entries, "c1").fingerprint, was_fingerprinted);
    assert_eq!(key_a(&entries, "c1").label, was_labelled);
    assert_eq!(key_a(&entries, "c2").line, c2_line, "the other device's line was rewritten");
}

#[tokio::test]
async fn a_device_cannot_register_a_node_key_for_another_device() {
    let (service, _home) = runner_with_devices(&["c1", "c2"]).await;
    // The request names c2; the connection is c1. The connection wins.
    let mut req = registering(NODE_KEY);
    req.client_id = "c2".into();

    enrollment::set_node_key(&service, &peer(Some("c1")), &req)
        .await
        .expect("the request is served, on the caller's own line");

    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c2").node_key, "", "c1 wrote onto c2's line");
    assert_eq!(key_a(&entries, "c1").node_key, NODE_KEY);
}

/// A local socket client names no device, so there is no line to write onto.
///
/// `host_admin` deliberately: this is the owner's own Mac app talking to its
/// own daemon, holding every scope there is, and it still has no line here.
/// Being trusted is not the same as being a device.
#[tokio::test]
async fn a_caller_that_names_no_device_is_refused() {
    let (service, _home) = runner_with_devices(&["c1"]).await;
    let local = Peer { client_id: None, scope: Scope::HostAdmin };

    assert!(enrollment::set_node_key(&service, &local, &registering(NODE_KEY)).await.is_err());

    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c1").node_key, "", "a caller with no line wrote onto one");
}

#[tokio::test]
async fn a_malformed_node_key_is_refused_and_changes_nothing() {
    let (service, _home) = runner_with_devices(&["c1"]).await;
    let before = key_a(&entries(&service).await, "c1").line.clone();

    // A quote and a space: the two characters that would end the forced command
    // early and turn the rest of the line into a key of somebody else's
    // choosing. Refused by `usable_node_key` before anything is opened.
    assert!(
        enrollment::set_node_key(&service, &peer(Some("c1")), &registering("nope\" x"))
            .await
            .is_err()
    );

    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c1").node_key, "");
    assert_eq!(key_a(&entries, "c1").line, before, "a refused key still rewrote the line");
}

/// A Mac is two lines under one id, and only one of them can hold this.
///
/// Key B has no forced command — that is the entire difference between the two
/// shapes — so there is nowhere to put a node key on it. The call must find the
/// restricted line and leave the plain one byte for byte: rewriting Key B would
/// put a forced command on the key Zed, git and Terminal use, which takes the
/// Mac's shell away.
#[tokio::test]
async fn a_macs_plain_line_is_left_exactly_as_it_was() {
    let (service, _home) = runner_with_devices(&["c1"]).await;
    enroll(&service, SHELL_KEY, "c1", true).await;
    let before = entries(&service).await;
    let plain = before.iter().find(|e| e.shell_access).expect("a Key B line").line.clone();

    enrollment::set_node_key(&service, &peer(Some("c1")), &registering(NODE_KEY))
        .await
        .expect("the Mac registers a node key");

    let after = entries(&service).await;
    assert_eq!(key_a(&after, "c1").node_key, NODE_KEY);
    let plain_after = after.iter().find(|e| e.shell_access).expect("the Key B line survives");
    assert_eq!(plain_after.line, plain, "the plain line was rewritten");
    assert!(!plain_after.line.contains("--node-key"), "a node key landed on a shell line");
}

// ---------------------------------------------------------------------------
// Over a real socket, through the real dispatch table.
//
// Everything above proves `enrollment::set_node_key` reads the peer. This
// proves `rpc.rs` hands it the CONNECTION's peer — a dispatch arm that built
// one out of `p.client_id` instead would pass every test above and hand any
// device a route onto any other device's line.
// ---------------------------------------------------------------------------

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

struct Harness {
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
    service: Arc<Service>,
    tmux_socket: String,
}

/// Take the tmux server down with the test that started it. `Service::open_in`
/// inventories tmux on the way up, and a server left behind per run accumulates
/// until new ones stop starting.
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
    let dir = tempfile::tempdir().expect("a scratch root");
    let socket = dir.path().join("farcoolerd.sock");
    let service = Arc::new(open_service(dir.path()).await);
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");
    // Constructed but not run: this is about dispatch, and a sampling loop
    // would make it race a tmux that may not be there.
    let watcher = farcooler_daemon::watch::Watcher::new(service.clone());

    let served = service.clone();
    tokio::spawn(async move {
        let _ = server
            .serve(move |preamble| {
                // The daemon's own answer to "who is this", called rather than
                // reproduced. A copy here would let the very bug this covers
                // pass, because the test would be asserting against its own
                // wiring.
                let peer = peer_from_preamble(preamble.as_ref())?;
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
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

    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, service, tmux_socket }
}

/// Connect, optionally saying which device this is first.
///
/// The preamble is written by hand because it is what a `farcoolerd --stdio`
/// relay actually puts on the socket, and the point is that the client id in it
/// is what decides which line gets written.
async fn connect(h: &Harness, preamble: Option<&str>) -> SocketClient {
    let stream = tokio::net::UnixStream::connect(&h.socket).await.expect("connect");
    let (read, mut write) = stream.into_split();
    if let Some(line) = preamble {
        write.write_all(line.as_bytes()).await.expect("preamble");
    }
    Client::over(read, write, "test-client", "0.0.0").await.expect("handshake")
}

#[tokio::test]
async fn the_dispatch_table_writes_onto_the_connections_own_line() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    // Enrol both devices over the local socket, the way a Mac app does.
    for (index, client_id) in ["c1", "c2"].iter().enumerate() {
        let mut req = request("client.enroll");
        req.payload =
            Some(request::Payload::ClientEnroll(farcooler_protocol::v1::ClientEnroll {
                public_key: KEYS[index].into(),
                label: "device".into(),
                client_id: (*client_id).into(),
                scope: Scope::Control as i32,
                shell_access: false,
            }));
        admin.call(req).await.expect("client.enroll");
    }

    // c1's connection, at `read` — the lowest scope a device is ever given, and
    // enough for this call by design.
    let mut phone = connect(&h, Some("farcooler-session read c1\n")).await;
    let mut req = request("client.set_node_key");
    // The request names c2. The connection is c1.
    req.payload = Some(request::Payload::ClientSetNodeKey(ClientSetNodeKey {
        client_id: "c2".into(),
        node_key: NODE_KEY.into(),
    }));
    let out = phone.call(req).await.expect("client.set_node_key");
    let Some(result::Value::ClientSetNodeKey(_)) = out.value else {
        panic!("client.set_node_key answered with the wrong result type");
    };

    let entries = entries(&h.service).await;
    assert_eq!(key_a(&entries, "c1").node_key, NODE_KEY, "the caller's own line was not written");
    assert_eq!(
        key_a(&entries, "c2").node_key,
        "",
        "the dispatch arm believed the request instead of the connection"
    );
}

/// A local caller holds every scope and still has no line here.
#[tokio::test]
async fn a_local_caller_is_refused_over_the_socket_too() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    let mut req = request("client.enroll");
    req.payload = Some(request::Payload::ClientEnroll(farcooler_protocol::v1::ClientEnroll {
        public_key: KEYS[0].into(),
        label: "device".into(),
        client_id: "c1".into(),
        scope: Scope::Control as i32,
        shell_access: false,
    }));
    admin.call(req).await.expect("client.enroll");

    let mut req = request("client.set_node_key");
    req.payload = Some(request::Payload::ClientSetNodeKey(ClientSetNodeKey {
        client_id: "c1".into(),
        node_key: NODE_KEY.into(),
    }));
    assert!(admin.call(req).await.is_err(), "a caller with no device wrote onto a line");

    let entries = entries(&h.service).await;
    assert_eq!(key_a(&entries, "c1").node_key, "");
}

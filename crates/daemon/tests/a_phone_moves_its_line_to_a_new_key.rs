//! A phone moves its own line to a new key (ov-569, "Make a New Key").
//!
//! The proof is made by the phone's own code (`farcooler_client::device_keys::
//! rekey_proof`), so this also pins the statement the two crates sign and
//! check. The last test runs over a real socket, through the real dispatch
//! table, with a peer built by `peer_from_preamble`: the property that the
//! connection's line moves, not the request's, can be lost in either place.

use std::path::Path;
use std::sync::Arc;

use farcooler_daemon::{enrollment, rpc::RpcFactory, service::Service, sessions::peer_from_preamble};
use farcooler_fence::Entry;
use farcooler_protocol::v1::{ClientEnroll, ClientRekey, Scope, request, result};
use farcooler_transport::{Client, HandshakeConfig, Peer, UnixListenerServer, request};
use ssh_key::private::Ed25519Keypair;
use ssh_key::{LineEnding, PrivateKey};
use tokio::io::AsyncWriteExt;

const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// An OpenSSH private key for a seed, as a phone keeps one.
fn key(seed: u8) -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
}

fn private(seed: u8) -> String {
    key(seed).to_openssh(LineEnding::LF).unwrap().to_string()
}

fn public(seed: u8) -> String {
    key(seed).public_key().to_openssh().unwrap()
}

fn fingerprint(seed: u8) -> String {
    key(seed).public_key().fingerprint(ssh_key::HashAlg::Sha256).to_string()
}

fn client_id(seed: u8) -> String {
    farcooler_client::ceremony::client_id(&public(seed)).unwrap()
}

async fn open_service(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a scratch home");
    farcooler_daemon::service::stub_agents_in_this_process();
    Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"))
}

fn enrolling(seed: u8, name: &str, scope: Scope, node_key: &str) -> ClientEnroll {
    ClientEnroll {
        public_key: public(seed),
        label: name.into(),
        client_id: client_id(seed),
        scope: scope as i32,
        shell_access: false,
        node_key: node_key.into(),
        for_teammate: false,
    }
}

/// The phone's request: its new key, proven by that key over its old one.
fn moving(new_seed: u8, old_seed: u8) -> ClientRekey {
    let made = farcooler_client::device_keys::rekey_proof(&private(new_seed), &public(old_seed)).unwrap();
    ClientRekey { public_key: made.public_key, client_id: made.client_id, proof: made.proof }
}

async fn entries(service: &Service) -> Vec<Entry> {
    let path = service.authorized_keys().to_path_buf();
    tokio::task::spawn_blocking(move || farcooler_fence::read(&path, farcooler_fence::AUTHORIZED_KEYS))
        .await
        .expect("the read task ran")
        .expect("authorized_keys parses")
}

fn line_of<'a>(entries: &'a [Entry], id: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.client_id == id)
}

fn peer(id: &str) -> Peer {
    Peer { client_id: Some(id.into()), scope: Scope::Read }
}

#[tokio::test]
async fn the_phones_line_moves_to_its_new_key_and_nobody_elses_does() {
    let dir = tempfile::tempdir().unwrap();
    let service = open_service(dir.path()).await;
    enrollment::enroll(&service, &enrolling(1, "iPhone", Scope::Control, NODE_KEY)).await.unwrap();
    enrollment::enroll(&service, &enrolling(2, "iPad", Scope::Read, "")).await.unwrap();
    let ipad = line_of(&entries(&service).await, &client_id(2)).unwrap().line.clone();

    let out = enrollment::rekey(&service, &peer(&client_id(1)), &moving(9, 1)).await.expect("moved");
    assert_eq!(out.client.unwrap().client_id, client_id(1), "the line's id changed");

    let after = entries(&service).await;
    let moved = line_of(&after, &client_id(1)).expect("the phone has no line");
    assert_eq!(moved.fingerprint, fingerprint(9), "the line still holds the old key");
    assert_eq!(moved.scope, Scope::Control);
    assert_eq!(moved.node_key, NODE_KEY);
    assert_eq!(farcooler_fence::device_name(&moved.label), Some("iPhone"));
    assert_eq!(line_of(&after, &client_id(2)).unwrap().line, ipad, "another device's line changed");
}

#[tokio::test]
async fn a_proof_for_another_devices_old_key_moves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let service = open_service(dir.path()).await;
    enrollment::enroll(&service, &enrolling(1, "iPhone", Scope::Control, "")).await.unwrap();
    enrollment::enroll(&service, &enrolling(2, "iPad", Scope::Read, "")).await.unwrap();
    let before: Vec<String> = entries(&service).await.into_iter().map(|e| e.line).collect();

    // The iPad's connection, replaying the phone's proof.
    let refused = enrollment::rekey(&service, &peer(&client_id(2)), &moving(9, 1)).await;
    assert!(refused.is_err(), "a proof made over the phone's key moved the iPad's line");
    let after: Vec<String> = entries(&service).await.into_iter().map(|e| e.line).collect();
    assert_eq!(before, after);
}

// Over a real socket, through the real dispatch table.

struct Harness {
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
    service: Arc<Service>,
    tmux_socket: String,
}

impl Drop for Harness {
    fn drop(&mut self) {
        farcooler_tmux::reap_server(&self.tmux_socket);
    }
}

async fn start() -> Harness {
    let dir = tempfile::tempdir().expect("a scratch root");
    let socket = dir.path().join("farcoolerd.sock");
    let service = Arc::new(open_service(dir.path()).await);
    let tmux_socket = service.tmux.socket().to_string();
    let server = UnixListenerServer::bind(&socket).expect("bind");
    let watcher = farcooler_daemon::watch::Watcher::new(service.clone());
    let served = service.clone();
    tokio::spawn(async move {
        let _ = server
            .serve(move |preamble| {
                let peer = peer_from_preamble(preamble.as_ref())?;
                Some((
                    HandshakeConfig { daemon_version: "test".into() },
                    RpcFactory::new(served.clone(), watcher.clone(), Arc::new(tokio::sync::Notify::new()), peer),
                ))
            })
            .await;
    });
    tokio::task::yield_now().await;
    Harness { _dir: dir, socket, service, tmux_socket }
}

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn connect(h: &Harness, preamble: Option<&str>) -> SocketClient {
    let stream = tokio::net::UnixStream::connect(&h.socket).await.expect("connect");
    let (read, mut write) = stream.into_split();
    if let Some(line) = preamble {
        write.write_all(line.as_bytes()).await.expect("preamble");
    }
    Client::over(read, write, "test-client", "0.0.0").await.expect("handshake")
}

#[tokio::test]
async fn a_read_scoped_phone_moves_its_own_line_over_the_wire() {
    let h = start().await;
    let mut admin = connect(&h, None).await;
    for (seed, name, scope) in [(1, "iPhone", Scope::Read), (2, "iPad", Scope::Read)] {
        let mut req = request("client.enroll");
        req.payload = Some(request::Payload::ClientEnroll(enrolling(seed, name, scope, "")));
        admin.call(req).await.expect("client.enroll");
    }

    // The phone's connection, at `read`, the lowest scope a device is given.
    let mut phone = connect(&h, Some(&format!("farcooler-session read {}\n", client_id(1)))).await;
    let mut req = request("client.rekey");
    req.payload = Some(request::Payload::ClientRekey(moving(9, 1)));
    let out = phone.call(req).await.expect("client.rekey");
    let Some(result::Value::ClientRekey(_)) = out.value else {
        panic!("client.rekey answered with the wrong result type");
    };
    let after = entries(&h.service).await;
    let moved = line_of(&after, &client_id(1)).unwrap();
    assert_eq!(moved.fingerprint, fingerprint(9));
    assert_eq!(moved.scope, Scope::Read, "the scope changed");
    assert!(line_of(&after, &client_id(2)).is_some());

    // The iPad's connection can't use a proof made over the phone's old key,
    // whatever line it hoped to move.
    let mut ipad = connect(&h, Some(&format!("farcooler-session read {}\n", client_id(2)))).await;
    let mut req = request("client.rekey");
    req.payload = Some(request::Payload::ClientRekey(moving(8, 1)));
    assert!(ipad.call(req).await.is_err(), "the dispatch arm moved a line that wasn't the connection's");
    assert!(line_of(&entries(&h.service).await, &client_id(2)).is_some());
}

/// Fix round 1, H1: who a device is on this runner rides on its client id, so
/// a member's phone is still that member after a new key, never the owner.
#[tokio::test]
async fn a_members_phone_is_still_theirs_after_a_new_key() {
    let dir = tempfile::tempdir().unwrap();
    let service = open_service(dir.path()).await;
    enrollment::enroll(&service, &enrolling(1, "iPhone", Scope::Control, "")).await.unwrap();
    service.store.set_member("sam", "Sam", &[client_id(1)]).unwrap();

    enrollment::rekey(&service, &peer(&client_id(1)), &moving(9, 1)).await.expect("moved");
    let line = line_of(&entries(&service).await, &client_id(1)).cloned().expect("the phone's line");
    let after = Peer { client_id: Some(line.client_id.clone()), scope: Scope::Control };
    assert_eq!(
        farcooler_daemon::people::person_of(&service.store, &after).unwrap(),
        farcooler_daemon::people::Person::Member("sam".into()),
        "the member's phone became someone else"
    );
}

/// A removed member's phone is nobody's, and gets no new key.
#[tokio::test]
async fn a_released_device_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let service = open_service(dir.path()).await;
    enrollment::enroll(&service, &enrolling(1, "iPhone", Scope::Control, "")).await.unwrap();
    service.store.set_member("sam", "Sam", &[client_id(1)]).unwrap();
    service.store.remove_member("sam").unwrap();
    let before: Vec<String> = entries(&service).await.into_iter().map(|e| e.line).collect();

    assert!(enrollment::rekey(&service, &peer(&client_id(1)), &moving(9, 1)).await.is_err());
    let after: Vec<String> = entries(&service).await.into_iter().map(|e| e.line).collect();
    assert_eq!(before, after);
}

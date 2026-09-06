//! The key a device offers reaches the file sshd reads.
//!
//! This is the break the first end-to-end run on a physical iPhone found. The
//! phone minted a node key, the offer carried it, `usable_node_key` said it was
//! good, and then `enrollment::enroll` rendered the line with a literal `None`:
//! the key was nowhere in `authorized_keys`, `allowlist::from_entries` admitted
//! nobody, and the phone got ten seconds of silence and `no_answer`. Two dials
//! on the same phone and the same build, differing only in that line: 10.1 s to
//! nothing, and 0.5 s to connected once the key was written by hand.
//!
//! **Everything here runs under a plain `cargo test`**, which is a build with
//! no Go archive at all. That is deliberate and it is what makes these
//! assertions worth having: the file is the authority, so what belongs on that
//! line is a question `farcooler_fence` answers with no tunnel anywhere in
//! sight. The tunnel half — the identity, the `serve`, the `allow_add` that
//! must not replace a running server — cannot be seen from here at all, and
//! lives in `a_pairing_gives_the_runner_a_tunnel.rs` under the helper backend,
//! where a tunnel is a process whose commands a test can read.
//!
//! The last test here is the other half of that: a runner whose tunnel cannot
//! start must still pair the device. A `cargo test` build is exactly such a
//! runner, so this is the one place that case is free to assert.

use std::path::Path;

use farcooler_daemon::{enrollment, service::Service};
use farcooler_fence::Entry;
use farcooler_protocol::v1::{ClientEnroll, Scope};

/// Distinct ed25519 public keys, obviously synthetic. Two devices have to be
/// two KEYS: `enroll` refuses a second line for a fingerprint already in the
/// file.
const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA phone";
const SHELL_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER laptop";

/// 32 bytes of X25519 public key as the 43 characters of unpadded base64-URL
/// `usable_node_key` requires. The same synthetic constant the allowlist tests
/// use.
const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// A short scratch root, not a deep one: `Service::open_in` stands up a tmux
/// server keyed off this path, and a socket path has a 108-byte ceiling.
async fn open_service(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a scratch home");
    Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"))
}

fn pairing(public_key: &str, client_id: &str, node_key: &str, shell_access: bool) -> ClientEnroll {
    ClientEnroll {
        public_key: public_key.into(),
        label: "device".into(),
        client_id: client_id.into(),
        scope: if shell_access { Scope::HostAdmin as i32 } else { Scope::Control as i32 },
        shell_access,
        node_key: node_key.into(),
    }
}

/// Every line in the runner's fence, as the parser that decides the allowlist
/// reads it back.
///
/// Through `farcooler_fence` rather than `enrollment::list`, and not by
/// preference: `EnrolledClient` — the wire type `list` answers with — carries
/// no node key at all, because nothing a client displays needs one. The file is
/// where the answer lives, and the file is what `allowlist::from_entries`
/// projects.
async fn entries(service: &Service) -> Vec<Entry> {
    let path = service.authorized_keys().to_path_buf();
    tokio::task::spawn_blocking(move || {
        farcooler_fence::read(&path, farcooler_fence::AUTHORIZED_KEYS)
    })
    .await
    .expect("the read task ran")
    .expect("authorized_keys parses")
}

fn key_a<'a>(entries: &'a [Entry], client_id: &str) -> &'a Entry {
    entries
        .iter()
        .find(|e| e.client_id == client_id && !e.shell_access)
        .unwrap_or_else(|| panic!("no Key A line for {client_id}"))
}

/// The whole of gap one, at the layer it was lost.
///
/// The assertion is on the FILE and on the allowlist projected from it, not on
/// what `enroll` returned: a result that said the right thing while the line
/// said nothing is precisely the state the phone was in for ten seconds.
#[tokio::test]
async fn a_pairing_that_carries_a_node_key_writes_it_onto_the_line() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    enrollment::enroll(&service, &pairing(KEY, "c1", NODE_KEY, false))
        .await
        .expect("the device enrolls");

    let entries = entries(&service).await;
    assert_eq!(
        key_a(&entries, "c1").node_key,
        NODE_KEY,
        "the key the device offered is not on its line: {:?}",
        key_a(&entries, "c1").line
    );
    // And the projection the tunnel is actually built from admits it. The line
    // carrying the right characters is not the claim worth making on its own —
    // `from_entries` is what decides who a runner lets in.
    let allowed = farcooler_daemon::allowlist::from_entries(&entries)
        .expect("a device carrying a node key is admitted");
    assert_eq!(allowed.keys(), [NODE_KEY]);
}

/// A device that offers no key still enrolls, and asks this runner for nothing.
///
/// The other direction, and it is not a formality: `render` refuses a node key
/// `usable_node_key` would refuse, and the empty string is one of those — so a
/// daemon that passed the field through as `Some("")` would refuse every v=1
/// device and every phone whose own mint failed. Absence and an unusable key
/// are different things.
#[tokio::test]
async fn a_pairing_that_carries_no_node_key_enrolls_as_direct() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    let out = enrollment::enroll(&service, &pairing(KEY, "c1", "", false))
        .await
        .expect("a device with no node key still enrolls");
    assert!(out.conn_blob.is_empty(), "a device that asked for no tunnel was given a token");

    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c1").node_key, "", "a node key appeared from nowhere");
    assert!(
        farcooler_daemon::allowlist::from_entries(&entries).is_none(),
        "a runner nobody asked to join built an allowlist"
    );
    // And no identity. "A runner nobody asked to join must still not join" is a
    // statement about a file existing, which is the same thing
    // `allowlist::tunnel_plan` reads.
    assert!(
        !service.tailcat_key().exists(),
        "a pairing that carried no node key gave this runner a tunnel identity"
    );
}

/// A plain line has no forced command, so there is nowhere on it to put a node
/// key — and the request is REFUSED rather than quietly written without one.
///
/// A Mac is two enrollments of one client id, and this is the rule that says
/// which of the two carries the key. Telling a caller "written" about a key
/// that went nowhere would leave a device waiting forever for a tunnel that
/// never admits it.
#[tokio::test]
async fn a_node_key_asked_for_on_a_plain_line_is_refused() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    let out = enrollment::enroll(&service, &pairing(SHELL_KEY, "mac", NODE_KEY, true)).await;
    assert!(out.is_err(), "a plain line was written carrying a node key: {out:?}");
    // Nothing was written at all — `render` refuses before the file is opened,
    // so a refused request does not create a `.ssh` directory or a backup on
    // its way out.
    assert!(
        entries(&service).await.is_empty(),
        "a refused pairing still wrote something"
    );

    // The same Mac's Key A call, which is where the key belongs, is accepted.
    // Without this the assertion above could be satisfied by a daemon that
    // refused every node key there is.
    enrollment::enroll(&service, &pairing(KEY, "mac", NODE_KEY, false))
        .await
        .expect("the Key A line takes the node key");
    assert_eq!(key_a(&entries(&service).await, "mac").node_key, NODE_KEY);
}

/// A tunnel that cannot come up must not fail the pairing.
///
/// A `cargo test` build is a runner whose tunnel cannot come up: no archive,
/// no helper, `serve` refuses every call for any input. The device still pairs,
/// still gets its line, and is answered with no token — which is the ceremony's
/// signal to record it as a direct runner. Somebody who paired a phone against
/// a runner they can reach has lost nothing; somebody whose pairing failed
/// outright has lost the device.
#[tokio::test]
async fn a_runner_whose_tunnel_cannot_start_still_pairs_the_device() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    let out = enrollment::enroll(&service, &pairing(KEY, "c1", NODE_KEY, false))
        .await
        .expect("a runner with no tunnel still enrolls the device");
    assert!(!out.already_enrolled);
    assert!(out.client.is_some(), "the pairing answered with no device");
    assert!(
        out.conn_blob.is_empty(),
        "a runner with no tunnel handed back a token: {:?}",
        out.conn_blob
    );
    // The line is there regardless, so the next boot of a runner that CAN serve
    // admits this device without anybody pairing it again.
    assert_eq!(key_a(&entries(&service).await, "c1").node_key, NODE_KEY);
    // And no identity file, because the stub refuses to create one rather than
    // touching the path. A file here would let `tunnel_plan` past its
    // `NoIdentity` guard on a build that can serve nothing.
    assert!(
        !service.tailcat_key().exists(),
        "a build with no tunnel created an identity file"
    );
}

/// Pairing the same phone twice against the same runner answers with the token
/// again, and writes nothing.
///
/// `already_enrolled` is the ordinary outcome of a ceremony offered a runner
/// the device is already on, and it is not a failure — but a device that was
/// told "already enrolled" and given no token would have no way to reach a
/// runner it can only reach through the tunnel. The line already carries its
/// key, so the runner already admits it, so there is a token to give.
#[tokio::test]
async fn pairing_the_same_device_twice_still_answers_with_the_tunnel() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    enrollment::enroll(&service, &pairing(KEY, "c1", NODE_KEY, false)).await.expect("first");
    let again = enrollment::enroll(&service, &pairing(KEY, "c1", NODE_KEY, false))
        .await
        .expect("second");
    assert!(again.already_enrolled, "a second pairing wrote a second line");
    // One line, not two: sshd takes the first line whose key matches, and two
    // would make "what can this device do" a question about line order.
    let entries = entries(&service).await;
    assert_eq!(entries.len(), 1, "a second pairing added a line: {entries:?}");
    assert_eq!(key_a(&entries, "c1").node_key, NODE_KEY);
}

/// A device whose line carries a DIFFERENT key is not admitted on this one.
///
/// `already_enrolled` writes nothing — that is what makes a re-pairing cheap —
/// so a runner that admitted the offered key anyway would have opened its live
/// server to a key its own `authorized_keys` does not hold, and the next
/// restart would rebuild the allowlist from the file and drop it again with
/// nothing said to anybody. The device is enrolled; it is just not tunneled,
/// and `revoke` then re-pair is the way out.
#[tokio::test]
async fn a_second_pairing_with_a_new_key_is_not_admitted_on_the_old_line() {
    let root = tempfile::tempdir().expect("a scratch root");
    let service = open_service(root.path()).await;

    enrollment::enroll(&service, &pairing(KEY, "c1", "", false)).await.expect("first, direct");
    let again = enrollment::enroll(&service, &pairing(KEY, "c1", NODE_KEY, false))
        .await
        .expect("second, offering a key");

    assert!(again.already_enrolled);
    assert!(
        again.conn_blob.is_empty(),
        "a token was handed back for a key the file does not hold"
    );
    let entries = entries(&service).await;
    assert_eq!(key_a(&entries, "c1").node_key, "", "an already-enrolled line was rewritten");
    assert!(
        farcooler_daemon::allowlist::from_entries(&entries).is_none(),
        "the allowlist grew a key that is not in the file"
    );
}

//! What a pairing does to this runner's tunnel — and, twice as importantly,
//! what it does to everybody else's.
//!
//! Two things are being guarded here and they pull in opposite directions.
//!
//! **A runner with no tunnel must get one.** Nothing in this product created
//! `tailcat.key`, so `allowlist::tunnel_plan` answered `NoIdentity` on every
//! runner there has ever been and no device could be admitted to anything. A
//! pairing that carries a node key is what asks a runner to join the tunnel
//! network, so the runner mints an identity and serves.
//!
//! **A runner that already has one must keep serving it.** `serve` REPLACES
//! the running server and tailcat copies the allowlist at `Start`, so a second
//! pairing that went through `serve` would drop every other device's live
//! tunnel — every phone in the house redialing because somebody paired a
//! laptop. `allow_add` mutates the running server instead.
//!
//! ## Why this file needs the helper backend, and why CI must run it under one
//!
//! A default `cargo test` links no tunnel: `farcooler_tailcat::serve` returns
//! `Err(NoTailcatLinked)` for any input at all, so "did this pairing replace
//! the running server" is not a question that build can answer — every
//! assertion about it would pass whatever the code did. Under `tailcat-helper`
//! the tunnel is a PROCESS, and a fake helper is a shell script that records
//! every command it was given, tagged with its own pid. "Was a second server
//! started" then stops being an inference and becomes two lines in a file with
//! two different numbers in them.
//!
//! That is also why `.github/workflows/ci.yml` names this file explicitly, the
//! same way it names `an_empty_allowlist_starts_no_tunnel.rs` and
//! `revocation_closes_what_it_revoked.rs`. `--workspace` builds default
//! features and would compile every test here away.
#![cfg(feature = "tailcat-helper")]

use std::path::Path;
use std::sync::Mutex;

use farcooler_daemon::{enrollment, service::Service};
use farcooler_protocol::v1::{ClientEnroll, Scope};

const KEYS: [&str; 3] = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA one",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER two",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi three",
];

/// Two node keys that are genuinely two keys. 43 base64 characters carry 258
/// bits, so the last character's low two bits are padding — "…AAA" through
/// "…AAD" all decode to the same 32 bytes — and only "…AAE" moves a bit that
/// is really there. These assertions compare strings and so would pass either
/// way, but "two devices" has to mean two devices to the runner, which decodes
/// them.
const NODE_A: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const NODE_B: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";

/// The token the fake helper answers `blob` with.
const TOKEN: &str = "tc-fake-token";

/// Serialises the tests in this binary.
///
/// Two pieces of process-wide state are in play: `FARCOOLER_TUNNEL_HELPER`,
/// which says which script a helper is, and `farcooler_tailcat`'s own
/// one-helper-per-process slot. A test that starts a tunnel and one that
/// asserts none is running would otherwise take turns failing.
static SERIAL: Mutex<()> = Mutex::new(());

/// A helper that is a shell script.
///
/// It records every command it hears, tagged with its own pid; creates the key
/// file it was started with when asked for an identity, which is what the real
/// helper's `loadOrCreateIdentity` does; and answers `blob` with a token.
///
/// The pid tag is what makes "the running tunnel was not replaced" an
/// observation. Two helpers append to one log, so a test comparing only the
/// command text could not tell one process answering twice from two processes
/// answering once each — which is exactly the difference between admitting a
/// device and dropping everybody's tunnel.
fn fake_helper(dir: &Path, log: &Path) {
    let script = dir.join("fake-tunnel-helper");
    let body = r#"#!/bin/sh
key=$(printf '%s' "$1" | sed 's/^--key=//')
while IFS= read -r line; do
  printf '%s\t%s\n' "$$" "$line" >> __LOG__
  case "$line" in
    identity) : > "$key"; chmod 600 "$key"; echo ok ;;
    blob) echo "ok __TOKEN__" ;;
    *) echo ok ;;
  esac
done
"#
    .replace("__LOG__", &log.display().to_string())
    .replace("__TOKEN__", TOKEN);
    std::fs::write(&script, body).expect("the fake helper is written");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the fake helper is executable");
    // SAFETY: `set_var` is sound only while no other thread reads the
    // environment. Every test here holds `SERIAL`, and nothing else in this
    // binary touches this variable.
    unsafe { std::env::set_var("FARCOOLER_TUNNEL_HELPER", &script) };
}

/// A helper that runs but refuses everything: a runner whose tunnel will not
/// come up.
fn broken_helper(dir: &Path, log: &Path) {
    let script = dir.join("broken-tunnel-helper");
    let body = r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\t%s\n' "$$" "$line" >> __LOG__
  echo 'err 5'
done
"#
    .replace("__LOG__", &log.display().to_string());
    std::fs::write(&script, body).expect("the broken helper is written");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the broken helper is executable");
    unsafe { std::env::set_var("FARCOOLER_TUNNEL_HELPER", &script) };
}

/// What the fake helper heard, as `(pid, command)` in the order it heard it.
/// An empty file — nothing asked yet — reads as no lines rather than an error.
fn heard(log: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let (pid, command) = line.split_once('\t').expect("a pid-tagged line");
            (pid.to_string(), command.to_string())
        })
        .collect()
}

fn commands(log: &Path) -> Vec<String> {
    heard(log).into_iter().map(|(_, c)| c).collect()
}

/// Drops whatever helper this process is holding.
///
/// An allowlist admitting nobody is refused — but `serve` tears the running
/// helper down BEFORE it refuses, on purpose, because that is how a revocation
/// of the last tunneled device stops a server. So this is the supported way to
/// get back to "no tunnel running", and it is the one these tests use rather
/// than reaching into the crate.
fn stop_any_tunnel() {
    let _ = farcooler_tailcat::serve(Path::new("/nonexistent/tailcat.key"), 22, &[]);
}

async fn open_service(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a scratch home");
    Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"))
}

fn pairing(public_key: &str, client_id: &str, node_key: &str) -> ClientEnroll {
    ClientEnroll {
        public_key: public_key.into(),
        label: "device".into(),
        client_id: client_id.into(),
        scope: Scope::Control as i32,
        shell_access: false,
        node_key: node_key.into(),
    }
}

/// The whole sequence, on one runner, in the order a person does it.
///
/// One test rather than three because the state under it is process-wide and
/// each step is only meaningful after the one before: there is no "second
/// pairing does not replace the tunnel" without a first pairing that started
/// one. The steps are labelled so a failure says which one broke.
#[tokio::test]
async fn a_first_pairing_starts_the_tunnel_and_a_second_never_replaces_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    stop_any_tunnel();

    let root = tempfile::tempdir().expect("a scratch root");
    let log = root.path().join("commands");
    fake_helper(root.path(), &log);
    let service = open_service(root.path()).await;

    // ---- Step 1: a fresh runner, and the first phone anybody pairs to it.
    assert!(
        !service.tailcat_key().exists(),
        "the fixture started with an identity, so step 1 proves nothing"
    );
    let first = enrollment::enroll(&service, &pairing(KEYS[0], "c1", NODE_A))
        .await
        .expect("the first device pairs");

    assert!(
        service.tailcat_key().exists(),
        "the first pairing left this runner with no identity, so it can never serve"
    );
    assert_eq!(
        first.conn_blob, TOKEN,
        "the first pairing did not answer with a token: {first:?}"
    );
    assert_eq!(
        commands(&log),
        [
            "identity".to_string(),
            format!("serve 22 {NODE_A}"),
            "blob".to_string(),
        ],
        "step 1: {:?}",
        heard(&log)
    );
    // The identity is created BEFORE the serve, which is the whole point of it
    // being a call of its own: `tunnel_plan` refuses a runner with no key file
    // and refuses before `serve` is reached, so the other order would never
    // serve at all.
    let serving_pid = heard(&log)[1].0.clone();

    // ---- Step 2: a second phone. Nobody else's tunnel may move.
    let second = enrollment::enroll(&service, &pairing(KEYS[1], "c2", NODE_B))
        .await
        .expect("the second device pairs");
    assert_eq!(second.conn_blob, TOKEN, "the second pairing got no token: {second:?}");

    let after = heard(&log);
    assert_eq!(
        commands(&log)[3..],
        [format!("allow {NODE_B}"), "blob".to_string()],
        "step 2: the second pairing did not admit the device to the running tunnel: {after:?}"
    );
    assert_eq!(
        commands(&log).iter().filter(|c| c.starts_with("serve ")).count(),
        1,
        "step 2: the tunnel was served twice, so every other device was dropped: {after:?}"
    );
    assert!(
        after[3..].iter().all(|(pid, _)| *pid == serving_pid),
        "step 2: a second helper process answered, so the first one is gone: {after:?}"
    );

    // ---- Step 3: a device that asks for no tunnel asks this runner for
    // nothing at all. Not merely "gets no token" — the running tunnel must not
    // hear a word about it, because a runner nobody asked to join does not
    // join, and a device with no node key can never be admitted anyway.
    let before = heard(&log).len();
    let direct = enrollment::enroll(&service, &pairing(KEYS[2], "c3", ""))
        .await
        .expect("a device with no node key still pairs");
    assert!(direct.conn_blob.is_empty(), "a device that asked for no tunnel got a token");
    assert_eq!(
        heard(&log).len(),
        before,
        "step 3: a pairing carrying no node key touched the tunnel: {:?}",
        heard(&log)
    );

    stop_any_tunnel();
    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

/// A tunnel that will not come up must not fail the pairing.
///
/// The helper here runs and refuses everything, which is a runner whose network
/// is wrong, whose key file is somebody else's, or whose relay is unreachable.
/// The device still gets its line and pairs as direct: somebody who paired a
/// phone against a runner they can reach has lost nothing, and somebody whose
/// pairing failed outright has lost the device.
#[tokio::test]
async fn a_tunnel_that_will_not_start_still_pairs_the_device() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    stop_any_tunnel();

    let root = tempfile::tempdir().expect("a scratch root");
    let log = root.path().join("commands");
    broken_helper(root.path(), &log);
    let service = open_service(root.path()).await;

    let out = enrollment::enroll(&service, &pairing(KEYS[0], "c1", NODE_A))
        .await
        .expect("a runner whose tunnel refuses still pairs the device");
    assert!(out.client.is_some(), "the pairing answered with no device");
    assert!(out.conn_blob.is_empty(), "a refusing tunnel handed back a token: {out:?}");

    // It was tried, so the assertion above is about a refusal rather than about
    // a path nothing ever walked.
    assert_eq!(commands(&log), ["identity"], "{:?}", heard(&log));
    // And the identity never appeared, so `tunnel_plan` stopped at `NoIdentity`
    // rather than the runner serving something it could not.
    assert!(!service.tailcat_key().exists());

    // The line is in the file regardless, so the next boot of a runner that CAN
    // serve admits this device without anybody pairing it again.
    let path = service.authorized_keys().to_path_buf();
    let entries = tokio::task::spawn_blocking(move || {
        farcooler_fence::read(&path, farcooler_fence::AUTHORIZED_KEYS)
    })
    .await
    .expect("the read task ran")
    .expect("authorized_keys parses");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].node_key, NODE_A, "a failed tunnel cost the device its key");

    stop_any_tunnel();
    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

//! The released devices' sweep and this runner's tunnel (ov-783).
//!
//! A runner that took devices off members' lists before ov-783 still has
//! their lines, each with a node key the tunnel admits. The sweep revokes
//! them all, and two things must hold:
//!
//! - **One rebuild, not one per device.** A rebuild replaces the running
//!   server and drops every other device's live tunnel, so K devices revoked
//!   one at a time would flap everyone's tunnel K times.
//! - **At start, the first tunnel never admits a swept device.** The daemon
//!   sweeps before it starts the tunnel, so the server is built from a file
//!   that no longer has their lines.
//!
//! Under the helper backend, for the reason `a_pairing_gives_the_runner_a_tunnel.rs`
//! gives: a default build links no tunnel, so "how many times was it served,
//! and with what" has the same answer whatever the code does. Here the tunnel
//! is a fake helper that records every command it hears. `ci.yml` names this
//! file for that reason.
#![cfg(feature = "tailcat-helper")]

use std::path::Path;
use std::time::{Duration, Instant};

use farcooler_daemon::{enrollment, release_sweep, service::Service};
use farcooler_protocol::v1::{ClientEnroll, Scope};
use tokio::process::Command;

mod common;
use common::DaemonChild;

const KEYS: [&str; 4] = [
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA one",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBERERERERERERERERERERERERERERERERERERERERER two",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAICIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIi three",
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMz four",
];

/// Four node keys that decode to four keys: the last of 43 base64 characters
/// carries two padding bits, so they differ by a multiple of four (A, E, I, M).
const NODES: [&str; 4] = [
    "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
    "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE",
    "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAI",
    "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM",
];

/// Bob's three devices, and the owner's one that stays.
const BOBS: [&str; 3] = ["d1", "d2", "d3"];
const OWNERS: &str = "d4";

/// Serializes the tests here: the helper's path is in this process's
/// environment, and `farcooler_tailcat` keeps one helper per process.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A helper that records every command it hears and answers each with `ok`,
/// creating its key file when asked for an identity.
fn fake_helper(dir: &Path, log: &Path) {
    let script = dir.join("fake-tunnel-helper");
    let body = r#"#!/bin/sh
key=$(printf '%s' "$1" | sed 's/^--key=//')
while IFS= read -r line; do
  printf '%s\n' "$line" >> __LOG__
  case "$line" in
    identity) : > "$key"; chmod 600 "$key"; echo ok ;;
    blob) echo "ok tc-fake-token" ;;
    *) echo ok ;;
  esac
done
"#
    .replace("__LOG__", &log.display().to_string());
    std::fs::write(&script, body).expect("the fake helper is written");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("the fake helper is executable");
    // SAFETY: every test here holds `SERIAL`, and nothing else in this binary
    // reads this variable.
    unsafe { std::env::set_var("FARCOOLER_TUNNEL_HELPER", &script) };
}

/// The serves the helper heard, one line each.
fn serves(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log).unwrap_or_default().lines().filter(|l| l.starts_with("serve ")).map(str::to_string).collect()
}

/// Drop whatever helper this process holds: an empty allowlist tears it down.
fn stop_any_tunnel() {
    let _ = farcooler_tailcat::serve(Path::new("/nonexistent/tailcat.key"), 22, &[]);
}

/// A runner at `root` with four devices enrolled, each with a node key, and
/// Bob's three taken off his list in the store alone, as a build from before
/// ov-783 left them: released, with their lines still there.
async fn a_runner_with_stale_lines(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a scratch home");
    farcooler_daemon::service::stub_agents_in_this_process();
    let service = Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"));
    for (i, id) in BOBS.iter().chain([&OWNERS]).enumerate() {
        let pairing = ClientEnroll {
            public_key: KEYS[i].into(),
            label: "device".into(),
            client_id: id.to_string(),
            scope: Scope::Control as i32,
            shell_access: false,
            node_key: NODES[i].into(),
            for_teammate: false,
            step_up_key: None,
            teammate: None,
        };
        enrollment::enroll(&service, &pairing).await.expect("the device pairs");
    }
    let bobs: Vec<String> = BOBS.iter().map(|d| d.to_string()).collect();
    service.store.set_member("bob", "Bob", &bobs).expect("Bob bound");
    service.store.set_member("bob", "Bob", &[]).expect("Bob's devices taken off");
    service
}

/// Three devices swept from a running runner: one serve, admitting only the
/// owner's device.
#[tokio::test]
async fn k_swept_devices_rebuild_the_tunnel_once() {
    let _serial = SERIAL.lock().await;
    stop_any_tunnel();
    let root = tempfile::tempdir().expect("a scratch root");
    let log = root.path().join("commands");
    fake_helper(root.path(), &log);
    let service = a_runner_with_stale_lines(root.path()).await;
    assert_eq!(serves(&log).len(), 1, "the fixture's first pairing serves once: {:?}", serves(&log));
    std::fs::write(&log, "").unwrap();

    let swept = release_sweep::sweep(&service).await.expect("the sweep ran");
    assert_eq!(swept.revoked, BOBS.map(String::from), "{swept:?}");
    assert_eq!(serves(&log), [format!("serve 22 {}", NODES[3])], "one rebuild, admitting only the owner's device");

    stop_any_tunnel();
    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

/// A swept device that's connected says its farewell (`device_released`,
/// ov-820) before the one rebuild takes its route: it says it 200 ms after
/// its session is closed, and the tunnel is still as it was then.
#[tokio::test]
async fn the_sweep_waits_for_a_connected_devices_farewell() {
    let _serial = SERIAL.lock().await;
    stop_any_tunnel();
    let root = tempfile::tempdir().expect("a scratch root");
    let log = root.path().join("commands");
    fake_helper(root.path(), &log);
    let service = a_runner_with_stale_lines(root.path()).await;
    std::fs::write(&log, "").unwrap();

    let connected = service.sessions().open(Some(BOBS[1].into()));
    let watched = log.clone();
    let saying = tokio::spawn(async move {
        connected.closed().await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let rebuilt = !serves(&watched).is_empty();
        connected.said_farewell();
        rebuilt
    });
    release_sweep::sweep(&service).await.expect("the sweep ran");

    assert!(!saying.await.unwrap(), "the sweep rebuilt the tunnel before the swept device's farewell was out");
    assert_eq!(serves(&log), [format!("serve 22 {}", NODES[3])], "one rebuild, admitting only the owner's device");

    stop_any_tunnel();
    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

/// The daemon's first tunnel admits none of the devices its start sweeps:
/// the real `farcoolerd`, so it's `main`'s order that's under test.
#[tokio::test]
async fn the_first_tunnel_at_start_never_admits_a_swept_device() {
    let _serial = SERIAL.lock().await;
    stop_any_tunnel();
    let root = tempfile::tempdir().expect("a scratch root");
    let log = root.path().join("commands");
    fake_helper(root.path(), &log);
    let authorized_keys = {
        let service = a_runner_with_stale_lines(root.path()).await;
        service.authorized_keys().to_path_buf()
    };
    stop_any_tunnel();
    std::fs::write(&log, "").unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .env("FARCOOLER_HOME", root.path())
        .env("FARCOOLER_CONFIG", root.path().join("config.toml"))
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .env("HOME", root.path().join("home"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd");
    let _daemon = DaemonChild { child, home: root.path().to_path_buf() };

    let deadline = Instant::now() + Duration::from_secs(30);
    while serves(&log).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(serves(&log), [format!("serve 22 {}", NODES[3])], "the first tunnel admitted a swept device");
    let keys = std::fs::read_to_string(&authorized_keys).unwrap();
    for device in BOBS {
        assert!(!keys.contains(&format!("--client {device} ")), "{device} still has a line:\n{keys}");
    }
    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

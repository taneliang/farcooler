//! `passkey_keys` in the hello follows this runner's sshd (ov-765).
//!
//! A browser's passkey signs as `webauthn-sk-ecdsa-sha2-nistp256@openssh.com`,
//! which OpenSSH verifies from 8.4 on. So the daemon reads `sshd -V` as it
//! starts and offers `passkey_keys` only on 8.4 or newer. These stub the sshd
//! with `FARCOOLER_SSHD`, a script printing the banner a real one prints, and
//! read the hello a client actually gets: through the socket a relayed ssh
//! session reaches, and from a `--stdio` process with no daemon to relay to.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use farcooler_protocol::capability::PASSKEY_KEYS;
use farcooler_transport::Client;

mod common;

/// A stand-in sshd whose `-V` prints `banner` on stderr, exiting as `status`.
/// OpenSSH before 9 has no `-V` and prints the banner above its usage, exiting 1.
fn stub_sshd(dir: &Path, banner: &str, status: i32) -> PathBuf {
    let path = dir.join("sshd");
    std::fs::write(&path, format!("#!/bin/sh\necho 'unknown option -- V' >&2\necho '{banner}' >&2\nexit {status}\n"))
        .expect("write the stub sshd");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make it executable");
    path
}

async fn hello_over_socket(banner: &str, status: i32) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let sshd = stub_sshd(dir.path(), banner, status);
    let _daemon = common::listening_daemon_with_env(dir.path(), &[("FARCOOLER_SSHD", sshd.to_str().unwrap())]).await;
    let client = Client::connect(dir.path().join("farcoolerd.sock"), "test-client", "0.0.0")
        .await
        .expect("handshake on the socket");
    client.server_hello().capabilities.clone()
}

/// Ubuntu 20.04's sshd: no passkeys, so the hello says nothing about them.
#[tokio::test]
async fn an_sshd_older_than_8_4_offers_no_passkeys() {
    let offered = hello_over_socket("OpenSSH_8.2p1 Ubuntu-4ubuntu0.5, OpenSSL 1.1.1f  31 Mar 2020", 1).await;
    assert!(!offered.iter().any(|c| c == PASSKEY_KEYS), "8.2 offered passkeys: {offered:?}");
    // The hello is otherwise whole: this is one name withheld, not a broken table.
    assert!(offered.iter().any(|c| c == farcooler_protocol::capability::ENROLLMENT), "{offered:?}");
}

/// 8.4 is the first that verifies a browser's signatures.
#[tokio::test]
async fn an_sshd_of_8_4_offers_passkeys() {
    let offered = hello_over_socket("OpenSSH_8.4p1 Debian-5+deb11u3, OpenSSL 1.1.1w  11 Sep 2023", 1).await;
    assert!(offered.iter().any(|c| c == PASSKEY_KEYS), "8.4 did not offer passkeys: {offered:?}");
}

/// No sshd to ask is not a yes.
#[tokio::test]
async fn no_sshd_offers_no_passkeys() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-sshd");
    let _daemon =
        common::listening_daemon_with_env(dir.path(), &[("FARCOOLER_SSHD", missing.to_str().unwrap())]).await;
    let client = Client::connect(dir.path().join("farcoolerd.sock"), "test-client", "0.0.0")
        .await
        .expect("handshake on the socket");
    let offered = &client.server_hello().capabilities;
    assert!(!offered.iter().any(|c| c == PASSKEY_KEYS), "{offered:?}");
}

/// A `--stdio` process with no daemon on the socket answers the hello itself,
/// so it asks the sshd too.
#[tokio::test]
async fn a_stdio_session_with_no_daemon_asks_the_sshd_too() {
    for (banner, offers) in [("OpenSSH_8.0p1, OpenSSL 1.1.1k  FIPS 25 Mar 2021", false), ("OpenSSH_10.3p1, LibreSSL 3.3.6", true)] {
        let dir = tempfile::tempdir().expect("tempdir");
        let sshd = stub_sshd(dir.path(), banner, 0);
        let (_child, client) = common::spawn_with_env(dir.path(), &[("FARCOOLER_SSHD", sshd.to_str().unwrap())]).await;
        let offered = &client.server_hello().capabilities;
        assert_eq!(offered.iter().any(|c| c == PASSKEY_KEYS), offers, "{banner}: {offered:?}");
    }
}

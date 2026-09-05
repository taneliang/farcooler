//! The other half of the `ProxyCommand` line the Mac already writes.
//!
//! `apps/macos/Sources/FarCooler/SshConfig.swift` puts
//! `ProxyCommand farcooler runner pipe <id>` in `~/.ssh/config` for a tunneled
//! runner, and `apps/macos/Tests/CeremonyTests/SshConfigTests.swift` guards
//! that half — including that the token never lands in the file. What it
//! cannot guard is that this binary accepts the line it wrote. These tests are
//! that contract, from the other side.
//!
//! The rendering itself is deliberately NOT re-tested in Rust. The `Host` block
//! is composed by the app and only WRITTEN by Rust — `farcooler_client::ffi`'s
//! `ssh_config_write` takes an array of finished lines, and its own comment
//! says "what is shared is the write, not the policy". A second composer here
//! would be a second policy, and the one that goes stale is whichever is not
//! the one being read.
//!
//! Every test points `FARCOOLER_HOME` at a scratch directory. A suite that read
//! whoever ran it's real runner store would be a suite that behaves differently
//! on every machine.

use std::path::Path;

/// A store with one runner and one device key, in the shape `RunnerStore`
/// serializes. Written as literal JSON rather than built through the type,
/// because what this file is testing is the boundary — a caller of the binary
/// gets no access to the type either.
fn store_json(id: &str, reach: &str) -> String {
    format!(r#"{{"node_key":"a-node-key","runners":[{{"id":"{id}","reach":{reach}}}]}}"#)
}

/// Unused with `--features tailcat`, where the two tests that dial are gone.
#[cfg_attr(feature = "tailcat", allow(dead_code))]
fn tunneled(token: &str) -> String {
    format!(r#"{{"kind":"tailcat","token":"{token}"}}"#)
}

const DIRECT: &str = r#"{"kind":"direct","host":"10.0.0.4","port":22}"#;

struct Ran {
    stdout: String,
    stderr: String,
    ok: bool,
}

/// `farcooler runner pipe <id>` against a scratch home.
fn pipe(home: &Path, id: &str) -> Ran {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", home)
        .args(["runner", "pipe", id])
        .output()
        .expect("run farcooler runner pipe");
    Ran {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        ok: out.status.success(),
    }
}

/// A scratch `FARCOOLER_HOME`, with the store owner-only.
///
/// The mode is part of the fixture, not tidiness: the store holds this
/// device's own tunnel private key and the reader refuses one anybody else
/// can read. A test inheriting a 022 umask would write 0644 and get that
/// refusal instead of the answer it is asking about.
fn home_with(store: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    if let Some(store) = store {
        let path = dir.path().join("runners.json");
        std::fs::write(&path, store).expect("write the store");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    dir
}

/// The three words after the binary name are what is already in somebody's
/// `~/.ssh/config`. A rename here does not rewrite that file; it only stops it
/// from working.
#[test]
fn the_spelling_macos_writes_is_the_spelling_this_binary_accepts() {
    let home = home_with(None);
    let ran = pipe(home.path(), "box");
    assert!(
        !ran.stderr.contains("unrecognized subcommand")
            && !ran.stderr.contains("unexpected argument")
            && !ran.stderr.contains("Usage:"),
        "`runner pipe box` was not accepted as a command: {}",
        ran.stderr
    );
}

/// An id the store does not know is named, so whoever reads the message can
/// find the `ProxyCommand` line that produced it.
#[test]
fn an_id_the_store_does_not_know_is_named() {
    let home = home_with(None);
    let ran = pipe(home.path(), "box");
    assert!(!ran.ok, "a runner that does not exist must not exit 0");
    assert!(ran.stderr.contains("box"), "the id is not in the message: {}", ran.stderr);
}

/// A direct runner is refused rather than dialed, and the message says where
/// the mistake is: a `ProxyCommand` on a block that should carry `HostName`.
#[test]
fn a_direct_runner_is_refused_rather_than_dialed() {
    let home = home_with(Some(&store_json("box", DIRECT)));
    let ran = pipe(home.path(), "box");
    assert!(!ran.ok);
    assert!(
        ran.stderr.contains("reached directly"),
        "a direct runner was not refused as one: {}",
        ran.stderr
    );
}

/// The token is resolved from the store and never printed — not on the way out
/// and not in the failure.
///
/// Skipped with `--features tailcat`, which is not the default and not what CI
/// runs: with the archive linked this reaches a real DERP dial for a token
/// nobody minted, and waits on a network timeout to do it.
#[cfg(not(feature = "tailcat"))]
#[test]
fn the_token_reaches_the_dial_and_nothing_else() {
    let home = home_with(Some(&store_json("box", &tunneled("tc-secret"))));
    let ran = pipe(home.path(), "box");

    // A plain build links no archive, so the dial fails with the one error
    // that names what is missing. Reaching that error is the assertion: it is
    // only reachable once the id resolved to a token and a device key.
    assert!(!ran.ok);
    assert!(
        ran.stderr.contains("no tunnel it can dial"),
        "the dial was not reached: {}",
        ran.stderr
    );
    assert!(
        !ran.stdout.contains("tc-secret") && !ran.stderr.contains("tc-secret"),
        "the token was printed.\nstdout: {}\nstderr: {}",
        ran.stdout,
        ran.stderr
    );
}

/// **stdout carries the tunnel and nothing else.**
///
/// This is a `ProxyCommand`: every byte on stdout is read by `ssh` as part of
/// the SSH protocol. One log line, one warning, one friendly "connecting…" and
/// the handshake fails with a message about a bad packet that names nothing
/// real. `main` already sends tracing to stderr for the `--json` commands; this
/// is the case where the same rule stops being about a decode failure and
/// starts being about a transport that cannot work at all.
#[cfg(not(feature = "tailcat"))]
#[test]
fn nothing_but_the_tunnel_ever_reaches_stdout() {
    for (name, store) in [
        ("an unknown runner", None),
        ("a direct runner", Some(store_json("box", DIRECT))),
        ("a failed dial", Some(store_json("box", &tunneled("tc-x")))),
    ] {
        let home = home_with(store.as_deref());
        let ran = pipe(home.path(), "box");
        assert!(
            ran.stdout.is_empty(),
            "{name} put {} bytes on stdout, which ssh would read as SSH: {:?}",
            ran.stdout.len(),
            ran.stdout
        );
    }
}

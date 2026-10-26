//! What a Mac's multiplexed ssh master does across a revoke (ov-818, review S6).
//!
//! A Mac reaching a runner names no identity, so `crates/cli/src/remote.rs`
//! multiplexes it: `ControlMaster=auto`, `ControlPersist=120`. sshd reads
//! `authorized_keys` only when a connection authenticates, and a master that's
//! already up never authenticates again. So after a runner revokes the Mac's key,
//! the next read doesn't dial at all: it opens one more channel on the master,
//! and sshd runs the forced command again, `--client` and all.
//!
//! This file measures what that channel gets, with a real sshd and OpenSSH's own
//! client, the way the Mac's CLI calls it.

use farcooler_protocol::v1::{result, Scope};
use farcooler_transport::{request, Client, ClientError};
use std::path::Path;
use tokio::process::{ChildStdin, ChildStdout, Command};

mod common;
mod real_sshd;
use real_sshd::{start, Runner};

#[tokio::test]
async fn a_new_channel_on_a_master_from_before_the_revoke() {
    let runner = start("mac-b", Scope::Control).await;
    let control = runner.runtime.join("cm");
    let mut admin = Client::connect(runner.runtime.join("farcoolerd.sock"), "test-admin", "0.0.0")
        .await
        .expect("handshake on the runner's own socket");
    admin.call(member_set("bob", &["mac-b"])).await.expect("the Mac bound to Bob");

    // The first read opens the master; it keeps answering for 120 s after.
    let (_first, mut first) = connect(&runner, &control).await;
    first.call(request("member.list")).await.expect("Bob's Mac reads who's here");
    // The runner names the key this Mac came in with (ov-818), which the Mac
    // remembers so a later refusal counts only when it's this key's.
    let host = first.call(request("host.get")).await.expect("host.get");
    let Some(result::Value::Host(host)) = host.value else { panic!("not a host: {host:?}") };
    let key = fingerprint(&runner.identity);
    assert_eq!(host.caller_key_fingerprint, key, "the runner named another key than the one that got in");
    assert!(master_alive(&runner, &control), "no master was left up");

    admin.call(member_set("bob", &[])).await.expect("the Mac taken off Bob's list");
    let outcome = first.call(request("member.list")).await;
    assert!(
        matches!(outcome, Err(ClientError::Closed | ClientError::Codec(_))),
        "the open session outlived the revoke: {outcome:?}"
    );

    // The next read: a fresh `farcooler --host`, which finds the master. sshd
    // serves the channel without reading `authorized_keys`, and the forced
    // command still names `mac-b`: the daemon takes it as the released device it
    // now is (ov-758), which reads `member.list` and nothing else. So a Mac that
    // was connected when it was removed is told so (ov-784's screen) for as long
    // as the master lives, and meets the refused key (ov-818) once it ends.
    assert!(master_alive(&runner, &control), "the master died with the revoke");
    let (_next, mut next) = connect(&runner, &control).await;
    let read = next.call(request("member.list")).await.expect("the released Mac reads member.list");
    let Some(result::Value::MemberList(list)) = read.value else { panic!("not a member list: {read:?}") };
    assert_eq!(list.you, "(released)", "the channel on the old master isn't the released device's");
    let fleet = next.call(request("worktree.list")).await;
    assert!(
        matches!(&fleet, Err(ClientError::Daemon { what, .. }) if what == "not_yours"),
        "a released Mac read the fleet, or was refused for another reason: {fleet:?}"
    );
    stop_master(&runner, &control);
    assert!(!master_alive(&runner, &control), "the master outlived this test");

    // Once the master is gone, the next read dials, and the Mac's verbose
    // probe sees what `KeyRefusals.isProvenKeyRefusal` needs: the key the
    // runner named, offered under a known host key, and refused.
    let probe = probe(&runner);
    assert!(probe.contains("Permission denied (publickey"), "not refused: {probe}");
    assert!(probe.contains("is known and matches the"), "host key not checked: {probe}");
    assert!(
        probe.lines().any(|l| l.contains("Offering public key") && l.contains(&key)),
        "the refused key isn't the one the runner named: {probe}"
    );
    assert!(!probe.contains("Server accepts key"), "the runner took the key: {probe}");
}

/// `ssh-keygen -lf`'s `SHA256:…` for `identity`'s public half.
fn fingerprint(identity: &Path) -> String {
    let out = std::process::Command::new("ssh-keygen")
        .arg("-lf")
        .arg(identity.with_extension("pub"))
        .output()
        .expect("ssh-keygen");
    String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1).expect("a fingerprint").to_string()
}

/// The Mac's probe, `DaemonClient.probeArguments`, against this runner: strict
/// host key checking against the host key this test records first.
fn probe(runner: &Runner) -> String {
    let known = runner.runtime.join("known_hosts");
    let scan = std::process::Command::new("ssh-keyscan")
        .args(["-p", &runner.port.to_string(), "127.0.0.1"])
        .output()
        .expect("ssh-keyscan");
    std::fs::write(&known, &scan.stdout).expect("known_hosts");
    let out = std::process::Command::new("ssh")
        .args(["-F", "/dev/null", "-p", &runner.port.to_string()])
        .arg("-i")
        .arg(&runner.identity)
        .args(["-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none"])
        .args(["-v", "-n", "-o", "BatchMode=yes", "-o", "ControlMaster=no", "-o", "ControlPath=none"])
        .args(["-o", "StrictHostKeyChecking=yes", "-o"])
        .arg(format!("UserKnownHostsFile={}", known.display()))
        .args(["--", "127.0.0.1", "true"])
        .output()
        .expect("ssh");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn member_set(handle: &str, devices: &[&str]) -> farcooler_protocol::v1::Request {
    let mut req = request("member.set");
    req.payload = Some(farcooler_protocol::v1::request::Payload::MemberSet(
        farcooler_protocol::v1::MemberSet {
            handle: handle.into(),
            name: "Bob".into(),
            client_ids: devices.iter().map(|d| d.to_string()).collect(),
            ..Default::default()
        },
    ));
    req
}

/// The options `remote.rs::ssh_args` sends with no identity named, bound to
/// this test's key and host so nothing of this machine's own ssh is read.
fn ssh(runner: &Runner, control: &Path) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-F", "/dev/null", "-p", &runner.port.to_string()])
        .arg("-i")
        .arg(&runner.identity)
        .args(["-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none", "-o", "BatchMode=yes"])
        .args(["-o", "ControlMaster=auto", "-o"])
        .arg(format!("ControlPath={}", control.display()))
        .args(["-o", "ControlPersist=120"])
        .args(["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]);
    command
}

async fn try_connect(runner: &Runner, control: &Path) -> Option<(tokio::process::Child, Client<ChildStdout, ChildStdin>)> {
    let mut child = ssh(runner, control)
        .args(["127.0.0.1", "--", "farcoolerd --stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn ssh");
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let client = tokio::time::timeout(std::time::Duration::from_secs(10), Client::over(stdout, stdin, "test-client", "0.0.0"))
        .await
        .ok()?
        .ok()?;
    Some((child, client))
}

async fn connect(runner: &Runner, control: &Path) -> (tokio::process::Child, Client<ChildStdout, ChildStdin>) {
    try_connect(runner, control).await.expect("handshake over a real sshd")
}

fn master_alive(runner: &Runner, control: &Path) -> bool {
    std::process::Command::new("ssh")
        .args(["-F", "/dev/null", "-p", &runner.port.to_string(), "-o"])
        .arg(format!("ControlPath={}", control.display()))
        .args(["-O", "check", "127.0.0.1"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The master is a client-side process this test started: ended by its own
/// socket, never by pattern.
fn stop_master(runner: &Runner, control: &Path) {
    let _ = std::process::Command::new("ssh")
        .args(["-F", "/dev/null", "-p", &runner.port.to_string(), "-o"])
        .arg(format!("ControlPath={}", control.display()))
        .args(["-O", "exit", "127.0.0.1"])
        .output();
}

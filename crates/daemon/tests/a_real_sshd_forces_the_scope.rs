//! A real sshd, a real fenced `authorized_keys`, and the scope that arrives.
//!
//! Every other test of this drives the daemon by putting `--scope read` on argv
//! itself — `a_relayed_session_keeps_its_scope.rs` through `common::spawn_with`.
//! That proves the daemon HONORS argv. It says nothing about whether sshd
//! DELIVERS that argv from the line `fence::render` wrote, and between the two
//! sits everything that can silently strip a device's identity or hand it the
//! whole runner:
//!
//! - whether `restrict,command="…"` is even syntactically valid to OpenSSH, or
//!   whether sshd refuses the line and the device simply cannot log in;
//! - whether `restrict`'s implied `no-pty` leaves a byte pipe the protocol can
//!   use at all;
//! - whether the connecting device can talk PAST the forced command by asking
//!   for a different scope on its own ssh command line. Nothing short of a real
//!   sshd can answer that one, because the answer is a property of sshd.
//!
//! So the line under test is rendered by `fence::render` and written by
//! `fence::write` — the shipped API, not a hand-rolled string. A hand-rolled
//! line would be this file asserting against its own idea of the format, which
//! is the one thing it must not do: the format is what is on trial.
//!
//! **Per-commit, on both runners.** These tests carry no `#[ignore]`, so
//! `cargo test --workspace` — the last step of `ci.yml`'s `rust` job, which
//! runs on `ubuntu-latest` and `macos-latest` — runs them on every commit,
//! and so does a plain `cargo test` on a developer's machine.
//!
//! They were `#[ignore]`d for four months, with a reason naming a scheduled
//! lane that was never built: `docs/farcooler-design.md` puts the
//! loopback-sshd conformance suite in the same scheduled environment as the
//! authenticated agent tests, and no workflow in `.github/workflows/` has ever
//! had a `schedule:` trigger. So the closest thing this repository has to a
//! security guard ran nowhere, behind a sentence that read as temporary.
//!
//! What the ignore was really protecting against was never named, and turned
//! out not to exist. **A non-root sshd is all this needs**, and it was measured
//! on both platforms before the attribute came off: `restrict`, `command=` and
//! `environment=` behave identically under OpenSSH 10.3p1 on macOS 26.6 and
//! OpenSSH 10.2p1 on Ubuntu 26.04, the forced command overrides the client's own,
//! and `PermitUserEnvironment` reaches it. Privilege separation is the thing
//! that would have needed root, and OpenSSH skips it outright when `getuid()`
//! is not 0 — so nothing here ever wanted a privileged runner. Three tests, a
//! real sshd each, in 0.9 seconds wall clock with `cargo test`'s default
//! parallelism.
//!
//! What a runner must supply is therefore small, and `ci.yml` has a step that
//! makes sure of the only part that is not universal: an `sshd` binary at
//! `/usr/sbin/sshd`. `ssh` and `ssh-keygen` come with any Unix that has a
//! developer toolchain, and the rest is a loopback port.
//!
//! A missing sshd fails loudly rather than skipping — see `real_sshd::start`. A test that
//! detected one and printed "ok" would be a lie told to a release gate, and
//! that half of the original reasoning was right and is unchanged.
//!
//! **A defect this file found.** `fence::render` used to name the daemon as a
//! bare `farcoolerd`, resolved out of whatever PATH the account's login shell
//! hands a non-interactive ssh command — while
//! `crates/cli/src/remote.rs::daemon_command` deliberately wrote
//! `~/.local/bin/farcoolerd-<channel>` and said why in a comment, and
//! `Channel::daemon_binary_name` existed precisely so a preview install could not
//! be handed the stable daemon. Two failure modes, not one: an enrollment onto a
//! runner whose login shell leaves `~/.local/bin` off PATH produced a device that
//! could never connect, and a canary daemon enrolling a device pointed it at the
//! STABLE install. `render` now writes the path and the channel, and
//! `fence`'s own `the_forced_command_names_this_channels_daemon_by_path` pins it.
//!
//! `forced_program` and `path_for_forced_command` in `real_sshd` read the program name out
//! of the RENDERED line rather than writing it down, so this file follows that
//! spelling wherever it goes next instead of pinning today's.

use farcooler_protocol::v1::{result, ErrorCode, Scope};
use farcooler_transport::{request, Client, ClientError};
use tokio::process::{ChildStdin, ChildStdout, Command};

mod common;
mod real_sshd;
use real_sshd::{start, Runner};

/// A read-scoped line in `authorized_keys` produces a read session.
///
/// The whole chain in one assertion pair: `fence::render` wrote the line,
/// OpenSSH parsed it, sshd ran the forced command in it, the daemon read its own
/// argv, and the scope reached the dispatcher. Break any link and this fails.
#[tokio::test]
async fn a_read_scoped_forced_command_yields_a_read_session() {
    let runner = start("phone-7", Scope::Read).await;
    // What the product's own client asks for, so nothing in the request is
    // asking to be demoted. sshd ignores it regardless — see the next test.
    let (_ssh, mut client) = connect(&runner, "farcoolerd --stdio").await;

    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "a read-enrolled key came back as something other than read"
    );

    // And the dispatcher refuses, which is the half the hello cannot see: a
    // hello only REPORTS a scope, and a scope that is reported and not enforced
    // is a settings screen telling somebody a comforting thing.
    match client.call(request("worktree.discover")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(code, ErrorCode::ScopeDenied as i32, "a read session reached host paths");
        }
        other => panic!("a read session through sshd must not discover worktrees: {other:?}"),
    }
}

/// The device cannot ask sshd for a scope its line does not grant.
///
/// This is the security claim of the whole design, and it is the one claim that
/// only a real sshd can settle: the forced command is what makes identity and
/// scope server-asserted, and "forced" is OpenSSH's promise, not ours. A client
/// that could append `--scope host_admin` to its own command line would turn
/// every read-enrolled phone into a host administrator on every runner it can
/// reach, with the fence still saying `read` and nothing anywhere logging a
/// refusal.
///
/// The requested string is not discarded by sshd — it arrives as
/// `SSH_ORIGINAL_COMMAND`, in this session's environment, next to the argv that
/// won. So this also asserts that nothing in the daemon reads that variable.
#[tokio::test]
async fn the_client_cannot_upgrade_its_own_scope() {
    let runner = start("phone-7", Scope::Read).await;
    // Asking for the whole runner, and for somebody else's device id while it is
    // at it. Both are the client's own bytes and neither may be believed.
    let (_ssh, mut client) =
        connect(&runner, "farcoolerd --stdio --scope host_admin --client somebody-else").await;

    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "a client talked past the forced command and chose its own scope"
    );
    match client.call(request("worktree.discover")).await {
        Err(ClientError::Daemon { code, .. }) => {
            let denied = ErrorCode::ScopeDenied as i32;
            assert_eq!(code, denied, "a session that asked for more reached host paths");
        }
        other => panic!("a client that asked for host_admin was given it: {other:?}"),
    }
}

/// The daemon attributes the session to the device the LINE names.
///
/// Asserted through revocation, because revocation is the only thing in the
/// product that observes a live session's client id — `client.list` reports the
/// fence, not who is connected, and `EnrolledClient` has no field for it. That
/// is not a workaround: containment is what the id is FOR, and
/// `sessions::Sessions` exists because sshd reads `authorized_keys` at
/// authentication and never again, so deleting a line does nothing at all to the
/// session that device is holding.
///
/// If the id were lost anywhere along the way — sshd dropping the `--client`
/// word, `requested_session` not reading it, `Session::preamble` not sending it,
/// `peer_from_preamble` not carrying it into `Peer` — the revoke would still
/// succeed, still delete the line, and close nothing. The ssh session would keep
/// answering, which is exactly what this asserts it does not.
///
/// The local caller is checked afterwards on purpose. A daemon that closed every
/// connection on any revoke would pass every other assertion here.
#[tokio::test]
async fn the_daemon_learns_the_client_id_from_the_forced_command() {
    let runner = start("phone-7", Scope::Read).await;
    let (_ssh, mut phone) = connect(&runner, "farcoolerd --stdio").await;
    phone.call(request("client.list")).await.expect("a read session may list clients");

    // The socket, which is this machine's owning user and therefore host_admin —
    // `client.revoke` has always required it. Nothing about this connection names
    // a device, which is also what keeps the revoke below from closing it.
    let mut admin =
        Client::connect(runner.runtime.join("farcoolerd.sock"), "test-admin", "0.0.0")
            .await
            .expect("handshake on the runner's own socket");

    // First that the daemon and sshd are reading ONE file. They are the same file
    // on a real runner, and a test in which they were two would prove nothing
    // about revocation reaching the session sshd authenticated.
    let enrolled = client_ids(&mut admin).await;
    assert!(
        enrolled.contains(&"phone-7".to_string()),
        "the daemon does not read the authorized_keys sshd authenticated against: {enrolled:?}"
    );

    let mut revoke = request("client.revoke");
    revoke.payload = Some(farcooler_protocol::v1::request::Payload::ClientRevoke(
        farcooler_protocol::v1::ClientRevoke { client_id: "phone-7".into() },
    ));
    admin.call(revoke).await.expect("client.revoke");

    let outcome = phone.call(request("client.list")).await;
    assert!(
        matches!(outcome, Err(ClientError::Closed | ClientError::Codec(_))),
        "the session sshd started was not attributed to the device its line names: {outcome:?}"
    );
    admin
        .call(request("client.list"))
        .await
        .expect("the local caller's own connection was closed by its own revoke");
}

/// The devices this runner's fence enrolls, as the daemon reads the file.
async fn client_ids(
    admin: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
) -> Vec<String> {
    let result = admin.call(request("client.list")).await.expect("client.list");
    let Some(result::Value::ClientList(list)) = result.value else { panic!("wrong result") };
    list.items.into_iter().map(|c| c.client_id).collect()
}

/// Speak the protocol to the daemon sshd starts, over ssh's own pipes.
///
/// `requested` is what the CLIENT asks to run, which the forced command
/// overrides — the whole point of the second test in this file. It is passed
/// after `--` for the reason `crates/cli/src/remote.rs::ssh_args` gives: ssh
/// reads a leading `-` as an option wherever it appears.
async fn connect(
    runner: &Runner,
    requested: &str,
) -> (tokio::process::Child, Client<ChildStdout, ChildStdin>) {
    let mut child = Command::new("ssh")
        .args(["-p", &runner.port.to_string()])
        .arg("-i")
        .arg(&runner.identity)
        // What `ssh_args` sends when an identity is named, and for its reasons:
        // bound to this key so an agent's other keys cannot exhaust MaxAuthTries,
        // never multiplexed, never interactive.
        .args(["-o", "IdentitiesOnly=yes"])
        .args(["-o", "ControlMaster=no"])
        .args(["-o", "BatchMode=yes"])
        // A throwaway host key on a throwaway port. Recording it would write into
        // the developer's own known_hosts, and refusing it would fail every run
        // after the first.
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", "UserKnownHostsFile=/dev/null"])
        .arg("127.0.0.1")
        .arg("--")
        .arg(requested)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        // Inherited, exactly as `remote.rs` arranges for a user: ssh's own
        // message and the remote daemon's complaints are the only diagnosis
        // available when a forced command fails to run at all.
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn ssh");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let client = Client::over(stdout, stdin, "test-client", "0.0.0")
        .await
        .expect("handshake over a real sshd");
    (child, client)
}


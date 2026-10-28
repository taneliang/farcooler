//! A device dials by token, lands on a real sshd, and gets the scope the file
//! gave it — over a real DERP session, with nothing mocked in between.
//!
//! Until this file there was no run, anywhere, in which the three parts of this
//! transport had ever met. The Go shim's own suite proves what `tailcat.go`
//! does with a `net.Pipe`; the daemon's tunnel tests prove which commands are
//! sent to a helper that is a shell script recording them; `linked.rs` — the
//! only backend that will ever run in production — was type-checked by a
//! `cargo check` and executed by nothing except one minting test. Every one of
//! those is a component agreeing with itself. What was missing is the sentence
//! this file asserts: **the token a runner mints, the node key its
//! `authorized_keys` admits, and the sshd its tunnel forwards to are the same
//! three facts.** A mock of any of them would have agreed with whatever this
//! code believed.
//!
//! The precedent is `a_real_sshd_forces_the_scope.rs`, which stands up a real
//! sshd rather than a mock because "forced" is OpenSSH's promise and not ours.
//! Same argument, one layer out: DERP rendezvous is tailscale's promise, the
//! allowlist is tailcat's, and neither can be settled by a test double.
//!
//! ## What is real here, and the one thing that is not
//!
//! Real: the relay (a `derper` built from the same `tailscale.com` the tunnel
//! pins), the runner's tunnel server and the device's dial (both the linked Go
//! archive, in this process), `authorized_keys` written by `fence::render` and
//! `fence::write`, the allowlist derived by the shipped
//! `allowlist::tunnel_plan`, a real `sshd`, a real forced command, and a real
//! `farcoolerd --stdio` answering the real protocol.
//!
//! Not real: **the port.** `Service::ssh_port()` answers 22 and says in as many
//! words that this is honest rather than a placeholder — nothing in the daemon
//! reads `sshd_config`. So driving this through `allowlist::start_tunnel` would
//! forward the tunnel to `127.0.0.1:22`, which on a developer's machine is that
//! developer's own sshd and on nobody's machine is this test's. This file
//! therefore performs `start_tunnel`'s three steps itself — read the fence,
//! `tunnel_plan`, `serve` — with the scratch sshd's port in place of the
//! constant, and asserts on `tunnel_plan`'s answer so the admission decision is
//! still the shipped one. The substitution is one integer and it is named here
//! so no reader has to discover it.
//!
//! ## Why it is not run by `cargo test --workspace`
//!
//! The whole file is `cfg`'d on `tailcat`, the daemon's mirror of
//! `farcooler-tailcat/linked`, because with no archive linked
//! `farcooler_tailcat::serve` and `dial` refuse every call for every input —
//! every assertion below would pass, or fail, for reasons that have nothing to
//! do with what it is asking. Running it needs three things a default build
//! does not have: the Go archive linked into this test binary, a `derper` on
//! disk, and `TS_DEBUG_USE_DERP_HTTP=1` in the environment at exec.
//! `scripts/tunnel-e2e.sh` supplies all three and is the only supported way to
//! run this; every one of the three is a loud failure when absent and none of
//! them is a skip.
//!
//! **No CI line runs this file.** `ci.yml` type-checks it — `cargo check -p
//! farcooler-daemon --features tailcat --all-targets`, so it cannot rot — and
//! runs nothing, because a job would need a Go toolchain, a built relay and an
//! Apple silicon runner. That is written out at the `cargo check` line itself
//! and in the script's header, including the one question that has to be
//! answered before such a job can be added.

#![cfg(feature = "tailcat")]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use farcooler_client::ssh::{Destination, HostKeyPolicy, Reach, Session, SshError};
use farcooler_daemon::allowlist;
use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::{ErrorCode, Scope};
use farcooler_tailcat::NodeKeyPair;
use farcooler_transport::{request, Client, ClientError};

mod common;
#[path = "support/derper.rs"]
mod derper;

/// One tunnel server per PROCESS, so one test at a time.
///
/// `farcooler_tailcat::serve` replaces whatever is running — that is what makes
/// it a revoke as well as a start — and `set_derp_map_url` is process-wide for
/// the same reason. Two of these tests running concurrently would take turns
/// tearing down each other's runner, and the symptom would be a dial that times
/// out, which is also what a genuinely broken allowlist looks like.
static SERIAL: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// A read-scoped device reaches the daemon through the tunnel, and the scope
/// that arrives is the file's.
///
/// The chain, and every link of it can break independently: `fence::render`
/// wrote a line carrying a node key and a scope; `allowlist::tunnel_plan` read
/// that node key back out and admitted it; tailcat registered the runner with a
/// relay and minted a token naming it; the device dialed that token under its
/// own node key and was recognized; the runner's `OnTCP` forwarded to the sshd
/// on the port it was told; sshd authenticated the device's SSH key against the
/// same file; the forced command in that line ran `farcoolerd --stdio` with the
/// scope in its argv; and the daemon enforced it.
///
/// The host key is PINNED rather than accepted, and that is not decoration: it
/// is what says the bytes ended up at this test's own sshd. A tunnel that
/// terminated anywhere else, or a forward that reached some other listener,
/// presents a different key and this refuses to authenticate at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_reaching_through_the_tunnel_gets_the_scope_in_the_file() {
    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let runner = Runner::start(&relay, &[&phone]).await;

    let mut session = reach(&runner, &phone).await;

    let mut client = speak(&mut session).await;
    assert_eq!(
        client.server_hello().granted_scope,
        Scope::Read as i32,
        "the tunnel changed the scope the file granted"
    );

    // The scope is the file's, not the connection's. A read device must be
    // refused a host path through the tunnel exactly as it is over a direct
    // connection — a transport that quietly widened what a session may do
    // would pass every assertion above.
    match client.call(request("worktree.discover")).await {
        Err(ClientError::Daemon { code, .. }) => {
            assert_eq!(
                code,
                ErrorCode::ScopeDenied as i32,
                "a read device reached host paths through the tunnel"
            );
        }
        other => panic!("a read device wrote through the tunnel: {other:?}"),
    }
}

/// A device whose node key left `authorized_keys` cannot reach sshd at all,
/// while the device still in the file can.
///
/// **The second half is the whole point.** Tailcat ignores an unrecognized
/// client silently, so a revoked device gets a timeout rather than a refusal —
/// and a timeout is also what a dead relay, a broken token, a crashed runner
/// and a test that never started anything produce. Asserting only that the
/// revoked phone times out would be a check that cannot fail: it would go green
/// on a machine where the tunnel never worked at all. So a laptop that is still
/// in the file dials the same token through the same relay in the same test,
/// and has to get through.
///
/// The revoke here is what `enrollment::revoke` does — rewrite the fence, then
/// make the running server match the file — because tailcat can only replace an
/// allowlist, never subtract from one. See `allowlist::start_tunnel`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_does_not_reach_sshd_through_the_tunnel() {
    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let laptop = Device::mint("laptop-2", Scope::Read);
    let mut runner = Runner::start(&relay, &[&phone, &laptop]).await;

    // Both devices reach the runner before anything is revoked, so what the
    // revoke changes is observed rather than assumed.
    reach(&runner, &phone).await;

    runner.revoke(&phone, &[&laptop]).await;

    // **The control runs first, deliberately.** The revoked device's dial can
    // only prove something once the replacement server is up and registered
    // with the relay, and the laptop getting through is what says it is. Run
    // the other way round, a phone that failed because nothing was serving yet
    // would read exactly like a phone that was correctly refused.
    reach(&runner, &laptop).await;

    let began = Instant::now();
    // `Session` carries a live russh handle and no `Debug`, so the success arm
    // is named rather than unwrapped. One attempt and no patience: a revoked
    // device must fail against a runner that has already been shown to be
    // answering.
    let Err(error) = open(&runner, &phone).await else {
        panic!("a revoked device reached sshd through the tunnel");
    };
    derper::note(&format!("the revoked dial failed in {:?}: {error}", began.elapsed()));
    assert!(
        matches!(error, SshError::Tunnel { code: "no_answer" }),
        "a revoked device's dial failed for the wrong reason: {error:?}"
    );
}

/// Revoking one device CLOSES every other device's live session at once, so
/// it redials, instead of leaving it to hang until the ssh keepalive gives up.
///
/// A revoke rebuilds the runner's tunnel server, because tailcat can only
/// replace an allowlist; the new server has none of the old one's WireGuard
/// peers or TCP connections. Before ov-762 the old server was simply closed,
/// so nothing told a surviving device: its session kept the old tunnel, every
/// write went unanswered, and it learned only from the keepalive (30 s × 3 in
/// `Session::open`). Measured here before the fix: the laptop's next call
/// failed after 118.9 s. What has to hold is that the laptop's call fails
/// within `A_DROP_IS_SEEN_WITHIN` — the runner sends it a FIN — and that
/// redialing then gets through, which is the phones' reconnect.
///
/// The bound is asserted against a call made AFTER the revoke has returned,
/// so it measures what a person would see on their next keystroke.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoking_one_device_closes_the_others_sessions_instead_of_hanging_them() {
    /// Seconds, not the keepalive's minutes. The FIN rides the same relay as
    /// every keystroke, so on loopback this is milliseconds; the margin is for
    /// a loaded machine, not for a design that polls.
    const A_DROP_IS_SEEN_WITHIN: Duration = Duration::from_secs(5);
    /// How long to watch before calling it a hang. Longer than the keepalive's
    /// 90 s so the unfixed number is measured rather than truncated.
    const WATCH: Duration = Duration::from_secs(150);

    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let laptop = Device::mint("laptop-2", Scope::Read);
    let mut runner = Runner::start(&relay, &[&phone, &laptop]).await;

    // Two live sessions, each shown to answer, before anything is revoked.
    let mut phone_session = reach(&runner, &phone).await;
    let mut phone_client = speak(&mut phone_session).await;
    phone_client.call(request("client.list")).await.expect("the phone's session answers");
    let mut laptop_session = reach(&runner, &laptop).await;
    let mut laptop_client = speak(&mut laptop_session).await;
    laptop_client.call(request("client.list")).await.expect("the laptop's session answers");

    runner.revoke(&phone, &[&laptop]).await;

    let began = Instant::now();
    let answer = tokio::time::timeout(WATCH, laptop_client.call(request("client.list"))).await;
    let waited = began.elapsed();
    derper::note(&format!("after the revoke the laptop's call ended in {waited:?}: {answer:?}"));
    match answer {
        Err(_) => panic!("the laptop's session hung for {WATCH:?} after another device was revoked"),
        Ok(Ok(_)) => panic!(
            "the laptop's call was answered over a tunnel that was rebuilt underneath it, \
             which this test does not expect: a rebuilt server has no TCP state"
        ),
        Ok(Err(error)) => assert!(
            waited < A_DROP_IS_SEEN_WITHIN,
            "the laptop learned its session was gone only after {waited:?} ({error}); \
             a revoke must close a surviving session, not leave it to the keepalive"
        ),
    }
    drop(laptop_client);
    drop(laptop_session);
    drop(phone_client);
    drop(phone_session);

    // The reconnect: the surviving device dials again and is admitted.
    reach(&runner, &laptop).await;
}

/// A device that reaches its runner only through Tailcat hears that it was
/// revoked (ov-820), over the tunnel the revocation then takes away.
///
/// Before ov-820 it couldn't: the rebuilt tunnel ignores its node key in
/// silence, so it read `no_answer`, a runner that is asleep. Now the runner's
/// session says `device_released` as its last frame, and `revoke` lets it
/// through before the rebuild. Everything is the shipped path: the forced
/// command's `farcoolerd --stdio` relays into this runner's daemon (in this
/// process, on `FARCOOLER_HOME`'s socket), and `enrollment::revoke` closes
/// the phone's session and rebuilds THIS process's tunnel, as a runner's does.
/// The rebuild serves port 22, `Service::ssh_port`; nothing dials it after.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_revoked_device_hears_it_was_released_through_the_tunnel() {
    use farcooler_protocol::v1::event::Payload;
    let _serial = SERIAL.lock().await;
    let relay = derper::start().await;
    let phone = Device::mint("phone-7", Scope::Read);
    let laptop = Device::mint("laptop-2", Scope::Read);
    let runner = Runner::start(&relay, &[&phone, &laptop]).await;
    let service = runner.daemon().await;

    let mut session = reach(&runner, &phone).await;
    let mut client = speak(&mut session).await;
    client.call(request("daemon.version")).await.expect("the phone's session answers through the daemon");

    let began = Instant::now();
    farcooler_daemon::enrollment::revoke(
        &service,
        &farcooler_protocol::v1::ClientRevoke { client_id: phone.client_id.clone(), with_added: false },
    )
    .await
    .expect("client.revoke");
    derper::note(&format!("the revoke, tunnel rebuild included, took {:?}", began.elapsed()));

    let mut heard = false;
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(15), client.next_event()).await {
        heard |= matches!(event.payload, Some(Payload::DeviceReleased(_)));
        if heard {
            break;
        }
    }
    assert!(heard, "the revoked phone never heard it was released, over the tunnel it reached the runner by");
}

/// The cases that need every byte to go through the relay (ov-781).
///
/// Two peers on one machine find a direct UDP path within a second, and then
/// a frozen relay freezes nothing: measured, the first run of the test below
/// had its call answered 2 ms after the freeze. `TS_DEBUG_ALWAYS_USE_DERP=1`
/// is upstream's switch for relay-only, and like `TS_DEBUG_USE_DERP_HTTP` Go
/// reads it once at startup, so it cannot be set for one test inside a
/// process the others share. `scripts/tunnel-e2e.sh` runs this module as a
/// second pass with it set, and the rest of the file without it, unchanged.
mod over_the_relay_only {
    use super::*;

    /// Refuse to run without the switch, rather than prove nothing.
    fn require_relay_only() {
        assert!(
            std::env::var("TS_DEBUG_ALWAYS_USE_DERP").is_ok_and(|v| v == "1"),
            "TS_DEBUG_ALWAYS_USE_DERP=1 must be in the environment at exec, or the peers go \
             direct and freezing the relay freezes nothing; scripts/tunnel-e2e.sh sets it for \
             this module"
        );
    }

    /// A tunnel that dies without a word is noticed in seconds, and the device
    /// dials again as soon as the relay is back (ov-781).
    ///
    /// The revoke above ends sessions with a FIN; this is the case nothing
    /// announces: a runner that crashes or restarts, or a relay that drops. The
    /// device's TCP lives inside the tunnel, so no reset ever reaches it, and
    /// before ov-781 its session hung until the 30 s keepalive had gone
    /// unanswered three times: about two minutes.
    ///
    /// **The relay is frozen, not the runner killed.** The runner's tunnel is in
    /// this test's own process, so it cannot crash without the test. A SIGSTOP on
    /// the relay is the same silence to the device: every connection stays open
    /// and no byte moves on any of them, which is what a crashed runner behind a
    /// live relay, or a relay that vanished, looks like from the phone.
    ///
    /// The bound is on the call made just after the freeze, so it measures what
    /// a person would see: the session ending, which is what the apps redial on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_tunnel_that_dies_without_a_word_is_noticed_in_seconds() {
        /// The card's number. The tunnel's keepalive gives up nine seconds after
        /// the last byte it received (`Liveness::TUNNEL`).
        const NOTICED_WITHIN: Duration = Duration::from_secs(10);
        /// Past the old keepalive's two minutes, so the unfixed number is
        /// measured rather than truncated.
        const WATCH: Duration = Duration::from_secs(150);

        require_relay_only();
        let _serial = SERIAL.lock().await;
        let relay = derper::start().await;
        let phone = Device::mint("phone-7", Scope::Read);
        let runner = Runner::start(&relay, &[&phone]).await;

        let mut session = reach(&runner, &phone).await;
        let mut client = speak(&mut session).await;
        client.call(request("client.list")).await.expect("the session answers before the drop");

        relay.freeze();
        let began = Instant::now();
        let answer = tokio::time::timeout(WATCH, client.call(request("client.list"))).await;
        let noticed = began.elapsed();
        derper::note(&format!("with the relay frozen the call ended in {noticed:?}: {answer:?}"));
        match answer {
            Err(_) => {
                relay.thaw();
                panic!("the session was still hanging {WATCH:?} after its tunnel died")
            }
            Ok(Ok(_)) => {
                relay.thaw();
                panic!(
                    "the call was answered with the relay frozen, so this run's bytes did not \
                     go through the relay and the test proved nothing"
                )
            }
            Ok(Err(error)) => {
                relay.thaw();
                assert!(
                    matches!(error, ClientError::Closed),
                    "the session ended with {error}, not as a closed link the apps redial on"
                );
                assert!(
                    noticed < NOTICED_WITHIN,
                    "a dead tunnel went unnoticed for {noticed:?}; the redial must start within \
                     {NOTICED_WITHIN:?}"
                );
            }
        }
        drop(client);
        drop(session);

        // The redial, as an app makes it the moment the session ends: no wait
        // for the runner to register, because it never left the relay.
        let redial = Instant::now();
        let deadline = redial + Duration::from_secs(60);
        let mut attempt = 0;
        let mut session = loop {
            attempt += 1;
            match open(&runner, &phone).await {
                Ok(session) => break session,
                Err(SshError::Tunnel { code: "no_answer" }) if Instant::now() < deadline => {}
                Err(other) => panic!("the redial failed on attempt {attempt}: {other}"),
            }
        };
        let mut client = speak(&mut session).await;
        client.call(request("client.list")).await.expect("the redialed session answers");
        derper::note(&format!(
            "the redial answered {:?} after the relay came back, on attempt {attempt}; \
             {:?} from the drop to a working session",
            redial.elapsed(),
            began.elapsed()
        ));
    }

    /// What the tunnel's keepalive costs an idle device on the relay, per minute.
    ///
    /// A measurement for the report, not a check, so it is `#[ignore]`d:
    /// `./scripts/tunnel-e2e.sh an_idle_tunnel --ignored` runs it. It reads the
    /// relay's own byte counters (`nettop`, by its pid) across a minute with an
    /// idle session and a minute with none, on the same relay and runner. The
    /// difference is the session's keepalive, as DERP payload: the relay's TCP
    /// and IP headers, and TLS (this relay is plain HTTP), are on top.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "a measurement; run it by name with --ignored"]
    async fn an_idle_tunnel_minute_on_the_relay() {
        const MINUTE: Duration = Duration::from_secs(60);

        require_relay_only();
        let _serial = SERIAL.lock().await;
        let relay = derper::start().await;
        let phone = Device::mint("phone-7", Scope::Read);
        let runner = Runner::start(&relay, &[&phone]).await;

        let mut session = reach(&runner, &phone).await;
        let mut client = speak(&mut session).await;
        client.call(request("client.list")).await.expect("the session answers");
        tokio::time::sleep(Duration::from_secs(5)).await;

        let before = relay_bytes(relay.pid()).await;
        tokio::time::sleep(MINUTE).await;
        let idle = relay_bytes(relay.pid()).await;
        drop(client);
        drop(session);
        tokio::time::sleep(Duration::from_secs(5)).await;
        let quiet = relay_bytes(relay.pid()).await;
        tokio::time::sleep(MINUTE).await;
        let after = relay_bytes(relay.pid()).await;

        let with = (idle.0 - before.0, idle.1 - before.1);
        let without = (after.0 - quiet.0, after.1 - quiet.1);
        derper::note(&format!(
            "the relay carried {} in and {} out in an idle session's minute, {} and {} in a \
             minute with no session: the keepalive is {} in and {} out",
            with.0,
            with.1,
            without.0,
            without.1,
            with.0 as i64 - without.0 as i64,
            with.1 as i64 - without.1 as i64
        ));
    }

    /// The relay's bytes in and out so far, from `nettop`.
    async fn relay_bytes(pid: u32) -> (u64, u64) {
        let out = tokio::process::Command::new("/usr/bin/nettop")
            .args(["-P", "-x", "-L", "1", "-J", "bytes_in,bytes_out", "-p", &pid.to_string()])
            .output()
            .await
            .expect("run nettop");
        let text = String::from_utf8_lossy(&out.stdout);
        let row = text
            .lines()
            .find(|line| line.contains(&format!(".{pid},")))
            .unwrap_or_else(|| panic!("no row for the relay in nettop's output: {text}"));
        let fields: Vec<&str> = row.split(',').collect();
        (fields[1].parse().expect("bytes_in"), fields[2].parse().expect("bytes_out"))
    }
}

/// A Mac's `farcooler runner pipe` reaches the runner's sshd through the
/// tunnel, from a `runners.json` the shipped commands wrote.
///
/// The phones' dial is covered above through `Session::open`. This is the other
/// client: the CLI binary a `ProxyCommand` starts, which has its own store, its
/// own key file and its own pump, and which until this test had never carried a
/// byte over a real tunnel (phase 2 of ov-481). What is real: the binary
/// (`FARCOOLER_CLI_BIN`, built WITH the archive by `scripts/tunnel-e2e.sh`), the
/// key it stores through `runner store-node-key`, the node key the runner
/// admits, the relay, and the sshd. What the test writes by hand is the
/// `runners` list in `runners.json` — written by `runner store-runners`, the
/// command the Mac app runs on grant. No DERP
/// map is given to the CLI: the token names the relay its runner registered
/// with, which is also why a Mac needs no rendezvous setting of its own to dial.
///
/// What arrives is sshd's banner. That proves the dial, the allowlist and the
/// forward (a wrong key, token or relay gets silence, then an exit), and it
/// proves `pump` carries the runner's bytes to stdout. The test after this one
/// runs a real SSH session through the same pipe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cli_pipe_carries_sshd_to_stdout_for_a_stored_node_key() {
    let _serial = SERIAL.lock().await;
    let cli = std::env::var("FARCOOLER_CLI_BIN").expect(
        "FARCOOLER_CLI_BIN names a `farcooler` built with the tunnel; scripts/tunnel-e2e.sh sets it",
    );
    let relay = derper::start().await;
    let mac = Device::mint("mac-1", Scope::Read);
    let runner = Runner::start(&relay, &[&mac]).await;

    let home = tempfile::tempdir().expect("a scratch FARCOOLER_HOME for the Mac");
    store_node_key(&cli, home.path(), &mac.node.private_key);
    store_runners(&cli, home.path(), "box", &runner.token, &runner.host_fingerprint);

    // Same patience as `reach`, for the same measured reason: the runner is
    // not findable for ~3 s after `serve`. Each attempt is a fresh process, as
    // each `ssh` would start one.
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let banner = pipe_banner(&cli, home.path(), "box").await;
        if banner.starts_with("SSH-2.0-") {
            derper::note(&format!("the CLI pipe carried sshd's banner: {}", banner.trim()));
            return;
        }
        assert!(
            Instant::now() < deadline,
            "`farcooler runner pipe box` never carried sshd's banner; last output: {banner:?}"
        );
    }
}

/// A real `ssh`, reading a `Host` block of the shape the Mac app writes, reaches
/// the daemon through the tunnel and is given the scope in the file.
///
/// The whole of phase 3 in one run: `store-node-key` and `store-runners` (what
/// the app runs on grant) fill `runners.json`; `ssh` reads the block, runs the
/// `ProxyCommand` it names, which dials the token under the stored node key;
/// sshd authenticates the device's key; the forced command runs `farcoolerd
/// --stdio`; and the protocol handshake reports the scope the line carries.
///
/// The block is spelled below the way the app spells it, and the host key is
/// pinned (`KnownHostsCommand`), not learned: the same runner under a wrong pin
/// must be refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_real_ssh_reading_the_generated_block_reaches_the_daemon() {
    let _serial = SERIAL.lock().await;
    let cli = std::env::var("FARCOOLER_CLI_BIN").expect(
        "FARCOOLER_CLI_BIN names a `farcooler` built with the tunnel; scripts/tunnel-e2e.sh sets it",
    );
    let relay = derper::start().await;
    let mac = Device::mint("mac-1", Scope::Read);
    let runner = Runner::start(&relay, &[&mac]).await;

    let home = tempfile::tempdir().expect("a scratch FARCOOLER_HOME for the Mac");
    store_node_key(&cli, home.path(), &mac.node.private_key);
    // The same runner twice: once pinned to the key its sshd really has, once
    // pinned to a key it does not. Two ids, so two entries in the store.
    let wrong_pin = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    assert_ne!(runner.host_fingerprint, wrong_pin);
    store_runners(&cli, home.path(), "fc-good", &runner.token, &runner.host_fingerprint);
    store_runners(&cli, home.path(), "fc-wrong", &runner.token, wrong_pin);

    // The bundle's path has a space in it, as the real one does.
    let bundle = home.path().join("Far Cooler.app");
    std::fs::create_dir_all(&bundle).expect("a bundle directory");
    std::os::unix::fs::symlink(&cli, bundle.join("farcooler")).expect("link the CLI in");
    let program = format!("'{}'", bundle.join("farcooler").display());
    let identity = home.path().join("id");
    std::fs::write(&identity, &mac.ssh_private).expect("the device's ssh key");
    std::fs::set_permissions(&identity, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    // The block the Mac app writes, from the file the app's own test pins
    // (`TunnelJoinTests.theE2EsBlockIsTheOneTheAppWrites`): only the markers are
    // filled in. A spelling made up here is how a `%d` that real ssh refuses got
    // past this test once.
    let golden = include_str!("fixtures/app-ssh-block.txt");
    let block = |host: &str, id: &str| {
        golden
            .replace("@PROGRAM@", &program)
            .replace("@ALIAS@", host)
            .replace("@ID@", id)
            .replace("@USER@", &unix_user())
            .replace("/@IDENTITY@", &identity.display().to_string())
    };
    let config = home.path().join("config");
    std::fs::write(&config, block("cosmo", "fc-good") + &block("cosmo-wrong", "fc-wrong"))
        .expect("the ssh config");

    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut attempts = 0;
    let granted = loop {
        attempts += 1;
        match ssh_hello(&config, home.path(), "cosmo").await {
            Ok(scope) => break scope,
            Err(why) => assert!(
                Instant::now() < deadline,
                "`ssh cosmo` never reached the daemon (attempt {attempts}); last: {why}"
            ),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    assert_eq!(granted, Scope::Read as i32, "the scope ssh was given is not the file's");

    // R-48: the same runner under a pin that is not its sshd's key is refused,
    // every time, though the tunnel, the node key and the device key are all fine.
    for _ in 0..3 {
        let refused = ssh_hello(&config, home.path(), "cosmo-wrong").await;
        assert!(refused.is_err(), "ssh trusted a host key that was not the pinned one");
    }
    derper::note(&format!(
        "ssh read the generated block and reached the daemon ({attempts} attempts); a wrong pin was refused"
    ));
}

/// The Mac app's conversation view reaches the daemon through the tunnel the way
/// `ssh` does: the core spawns the `ProxyCommand` the app's block names
/// (ov-496), pinned to the host key the grant carried.
///
/// Nothing here is spelled by hand that the app spells. The block is the golden
/// one the app's own test pins; a real `ssh -G` reads it and says what its
/// `ProxyCommand` is; that command is what `Reach::Pipe` spawns; and the pin
/// comes from `runner pinned-key`, the command the app asks, not from the
/// runner. `Session::open` is the call the app's core makes for `farcooler_client_connect`.
/// Then the same runner under a wrong pin must be refused: the key the session
/// is shown is checked against the grant's, not learned.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mac_core_spawning_the_blocks_proxy_command_reaches_the_daemon() {
    let _serial = SERIAL.lock().await;
    let cli = std::env::var("FARCOOLER_CLI_BIN").expect(
        "FARCOOLER_CLI_BIN names a `farcooler` built with the tunnel; scripts/tunnel-e2e.sh sets it",
    );
    let relay = derper::start().await;
    let mac = Device::mint("mac-1", Scope::Read);
    let runner = Runner::start(&relay, &[&mac]).await;

    let home = tempfile::tempdir().expect("a scratch FARCOOLER_HOME for the Mac");
    store_node_key(&cli, home.path(), &mac.node.private_key);
    let wrong_pin = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    store_runners(&cli, home.path(), "fc-good", &runner.token, &runner.host_fingerprint);
    store_runners(&cli, home.path(), "fc-wrong", &runner.token, wrong_pin);

    let bundle = home.path().join("Far Cooler.app");
    std::fs::create_dir_all(&bundle).expect("a bundle directory");
    std::os::unix::fs::symlink(&cli, bundle.join("farcooler")).expect("link the CLI in");
    let program = format!("'{}'", bundle.join("farcooler").display());
    let golden = include_str!("fixtures/app-ssh-block.txt");
    let block = |host: &str, id: &str| {
        golden
            .replace("@PROGRAM@", &program)
            .replace("@ALIAS@", host)
            .replace("@ID@", id)
            .replace("@USER@", &unix_user())
            .replace("/@IDENTITY@", &home.path().join("id").display().to_string())
    };
    let config = home.path().join("config");
    std::fs::write(&config, block("cosmo", "fc-good") + &block("cosmo-wrong", "fc-wrong"))
        .expect("the ssh config");

    let destination = |host: &str, pin: String, argv: Vec<String>| {
        let _ = host;
        Destination {
            reach: Reach::Pipe { argv },
            user: unix_user(),
            private_key: mac.ssh_private.clone(),
            passphrase: None,
            host_key: HostKeyPolicy::Pinned(pin),
            derp_map: String::new(),
        }
    };
    // What the app's core is handed: ssh's own reading of the block, and the
    // CLI's answer for the pin.
    let good_argv = proxy_argv(&config, "cosmo");
    let wrong_argv = proxy_argv(&config, "cosmo-wrong");
    assert_eq!(good_argv[1..], ["runner", "pipe", "fc-good"], "ssh read another command: {good_argv:?}");
    let good_pin = pinned_key(&cli, home.path(), "fc-good");
    let wrong_pinned = pinned_key(&cli, home.path(), "fc-wrong");
    assert_eq!(good_pin, runner.host_fingerprint, "`runner pinned-key` is not the grant's pin");
    assert_eq!(wrong_pinned, wrong_pin);

    // The pipe child inherits this process's environment; the Mac app's does
    // too, and `FARCOOLER_HOME` is what points it at its store.
    // SAFETY: this file's tests run one at a time under SERIAL.
    unsafe { std::env::set_var("FARCOOLER_HOME", home.path()) };

    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut session = loop {
        match Session::open(&destination("cosmo", good_pin.clone(), good_argv.clone())).await {
            Ok(session) => break session,
            Err(why) => {
                assert!(Instant::now() < deadline, "the core never reached the daemon through the pipe: {why}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    };
    let client = speak(&mut session).await;
    assert_eq!(client.server_hello().granted_scope, Scope::Read as i32);

    // Twice, and each must END in the changed-host-key refusal. A dial right
    // after another can be dropped by the relay (`Handshake("Disconnected")`,
    // seen once in a full run), so an attempt that fails any OTHER way is tried
    // again, up to five times; one that succeeds fails the test at once, and so
    // does a run that never reaches the refusal.
    for round in 0..2 {
        let mut last = None;
        for _ in 0..5 {
            match Session::open(&destination("cosmo-wrong", wrong_pinned.clone(), wrong_argv.clone())).await {
                Ok(_) => panic!("the core trusted a host key that was not the pinned one"),
                Err(SshError::HostKeyChanged { expected, actual, .. }) => {
                    assert_eq!(expected, wrong_pin, "the refusal named another pin");
                    assert_eq!(actual, runner.host_fingerprint, "the refusal named another key");
                    last = None;
                    break;
                }
                Err(other) => {
                    last = Some(other);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        assert!(last.is_none(), "round {round}: the wrong pin was not refused as a changed host key: {last:?}");
    }
    // SAFETY: as above.
    unsafe { std::env::remove_var("FARCOOLER_HOME") };
    derper::note("the core spawned the block's ProxyCommand and reached the daemon; a wrong pin was refused");
}

/// `ssh -G`'s `proxycommand` for `host` under `config`, as the argv the app
/// would hand the core: the program unquoted, then the rest of the words.
#[cfg(target_os = "macos")]
fn proxy_argv(config: &Path, host: &str) -> Vec<String> {
    let out = std::process::Command::new("/usr/bin/ssh")
        .arg("-F")
        .arg(config)
        .args(["-G", "--", host])
        .output()
        .expect("run ssh -G");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("proxycommand "))
        .unwrap_or_else(|| panic!("ssh -G named no proxycommand for {host}: {text}"));
    let (program, rest) = match line.strip_prefix('\'') {
        Some(quoted) => quoted.split_once("' ").expect("an unclosed quote"),
        None => line.split_once(' ').expect("a bare program"),
    };
    std::iter::once(program.to_string()).chain(rest.split_whitespace().map(str::to_string)).collect()
}

/// `farcooler runner pinned-key <id>`, as the Mac app runs it.
#[cfg(target_os = "macos")]
fn pinned_key(cli: &str, home: &Path, id: &str) -> String {
    let out = std::process::Command::new(cli)
        .env("FARCOOLER_HOME", home)
        .args(["runner", "pinned-key", id])
        .output()
        .expect("run runner pinned-key");
    assert!(out.status.success(), "pinned-key failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `ssh <host> farcoolerd --stdio` under `config`, and the scope the daemon's
/// hello grants. Err carries what went wrong, for the retry loop to report.
async fn ssh_hello(config: &Path, home: &Path, host: &str) -> Result<i32, String> {
    let mut child = tokio::process::Command::new("/usr/bin/ssh")
        .env("FARCOOLER_HOME", home)
        .arg("-F")
        .arg(config)
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=20", host, "farcoolerd --stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::from(
            std::fs::File::create(home.join("ssh.err")).map_err(|e| e.to_string())?,
        ))
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let read = child.stdout.take().ok_or("no stdout")?;
    let write = child.stdin.take().ok_or("no stdin")?;
    let hello = tokio::time::timeout(
        Duration::from_secs(25),
        Client::over(read, write, "test-client", "0.0.0"),
    )
    .await
    .map_err(|_| {
        let said = std::fs::read_to_string(home.join("ssh.err")).unwrap_or_default();
        format!("no hello in 25 s; ssh said: {}", said.trim())
    })?
    .map_err(|e| {
        let said = std::fs::read_to_string(home.join("ssh.err")).unwrap_or_default();
        format!("{e}; ssh said: {}", said.trim())
    })?;
    let scope = hello.server_hello().granted_scope;
    let _ = child.kill().await;
    Ok(scope)
}

/// `farcooler runner store-node-key`, as the Mac app runs it.
fn store_node_key(cli: &str, home: &Path, key: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new(cli)
        .env("FARCOOLER_HOME", home)
        .args(["runner", "store-node-key"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("run runner store-node-key");
    child.stdin.take().expect("stdin").write_all(key.as_bytes()).expect("the key");
    assert!(child.wait().expect("wait").success(), "store-node-key failed");
}

/// `farcooler runner store-runners`, as the Mac app runs it on grant.
fn store_runners(cli: &str, home: &Path, id: &str, token: &str, host_key: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new(cli)
        .env("FARCOOLER_HOME", home)
        .args(["runner", "store-runners"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("run runner store-runners");
    let body = serde_json::json!([{ "id": id, "token": token, "host_key": host_key }]).to_string();
    child.stdin.take().expect("stdin").write_all(body.as_bytes()).expect("the runners");
    assert!(child.wait().expect("wait").success(), "store-runners failed");
}

/// Start `runner pipe`, return the first line it writes to stdout, and end it.
/// Empty when it exits (or stays silent for 15 s) without writing one.
async fn pipe_banner(cli: &str, home: &Path, id: &str) -> String {
    use tokio::io::AsyncBufReadExt as _;
    let mut child = tokio::process::Command::new(cli)
        .env("FARCOOLER_HOME", home)
        .args(["runner", "pipe", id])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("run runner pipe");
    let mut line = String::new();
    let mut out = tokio::io::BufReader::new(child.stdout.take().expect("stdout"));
    let _ = tokio::time::timeout(Duration::from_secs(15), out.read_line(&mut line)).await;
    let _ = child.kill().await;
    line
}

/// Dial until the runner answers, or give up loudly.
///
/// **A runner is not reachable the moment `serve` returns, and this is not the
/// test being lenient about it.** `Start` brings up the WireGuard engine and
/// returns; the DERP connection that makes the runner findable is established
/// in a goroutine afterwards, and magicsock picks a home relay on its first
/// netcheck. Measured here: the server logs `home is now derp-900` **3.0
/// seconds** after `serve` returned, and a dial issued before that gets
/// `derp-1 does not know about peer [...], removing route` and then spends its
/// whole ten-second budget without re-adding the route — so it fails even
/// though the runner came up while it was waiting.
///
/// That is a real property of the transport and a phone that dials a runner
/// which has just booted meets it. What this loop must not become is a way for
/// the test to pass by trying until something works: the budget is bounded, a
/// failure that is not `no_answer` is reported at once rather than retried, and
/// every attempt is printed with what it cost.
async fn reach(runner: &Runner, device: &Device) -> Session {
    // The measured 3.0 s plus margin, so the ordinary run pays one dial rather
    // than one failed dial (ten seconds) and then a good one. The loop below
    // is what makes this an optimization instead of a fixed sleep to be flaky
    // about.
    tokio::time::sleep(Duration::from_millis(3_500)).await;

    let began = Instant::now();
    let deadline = began + Duration::from_secs(90);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let tried = Instant::now();
        match open(runner, device).await {
            Ok(session) => {
                derper::note(&format!(
                    "{} reached the runner on attempt {attempt}, {:?} after serving",
                    device.client_id,
                    began.elapsed()
                ));
                return session;
            }
            Err(SshError::Tunnel { code: "no_answer" }) if Instant::now() < deadline => {
                derper::note(&format!(
                    "{} got no answer after {:?}; the runner may not have registered yet",
                    device.client_id,
                    tried.elapsed()
                ));
            }
            Err(other) => panic!(
                "{} could not reach the runner on attempt {attempt} after {:?}: {other}",
                device.client_id,
                began.elapsed()
            ),
        }
    }
}

/// Dial this runner as this device, exactly as a phone would.
///
/// `Session::open` and not a hand-rolled dial: the branch under test is the
/// shipped one, including the port-22-as-a-name convention and the error
/// mapping. A test that called `farcooler_tailcat::dial` itself would prove
/// the Go shim works and say nothing about whether the client reaches it.
async fn open(runner: &Runner, device: &Device) -> Result<Session, SshError> {
    Session::open(&Destination {
        reach: Reach::Tailcat {
            token: runner.token.clone(),
            client_key: device.node.private_key.clone(),
        },
        user: unix_user(),
        private_key: device.ssh_private.clone(),
        passphrase: None,
        host_key: HostKeyPolicy::Pinned(runner.host_fingerprint.clone()),
        // The relay this whole test agrees on, supplied the way a phone
        // supplies it: in the destination, so `Session::open`'s caller sets
        // the rendezvous on the way in rather than the test setting it out of
        // band. `Runner::start` has already set the same URL process-wide
        // because the runner's own `serve` needed it before any device
        // existed — this is the DEVICE half of the same setting, and it is
        // the one a phone actually exercises.
        derp_map: runner.derp_map.clone(),
    })
    .await
}

/// Run the shipped forced command's program over the session and handshake.
///
/// What is asked for is what `crates/cli/src/remote.rs` asks for. sshd
/// overrides it with the line's forced command, which is the property
/// `a_real_sshd_forces_the_scope.rs` is about; here it matters only that
/// nothing in this test is choosing the daemon's argv.
async fn speak(
    session: &mut Session,
) -> Client<farcooler_client::ssh::ChannelReader, std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send>>>
{
    let streams = session.exec("farcoolerd --stdio").await.expect("exec through the tunnel");
    Client::over(streams.reader, streams.writer, "test-client", "0.0.0")
        .await
        .expect("handshake over a tunnel, a relay and a real sshd")
}

/// One device: an SSH key pair for sshd and a tailcat node key pair for the
/// tunnel. Two identities, and the test is partly about them staying attached
/// to each other.
struct Device {
    client_id: String,
    scope: Scope,
    ssh_private: String,
    ssh_public: String,
    node: NodeKeyPair,
}

impl Device {
    /// Both halves of both identities, from the shipped minting call.
    ///
    /// `farcooler_tailcat::mint_node_key` rather than a constant, because a
    /// constant would be this file's idea of a node key and what is on trial
    /// includes whether Go's encoder and `fence::usable_node_key` agree about
    /// the spelling.
    fn mint(client_id: &str, scope: Scope) -> Self {
        let node = farcooler_tailcat::mint_node_key()
            .expect("a linked build mints a node key for a device");
        let dir = tempfile::tempdir().expect("a scratch directory for a device key");
        let path = dir.path().join("id");
        keygen(&path, client_id);
        Self {
            client_id: client_id.to_string(),
            scope,
            ssh_private: std::fs::read_to_string(&path).expect("the device's private key"),
            ssh_public: std::fs::read_to_string(path.with_extension("pub"))
                .expect("the device's public key"),
            node,
        }
    }

    /// The `authorized_keys` line this device is admitted by, from the shipped
    /// renderer. Never a string in this file: the format is what is on trial.
    fn line(&self) -> String {
        fence::render(
            self.ssh_public.trim(),
            "Test Device",
            &self.client_id,
            self.scope,
            Grant::FarCooler,
            Some(&self.node.public_key),
        )
        .expect("render a Key A line carrying a node key")
    }
}

/// One runner: a real sshd, a real fence, a real tunnel identity, and the
/// tailcat server that this process is holding open on its behalf.
///
/// **Field order is drop order.** `_dir` deletes the directory the tmux reaper
/// reads an install id out of, so it is declared last and dropped last — the
/// exact leak `common::DaemonChild` was written about.
struct Runner {
    _sshd: Sshd,
    /// The token a device dials. Minted by `conn_blob` from the running
    /// server, so it names the relay this runner actually registered with.
    token: String,
    /// The scratch sshd's host key, in the form `Destination::host_key` pins.
    host_fingerprint: String,
    /// The DERP map naming this test's own relay, so a device can be handed it
    /// in its `Destination` the way a phone hands over its own setting.
    derp_map: String,
    ssh_port: u16,
    tailcat_key: PathBuf,
    authorized_keys: PathBuf,
    _tmux: common::TmuxReaper,
    _dir: tempfile::TempDir,
}

impl Runner {
    /// Enroll these devices, put a real sshd in front, and serve a tunnel.
    async fn start(relay: &derper::Derper, devices: &[&Device]) -> Self {
        let sshd_binary = Path::new("/usr/sbin/sshd");
        assert!(
            sshd_binary.exists(),
            "no {} on this machine, so nothing here was tested",
            sshd_binary.display()
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let home = root.join("home");
        std::fs::create_dir(&home).expect("the account home");
        let runtime = root.join("rt");
        std::fs::create_dir(&runtime).expect("the runtime directory");
        let bin = root.join("bin");
        std::fs::create_dir(&bin).expect("the bin directory");

        keygen(&root.join("host_ed25519"), "host");
        let host_fingerprint = fingerprint(&root.join("host_ed25519.pub"));

        let authorized_keys = home.join(".ssh").join("authorized_keys");
        write_fence(&authorized_keys, devices);
        scaffold(&authorized_keys, &home, &runtime, &bin, devices);

        let ssh_port = free_port();
        let sshd = start_sshd(sshd_binary, &root, &authorized_keys, ssh_port).await;

        // Every process in this test agrees on one relay, which is what makes
        // it a LOCAL test: the runner registers there and the device is told
        // about it inside the token. Process-wide because it is deployment
        // configuration rather than a property of one connection — see
        // `farcooler_tailcat::set_derp_map_url`.
        farcooler_tailcat::set_derp_map_url(&relay.map_url);

        // The runner's own identity, through the shipped call rather than by
        // letting `serve` create one as a side effect. This is the first
        // execution of `ensure_identity` in any build: it has been
        // type-checked and never run.
        let tailcat_key = runtime.join("tailcat.key");
        farcooler_tailcat::ensure_identity(&tailcat_key).expect("mint this runner's identity");
        assert!(tailcat_key.exists(), "ensure_identity reported success and wrote no key");
        assert_eq!(
            mode_of(&tailcat_key),
            0o600,
            "a runner's tunnel private key was left readable by other accounts"
        );

        let mut runner = Self {
            _sshd: sshd,
            token: String::new(),
            host_fingerprint,
            derp_map: relay.map_url.clone(),
            ssh_port,
            tailcat_key,
            authorized_keys,
            _tmux: common::TmuxReaper::new(&runtime),
            _dir: dir,
        };
        runner.token = runner.serve(devices).await;
        runner
    }

    /// Make the running tunnel match `authorized_keys`, and answer with the
    /// token it is now serving.
    ///
    /// `allowlist::start_tunnel`'s three steps, in its order, with its
    /// decision: read the file with `fence::read`, ask `tunnel_plan` who is
    /// admitted, hand that to `serve`. Only the port differs, and this file's
    /// header says why.
    async fn serve(&self, expected: &[&Device]) -> String {
        let entries = fence::read(&self.authorized_keys, fence::AUTHORIZED_KEYS)
            .expect("the fence sshd authenticates against is readable");

        // The node key survived the file, which is not free: it goes in
        // through `render`'s forced command and comes back out through
        // `entry_from_line`'s `--node-key` flag, past a `usable_node_key`
        // filter, in a line this test then prefixed with `environment=`
        // options of its own.
        for device in expected {
            let entry = entries
                .iter()
                .find(|e| e.client_id == device.client_id)
                .unwrap_or_else(|| panic!("{} is not in the fence", device.client_id));
            assert_eq!(
                entry.node_key, device.node.public_key,
                "the node key {} offered did not survive authorized_keys",
                device.client_id
            );
        }

        let allowed = allowlist::tunnel_plan(&self.tailcat_key, &entries)
            .expect("this runner has an identity and admits somebody");
        let expected_keys: Vec<String> =
            expected.iter().map(|d| d.node.public_key.clone()).collect();
        assert_eq!(
            allowed.keys(),
            expected_keys.as_slice(),
            "the shipped projection admits a different set than the file names"
        );

        let key_path = self.tailcat_key.clone();
        let ssh_port = self.ssh_port;
        let keys = allowed.keys().to_vec();
        let began = Instant::now();
        // On the blocking pool because `serve` is synchronous and can hold
        // Go's package mutex for tens of seconds — a region pick, then up to
        // two `Start` attempts. Blocking a worker here would also park this
        // test's own DERP-map HTTP server, which `serve` is about to fetch
        // from.
        tokio::task::spawn_blocking(move || farcooler_tailcat::serve(&key_path, ssh_port, &keys))
            .await
            .expect("the serving task finished")
            .expect("this runner's tunnel came up against the local relay");
        let token = tokio::task::spawn_blocking(farcooler_tailcat::conn_blob)
            .await
            .expect("the token task finished")
            .expect("a serving runner has a token");
        derper::note(&format!(
            "the runner served in {:?} and minted a {}-byte token",
            began.elapsed(),
            token.len()
        ));
        token
    }

    /// Take a device out of `authorized_keys` and make the running tunnel
    /// match, exactly as `enrollment::revoke` does.
    async fn revoke(&mut self, gone: &Device, remaining: &[&Device]) {
        let before = fence::read(&self.authorized_keys, fence::AUTHORIZED_KEYS)
            .expect("read the fence before revoking");
        assert!(
            before.iter().any(|e| e.client_id == gone.client_id),
            "nothing to revoke: {} is not in the fence",
            gone.client_id
        );

        write_fence(&self.authorized_keys, remaining);
        scaffold(&self.authorized_keys, &self.home(), &self.runtime(), &self.bin(), remaining);
        self.token = self.serve(remaining).await;
    }

    /// This runner's daemon, serving its `FARCOOLER_HOME` socket from this
    /// process, so a forced `farcoolerd --stdio` relays into it rather than
    /// opening a service of its own, and a revoke here closes that session.
    async fn daemon(&self) -> std::sync::Arc<farcooler_daemon::service::Service> {
        use farcooler_daemon::{rpc::RpcFactory, sessions::peer_from_preamble};
        farcooler_daemon::service::stub_agents_in_this_process();
        let service = std::sync::Arc::new(
            farcooler_daemon::service::Service::open_in(self.runtime())
                .await
                .expect("this runner's service")
                .enrolling_into(self.authorized_keys.clone()),
        );
        assert_eq!(service.tailcat_key(), self.tailcat_key, "the daemon would rebuild some other tunnel");
        let server = farcooler_transport::UnixListenerServer::bind(self.runtime().join("farcoolerd.sock"))
            .expect("bind this runner's socket");
        let watcher = farcooler_daemon::watch::Watcher::new(service.clone());
        let served = service.clone();
        tokio::spawn(async move {
            let _ = server
                .serve(move |preamble| {
                    let peer = peer_from_preamble(preamble.as_ref())?;
                    Some((
                        farcooler_transport::HandshakeConfig { daemon_version: "test".into() },
                        RpcFactory::new(served.clone(), watcher.clone(), std::sync::Arc::new(tokio::sync::Notify::new()), peer),
                    ))
                })
                .await;
        });
        service
    }

    fn home(&self) -> PathBuf {
        self._dir.path().join("home")
    }
    fn runtime(&self) -> PathBuf {
        self._dir.path().join("rt")
    }
    fn bin(&self) -> PathBuf {
        self._dir.path().join("bin")
    }
}

/// Write these devices' lines through the shipped fence writer.
fn write_fence(path: &Path, devices: &[&Device]) {
    let lines: Vec<String> = devices.iter().map(|d| d.line()).collect();
    fence::write(path, fence::AUTHORIZED_KEYS, &lines, &[], Placement::Last)
        .expect("write the fence");
    // Asserted rather than imposed: sshd under `StrictModes` refuses a file
    // anybody else can write, so this is a property of the shipped writer that
    // a real sshd is entitled to check.
    assert_eq!(mode_of(path), 0o600, "the fence writer left the file too open");
}

/// Prefix each of our lines with the environment a forced command needs.
///
/// The same scaffolding, for the same reasons, as
/// `a_real_sshd_forces_the_scope.rs`: the shipped forced command names a
/// program that can only be found on PATH, and the daemon reads
/// `$HOME/.ssh/authorized_keys` with no override — so a forced command that
/// inherited the real HOME would have this test's session reading the
/// developer's own fence.
///
/// **Everything after the `environment=` options is `fence::render`'s output
/// verbatim.** `PermitUserEnvironment` is off by default and no runner turns it
/// on, so these options carry no privilege; they are scaffolding and nothing
/// else.
fn scaffold(path: &Path, home: &Path, runtime: &Path, bin: &Path, devices: &[&Device]) {
    let mut file = std::fs::read_to_string(path).expect("read back the fence");
    for device in devices {
        let line = device.line();
        let prefix = format!(
            "environment=\"HOME={}\",environment=\"FARCOOLER_HOME={}\",\
             environment=\"FARCOOLER_TEST_STUB_AGENTS=1\",environment=\"PATH={}\"",
            home.display(),
            runtime.display(),
            path_for_forced_command(home, bin, &line)
        );
        let prefix = prefix + &tmux_tmpdir_option();
        let scaffolded = file.replacen(&line, &format!("{prefix},{line}"), 1);
        assert_ne!(
            scaffolded, file,
            "the rendered line for {} was not in the file just written",
            device.client_id
        );
        file = scaffolded;
    }
    std::fs::write(path, &file).expect("prefix the test scaffolding");
    assert_eq!(mode_of(path), 0o600, "the scaffolding rewrite widened the mode");
}

/// A real sshd, and the rule for ending it.
struct Sshd {
    /// Read from `PidFile` at startup, while the directory still exists. Drop
    /// must not depend on reading a temporary directory that may already be
    /// gone.
    pid: i32,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // By pid, never by pattern. This machine runs the developer's own
        // sshd, and a pattern kill here would take their remote access with
        // it. Only the listener is signalled; each connection's `sshd-session`
        // child ends when its tunnel does.
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
    }
}

/// Configure, validate and start a loopback sshd, and wait until it answers.
async fn start_sshd(binary: &Path, root: &Path, authorized_keys: &Path, port: u16) -> Sshd {
    let config = root.join("sshd_config");
    let log = root.join("sshd.log");
    let pid_file = root.join("sshd.pid");
    std::fs::write(
        &config,
        format!(
            "Port {port}\n\
             ListenAddress 127.0.0.1\n\
             HostKey {root}/host_ed25519\n\
             AuthorizedKeysFile {authorized_keys}\n\
             PidFile {pid_file}\n\
             StrictModes no\n\
             UsePAM no\n\
             PermitUserEnvironment yes\n\
             PasswordAuthentication no\n\
             KbdInteractiveAuthentication no\n\
             LogLevel DEBUG1\n",
            root = root.display(),
            authorized_keys = authorized_keys.display(),
            pid_file = pid_file.display(),
        ),
    )
    .expect("write sshd_config");

    // Validated before it is started, so a config this test got wrong reads as
    // sshd's own complaint rather than as a tunnel that goes nowhere.
    let checked = std::process::Command::new(binary)
        .arg("-t")
        .arg("-f")
        .arg(&config)
        .output()
        .expect("run sshd -t");
    assert!(
        checked.status.success(),
        "sshd refused the config: {}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let started = std::process::Command::new(binary)
        .arg("-f")
        .arg(&config)
        .arg("-E")
        .arg(&log)
        .status()
        .expect("run sshd");
    assert!(started.success(), "sshd did not start; see {}", log.display());

    // Both facts, because they are different ones: `sshd -f` forks and returns
    // as soon as the parent forked, so its status says nothing about whether
    // anything is listening.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(pid) =
            std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<i32>().ok())
            && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
        {
            return Sshd { pid };
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no sshd ever listened on port {port}; see {}", log.display());
}

/// Put the built daemon where the rendered forced command will look for it.
///
/// The name is read out of the RENDERED line rather than written down, so this
/// follows the product wherever the spelling goes. A `~/`-anchored name is
/// resolved against the OVERRIDDEN home: this machine has a live Far Cooler
/// install, and a link placed in the real `~/.local/bin` would clobber it.
fn path_for_forced_command(home: &Path, bin: &Path, line: &str) -> String {
    let program = forced_program(line);
    let target = match program.strip_prefix("~/") {
        Some(rest) => {
            let path = home.join(rest);
            let parent = path.parent().expect("a path-bearing program name has a parent");
            std::fs::create_dir_all(parent).expect("the directory the forced command names");
            path
        }
        None if program.starts_with('/') => {
            panic!("the forced command names the absolute path {program}, which this test will not create")
        }
        None => bin.join(program),
    };
    // Idempotent: `scaffold` runs again after a revoke, over the same home.
    if !target.exists() {
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_farcoolerd"), &target)
            .expect("link the daemon under the name the forced command asks for");
    }
    // The real directories after ours, because sshd runs a forced command
    // through the account's login shell and a shell that cannot find `tty` or
    // `mktemp` writes pages of its own errors down the session.
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// The program a rendered line asks sshd to run.
fn forced_program(line: &str) -> &str {
    let (_, after) = line.split_once("command=\"").expect("a Key A line carries a forced command");
    let (command, _) = after.split_once('"').expect("a forced command is quoted");
    command.split_whitespace().next().expect("a forced command names a program")
}

/// The account sshd will authenticate. A non-root sshd can only ever
/// authenticate the user that started it, which is this test's own.
fn unix_user() -> String {
    std::env::var("USER").expect("a test runs as somebody")
}

/// An ed25519 keypair, from OpenSSH's own tool.
///
/// `ssh-keygen` rather than a Rust keypair, because these keys go into a file
/// OpenSSH parses: a key generated another way and then refused would be
/// indistinguishable from the fence writing a line sshd refuses.
fn keygen(path: &Path, comment: &str) {
    let status = std::process::Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen did not write {}", path.display());
}

/// The `SHA256:…` a client pins, computed the way the client computes it.
fn fingerprint(public_key: &Path) -> String {
    let text = std::fs::read_to_string(public_key).expect("read a public key");
    ssh_key::PublicKey::from_openssh(text.trim())
        .expect("ssh-keygen wrote a key ssh-key can read")
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string()
}

/// A port nothing was listening on a moment ago.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a scratch port");
    listener.local_addr().expect("the scratch port's address").port()
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

/// `TMUX_TMPDIR` for a forced command, as an `environment=` option, when this
/// test run has a private one (`scripts/tmux-leak-check.py`): sshd hands the
/// command none of our environment, so without it the command would look for
/// tmux's sockets somewhere else (ov-207).
fn tmux_tmpdir_option() -> String {
    std::env::var("TMUX_TMPDIR").map(|d| format!(",environment=\"TMUX_TMPDIR={d}\"")).unwrap_or_default()
}

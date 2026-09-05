//! The guard this design is most likely to lose.
//!
//! Tailcat reads an empty `AllowedClients` as "admit everyone", so a runner
//! that started a server from a fleet enrolled before the tunnel existed would
//! open its sshd to anyone holding its token.
//!
//! `cargo test` builds this crate's DEFAULT features — no Go archive linked —
//! which makes `farcooler_tailcat::serve` return `Err(NoTailcatLinked)`
//! unconditionally, for any input at all. An assertion here that only checked
//! `start_tunnel(&service).await.is_none()` could not tell a guard that ran
//! from a guard that was deleted: either way the stub refuses, and either way
//! the answer is "no tunnel". `allowlist.rs`'s own unit tests assert on
//! `tunnel_plan` directly for exactly that reason — it never reaches
//! `farcooler_tailcat` at all, so a stub cannot stand in for a guard that was
//! removed. The two tests below assert on `start_tunnel`'s distinguishing
//! `TunnelOutcome` variant instead of a bare `Option`, which gets the same
//! property for the end-to-end path: deleting either guard in `start_tunnel`
//! routes to `serve`, which the stub still refuses, but with a DIFFERENT
//! variant (`ServeFailed("no_tailcat")`) than the guard would have produced —
//! so the specific variant asserted below is what a deleted guard breaks.

use std::path::Path;

use farcooler_daemon::{
    allowlist::{self, TunnelOutcome},
    service::Service,
};
use farcooler_fence::Grant;
use farcooler_protocol::v1::Scope;

/// A valid ed25519 public key, chosen for being obviously synthetic. Not the
/// point of either test — only `render` needs it to parse — so one key does
/// for both.
const DEVICE_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA phone";

/// 32 bytes of X25519 public key, as 43 characters of unpadded base64 — the
/// shape `usable_node_key` requires. Matches the constant `crates/fence/src/
/// lib.rs` and `crates/daemon/src/allowlist.rs` already use for the same
/// reason: obviously synthetic, and long enough to be a real key rather than
/// a placeholder.
const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// A daemon `Service` with its own scratch `authorized_keys` and its own
/// `FARCOOLER_HOME`, carrying a tailcat identity from the start — so the
/// second test's `remove_file` is a real change from a real starting state,
/// not an absence this fixture never bothered to create.
///
/// A short root, not a long one: `Service::open_in` also stands up a tmux
/// server keyed off this path, and the daemon's own Unix socket (bound
/// elsewhere in production, never here) has a 108-byte path limit that a
/// deeply nested `tempfile` root can blow past.
async fn test_service(root: &Path) -> Service {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();

    let key_path = root.join("tailcat.key");
    std::fs::write(&key_path, b"scratch tailcat identity, never read by the stub").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    Service::open_in(root.to_path_buf())
        .await
        .expect("service")
        .enrolling_into(home.join(".ssh").join("authorized_keys"))
}

/// Enroll a device the ordinary way: a Far Cooler line, no node key. This is
/// exactly what every device enrolled before the tunnel existed looks like —
/// the fleet `start_tunnel` must never open sshd to.
async fn enroll_without_a_node_key(service: &Service, client_id: &str) {
    write_line(service, client_id, None).await;
}

/// Enroll a device carrying a node key, as a ceremony that could offer one
/// would leave it. None can: see `write_line`.
async fn enroll_with_a_node_key(service: &Service, client_id: &str, node_key: &str) {
    write_line(service, client_id, Some(node_key)).await;
}

/// Render one Far Cooler line and place it inside the fence.
///
/// Not `enrollment::enroll`: that RPC layer function always renders with no
/// node key — `ClientEnroll` has no field for one, and while the ceremony offer
/// gained a `node_key` at v=2, nothing populates it, so no enrollment path
/// passes one either — and it therefore cannot produce the fixture
/// `enroll_with_a_node_key` needs. This goes straight to `farcooler_fence`,
/// which is already a direct dependency of this crate and is the same
/// primitive `enrollment::enroll` itself calls.
async fn write_line(service: &Service, client_id: &str, node_key: Option<&str>) {
    let line = farcooler_fence::render(
        DEVICE_KEY,
        "device",
        client_id,
        Scope::Control,
        Grant::FarCooler,
        node_key,
    )
    .expect("a synthetic key and id render");

    let path = service.authorized_keys().to_path_buf();
    tokio::task::spawn_blocking(move || {
        farcooler_fence::update(
            &path,
            farcooler_fence::AUTHORIZED_KEYS,
            farcooler_fence::Placement::Last,
            move |entries| {
                let mut ours: Vec<String> = entries.iter().map(|e| e.line.clone()).collect();
                ours.push(line);
                Ok::<_, farcooler_fence::FenceError>((
                    farcooler_fence::Change::Write { entries: ours, foreign: Vec::new() },
                    (),
                ))
            },
        )
    })
    .await
    .expect("the write task ran")
    .expect("authorized_keys accepted the line");
}

#[tokio::test]
async fn a_runner_whose_devices_have_no_node_keys_starts_no_tunnel() {
    let home = tempfile::tempdir().unwrap();
    let service = test_service(home.path()).await;
    enroll_without_a_node_key(&service, "c1").await;

    let outcome = allowlist::start_tunnel(&service).await;
    assert_eq!(
        outcome,
        TunnelOutcome::NobodyAdmitted,
        "a runner with no admitted devices did not refuse with NobodyAdmitted: {outcome:?}"
    );
}

#[tokio::test]
async fn a_runner_with_no_key_file_starts_no_tunnel() {
    let home = tempfile::tempdir().unwrap();
    let service = test_service(home.path()).await;
    enroll_with_a_node_key(&service, "c1", NODE_KEY).await;
    std::fs::remove_file(service.tailcat_key()).ok();

    let outcome = allowlist::start_tunnel(&service).await;
    assert_eq!(
        outcome,
        TunnelOutcome::NoIdentity,
        "a runner with no key file did not refuse with NoIdentity: {outcome:?}"
    );
    // A second, independent signal: even under a build with the real archive
    // linked, `serve` must never be reached (it is what would call Go's
    // `loadOrCreateKey`, which writes this file if absent).
    assert!(!service.tailcat_key().exists(), "a key file appeared for a runner with no tunnel");
}

/// The daemon-side counterpart to `crates/client/src/ssh.rs`'s
/// `a_tunnel_destination_without_an_archive_says_so`. A runner that clears
/// BOTH guards — real identity, a real admitted device — must still fail by
/// name, not by accident, when this build carries no tunnel at all. A plain
/// `cargo test` is exactly that build.
///
/// Both tunnel features are excluded, not just `tailcat`. Under
/// `tailcat-helper` this build HAS a tunnel backend — it would reach a helper,
/// or fail trying, and answer `io` rather than `no_tailcat` — so the premise
/// is false rather than the assertion being wrong. It would also race the test
/// below it, which sets `FARCOOLER_TUNNEL_HELPER` for the whole process.
#[cfg(not(any(feature = "tailcat", feature = "tailcat-helper")))]
#[tokio::test]
async fn a_runner_with_no_archive_linked_fails_the_tunnel_by_name() {
    let home = tempfile::tempdir().unwrap();
    let service = test_service(home.path()).await;
    enroll_with_a_node_key(&service, "c1", NODE_KEY).await;

    let outcome = allowlist::start_tunnel(&service).await;
    assert_eq!(
        outcome,
        TunnelOutcome::ServeFailed("no_tailcat"),
        "a runner past both guards did not fail by the stub's own name: {outcome:?}"
    );
}

/// The same guard, one level lower: a runner that admits nobody must not
/// START anything either.
///
/// On Linux the tunnel is not linked into this daemon — it is a separate
/// process `farcoolerd` spawns, because a Go c-archive inside a musl binary
/// segfaults before it can log a word (`crates/tailcat/src/helper.rs`). That
/// makes "no tunnel" a thing with a visible footprint for the first time: a
/// process either exists or it does not. The two tests above assert on a
/// return value, and a return value is what a stub can fake. A process cannot
/// be faked, which is what makes this the stronger form of the same claim.
///
/// It needs no Go toolchain and no helper binary. `FARCOOLER_TUNNEL_HELPER`
/// points at a shell script that records that it ran, so "was a helper
/// started" is a file on disk rather than an inference.
///
/// Both halves in one test, deliberately: the environment variable is
/// process-wide, and two tests setting it would race each other under `cargo
/// test`'s default parallelism. The second half is what stops the first from
/// being vacuous — a `serve` that never spawned anything under any
/// circumstances would satisfy the first assertion perfectly.
#[cfg(feature = "tailcat-helper")]
#[tokio::test]
async fn a_runner_that_admits_nobody_starts_no_helper_process() {
    let home = tempfile::tempdir().unwrap();
    let marker = home.path().join("the-helper-ran");

    // Answers one command and exits. Enough to be spawned and talked to; the
    // errno it reports is not the point, only that it was reached at all.
    let script = home.path().join("fake-tunnel-helper");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch {}\necho 'err 22'\n", marker.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // SAFETY: `cargo test` runs these on a multi-threaded runtime, and
    // `set_var` is only sound while no other thread reads the environment.
    // Nothing else in this file touches it, and this is the only test that
    // sets it — see the note above about why both halves live in one test.
    unsafe { std::env::set_var("FARCOOLER_TUNNEL_HELPER", &script) };

    let service = test_service(home.path()).await;
    enroll_without_a_node_key(&service, "c1").await;
    let outcome = allowlist::start_tunnel(&service).await;
    assert_eq!(outcome, TunnelOutcome::NobodyAdmitted, "{outcome:?}");
    assert!(
        !marker.exists(),
        "a runner admitting nobody started a tunnel helper anyway"
    );

    // The other direction, so the assertion above is a fact about the guard
    // rather than a fact about this fixture never spawning anything.
    enroll_with_a_node_key(&service, "c2", NODE_KEY).await;
    let outcome = allowlist::start_tunnel(&service).await;
    assert!(
        matches!(outcome, TunnelOutcome::ServeFailed(_)),
        "a runner with an admitted device did not reach the helper: {outcome:?}"
    );
    assert!(
        marker.exists(),
        "no helper was started for a runner that admits somebody, so the \
         assertion above proves nothing"
    );

    unsafe { std::env::remove_var("FARCOOLER_TUNNEL_HELPER") };
}

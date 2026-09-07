//! `FARCOOLER_DERP_MAP`, through a real `farcoolerd`, to the thing that serves.
//!
//! ## What this exists to catch
//!
//! `rendezvous::apply_configured_derp_map` is a unit-tested function, and its
//! unit tests prove that reading the variable and telling the tunnel are wired
//! to each other. They cannot prove the daemon ever CALLS it: the call is one
//! line inside `#[tokio::main] async fn main()`, and nothing in this workspace
//! links that binary as a library. Deleting the line left every test green,
//! which is the defect this file closes — the same shape as an `#[ignore]`
//! whose lane does not exist, except with no attribute to notice.
//!
//! So this spawns the actual `farcoolerd` binary with the variable set and asks
//! the far end of the chain what it heard. Every link is real: the process's own
//! environment, `main`, `apply_configured_derp_map`,
//! `farcooler_tailcat::set_derp_map_url`, `allowlist::start_tunnel`, `serve`,
//! and `helper::spawn`. Break any of them and this goes red.
//!
//! ## Why the helper backend, and why CI must name this file
//!
//! A default `cargo test` links no tunnel. `farcooler_tailcat::serve` returns
//! `Err(NoTailcatLinked)` for any input at all, so nothing is spawned, nothing
//! is told anything, and "did the DERP map reach the thing that serves" is not
//! a question that build can answer — the assertion would be vacuous whatever
//! the daemon did.
//!
//! Under `tailcat-helper` the tunnel is a PROCESS, and the helper is told its
//! DERP map on stdin before it is told to serve (`helper::spawn`). A fake
//! helper is a shell script that writes down every line it is given, so what
//! the runner's rendezvous ended up being stops being an inference and becomes
//! a word in a file.
//!
//! `.github/workflows/ci.yml` therefore names this file explicitly, as it does
//! `an_empty_allowlist_starts_no_tunnel.rs` and the two beside it.
//! `--workspace` builds default features and would compile every test here
//! away.
//!
//! ## Why the ORDER is asserted and not just the presence
//!
//! `helper::spawn` sends `derpmap` and then `serve`, and that order is the
//! whole value of the setting: tailcat reads the map when the server is built,
//! so a runner told afterwards is a runner on the wrong rendezvous until
//! somebody restarts it. A `derpmap` line that arrived after `serve` would
//! satisfy a presence check and fix nothing.
#![cfg(feature = "tailcat-helper")]

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::Scope;

mod common;
use common::DaemonChild;

/// What this runner's installer is pretending to have written.
const MAP_URL: &str = "https://derp.example/derpmap.json";

/// One enrolled device, so `authorized_keys` admits somebody and
/// `allowlist::tunnel_plan` does not stop at `NobodyAdmitted`.
const RECEIVED_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA phone";

/// Its node key: 43 characters of unpadded base64-URL, which is what
/// `fence::usable_node_key` accepts and what admits a device to the tunnel.
const NODE_KEY: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// A runner installed with a DERP map serves on that map.
///
/// The assertion is on the helper's own transcript, and on the order of two
/// lines in it, because that is the difference between a setting that works and
/// a setting that is applied one restart too late.
#[tokio::test]
async fn a_runner_installed_with_a_derp_map_serves_on_it() {
    let runner = Runner::start(Some(MAP_URL)).await;
    let heard = runner.await_serve().await;

    let derpmap = heard
        .iter()
        .position(|line| line == &format!("derpmap {MAP_URL}"))
        .unwrap_or_else(|| {
            panic!("the runner's DERP map never reached the thing that serves: {heard:?}")
        });
    let serve = heard
        .iter()
        .position(|line| line.starts_with("serve "))
        .expect("await_serve returned without a serve line");
    assert!(
        derpmap < serve,
        "the DERP map arrived after the server was built, so this runner is on the wrong \
         rendezvous until it restarts: {heard:?}"
    );
}

/// A runner nobody configured is told nothing, and is left on the library's
/// own default.
///
/// The counterpart the first test cannot supply: a daemon that sent `derpmap`
/// unconditionally — with an empty URL, or with a default of its own baked in —
/// would pass every assertion above and would quietly override the map every
/// build of tailcat ships with.
#[tokio::test]
async fn an_unconfigured_runner_is_told_no_map_at_all() {
    let runner = Runner::start(None).await;
    let heard = runner.await_serve().await;

    assert!(
        !heard.iter().any(|line| line.starts_with("derpmap")),
        "a runner nobody configured was moved off the library's default: {heard:?}"
    );
}

/// A daemon, the fence it reads, and the fake helper it talks to.
///
/// **Field order is drop order.** `_dir` deletes the directory `DaemonChild`'s
/// Drop reads `install-id` out of to find the tmux server it has to reap, so it
/// is declared last and dropped last — the same rule, and the same leak,
/// `a_real_sshd_forces_the_scope.rs` documents.
struct Runner {
    _daemon: DaemonChild,
    /// Every line the fake helper was given, in order.
    log: PathBuf,
    /// The daemon's own stderr, for a failure that is about the daemon rather
    /// than about the helper.
    stderr: PathBuf,
    _dir: tempfile::TempDir,
}

impl Runner {
    /// A runner whose installer set `FARCOOLER_DERP_MAP` to `map`, or did not.
    ///
    /// Everything is passed in the CHILD's environment. Nothing here calls
    /// `set_var` on this test process, which is what lets the two tests in this
    /// file run in parallel: `cargo test` runs them on two threads in one
    /// process, and a shared `FARCOOLER_DERP_MAP` would have them taking turns
    /// failing. It is also the only honest way to test this at all — the
    /// variable is read by the daemon at ITS startup, so setting it here would
    /// be testing this test's environment.
    async fn start(map: Option<&str>) -> Runner {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        // The account home, whose `.ssh` the fence writer creates itself. The
        // daemon reads `$HOME/.ssh/authorized_keys` with no override, so a
        // daemon that inherited the real HOME would read — and could rewrite —
        // the developer's own fence.
        let home = root.join("home");
        std::fs::create_dir(&home).expect("the account home");
        let runtime = root.join("rt");
        std::fs::create_dir(&runtime).expect("the runtime directory");

        // One device, admitted, through the shipped renderer rather than a
        // string written here: a hand-rolled line that `fence::read` did not
        // parse would make this test report `NobodyAdmitted` as a missing DERP
        // map.
        let line = fence::render(
            RECEIVED_KEY,
            "Test Device",
            "phone-7",
            Scope::Read,
            Grant::FarCooler,
            Some(NODE_KEY),
        )
        .expect("render a Key A line carrying a node key");
        fence::write(
            &home.join(".ssh").join("authorized_keys"),
            fence::AUTHORIZED_KEYS,
            std::slice::from_ref(&line),
            &[],
            Placement::Last,
        )
        .expect("write the fence");

        // `tunnel_plan` answers `NoIdentity` and starts nothing without this,
        // and at boot nothing mints one: `enrollment::enroll` is the ceremony
        // that does, and no pairing happens here. `scripts/tunnel-smoke.sh`
        // writes one by hand for the same reason.
        let key = runtime.join("tailcat.key");
        std::fs::write(&key, b"").expect("this runner's tailcat identity");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600))
            .expect("0600, as the real one is");

        let log = root.join("helper.log");
        let helper = fake_helper(&root, &log);
        let stderr = root.join("daemon.err");

        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcoolerd"));
        command
            .env("FARCOOLER_HOME", &runtime)
            .env("HOME", &home)
            .env("FARCOOLER_TUNNEL_HELPER", &helper)
            .env("RUST_LOG", "farcooler=debug")
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&stderr).expect("the daemon's stderr"))
            .kill_on_drop(true);
        match map {
            Some(url) => command.env("FARCOOLER_DERP_MAP", url),
            // Removed rather than left alone: this test process inherits the
            // developer's environment, and a machine that happens to have this
            // set would make the unconfigured case pass for the wrong reason.
            None => command.env_remove("FARCOOLER_DERP_MAP"),
        };
        let child = command.spawn().expect("spawn farcoolerd");

        Runner {
            _daemon: DaemonChild { child, home: runtime },
            log,
            stderr,
            _dir: dir,
        }
    }

    /// Every line the helper heard, once it has been asked to serve.
    ///
    /// `serve` is the terminator rather than a fixed sleep because it is the
    /// last thing `helper::spawn`'s caller sends: waiting for it means every
    /// line that was going to arrive has arrived, so an absent `derpmap` is
    /// genuinely absent rather than merely late. A sleep long enough to be
    /// safe here would be flaky in the direction that reads as a real defect.
    async fn await_serve(&self) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            let heard = self.heard();
            if heard.iter().any(|line| line.starts_with("serve ")) {
                return heard;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "this runner never asked anything to serve, so nothing here was tested.\n\
             helper heard: {:?}\n\
             daemon stderr:\n{}",
            self.heard(),
            std::fs::read_to_string(&self.stderr).unwrap_or_default()
        );
    }

    fn heard(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// A helper that is a shell script: it writes down every line it is given and
/// answers each one `ok`.
///
/// The same device `a_pairing_gives_the_runner_a_tunnel.rs` uses, minus the pid
/// tag — nothing here counts helpers, and one transcript in order is the whole
/// observation.
///
/// Written into this runner's own directory and named in its own environment,
/// so the two tests in this file do not share one.
fn fake_helper(dir: &Path, log: &Path) -> PathBuf {
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
    let mut file = std::fs::File::create(&script).expect("the fake helper is written");
    file.write_all(body.as_bytes()).expect("the fake helper is written");
    drop(file);
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the fake helper is executable");
    script
}

//! What the Mac reads off the CLI when a removal needs the typed name.
//!
//! `DaemonClient.removeWorktree` and `removeRoot` ask whether stderr carries
//! `code: confirmation-required`. Both commands used to turn the daemon's
//! refusal into a bare string, so that line was never printed, and a dirty
//! worktree could not be removed from the Mac at all while every Swift test,
//! fed a hand-typed `code:` line, stayed green. This runs the real binary
//! against a real scratch daemon and reads what it really prints.
//!
//! Needs tmux, as every daemon test here does (CI installs it).
//!
//! The scratch daemon runs with `FARCOOLER_TEST_STUB_AGENTS`, as every
//! harness's does: its seat walk then takes this test for the person it stands
//! in for, whether it runs under an agent's pane or with no terminal at all
//! (CI, a landing gate), which a real daemon reads as unknown and refuses a
//! removal (ov-1296).
//!
//! The test starts that daemon itself, under an owner that outlives the test
//! (`OWNER`), so it stops however the test ends (ov-1522).

use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Scratch {
    home: PathBuf,
    /// The daemon's owner, once started. See `OWNER`.
    owner: Option<Child>,
}

/// Starts the daemon (`$1`), writes its pid to `$2`, and holds it for as long
/// as stdin stays open. Only the test holds the other end, so it closes when
/// `Scratch` drops and also when the test process dies without running a
/// destructor. Then this ends that one pid and removes the home (`$3`).
///
/// A process of its own, in a group of its own, because a cancelled or timed
/// out test is killed along with its group: nothing inside it gets to run.
const OWNER: &str = r#"
"$1" </dev/null >/dev/null 2>&1 &
daemon=$!
echo "$daemon" > "$2.tmp" && mv "$2.tmp" "$2"
read -r _
kill -TERM "$daemon" 2>/dev/null
tries=0
while kill -0 "$daemon" 2>/dev/null && [ "$tries" -lt 100 ]; do
    sleep 0.1
    tries=$((tries + 1))
done
kill -KILL "$daemon" 2>/dev/null
wait "$daemon"
rm -rf "$3"
"#;

/// The daemon built beside the CLI under test, or this channel's on PATH,
/// which is where the CLI itself would have looked.
fn daemon_binary() -> PathBuf {
    if let Some(named) = std::env::var_os("FARCOOLERD_BIN") {
        return named.into();
    }
    let beside = Path::new(env!("CARGO_BIN_EXE_farcooler")).with_file_name("farcoolerd");
    if beside.is_file() { beside } else { farcooler_protocol::CHANNEL.daemon_binary_name().into() }
}

impl Scratch {
    fn new() -> Scratch {
        // Short: the daemon's socket lives under it, and a unix socket path
        // has about a hundred bytes.
        let home = PathBuf::from(format!("/tmp/fcr-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        Scratch { home, owner: None }
    }

    fn env(&self, command: &mut Command) {
        command
            .env("FARCOOLER_HOME", self.home.join("h"))
            .env("FARCOOLER_CONFIG", self.home.join("config.toml"))
            .env("FARCOOLER_TEST_STUB_AGENTS", "1");
    }

    /// Start the daemon under its owner, and answer with its pid. It may not
    /// be listening yet.
    fn spawn_daemon(&mut self) -> String {
        let pidfile = self.home.join("daemon.pid");
        let mut owner = Command::new("/bin/sh");
        owner.args(["-c", OWNER, "daemon-owner"]).arg(daemon_binary()).arg(&pidfile).arg(&self.home);
        self.env(&mut owner);
        let owner = owner
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the daemon's owner");
        self.owner = Some(owner);
        assert!(within(Duration::from_secs(30), || pidfile.exists()), "the owner reported no pid");
        std::fs::read_to_string(pidfile).unwrap().trim().to_string()
    }

    /// Start the daemon and wait until it answers.
    fn start_daemon(&mut self) -> String {
        let pid = self.spawn_daemon();
        let socket = self.home.join("h").join("farcoolerd.sock");
        let up = within(Duration::from_secs(60), || !running(&pid) || UnixStream::connect(&socket).is_ok());
        assert!(up && running(&pid), "{} did not come up", daemon_binary().display());
        pid
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_farcooler"));
        self.env(&mut command);
        let out = command
            // The CLI starts a detached daemon when none answers, and nothing
            // would own that one. It gets a binary that isn't there.
            .env("FARCOOLERD_BIN", self.home.join("only-the-test-starts-the-daemon"))
            .args(args)
            .output()
            .expect("run farcooler");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Scratch {
    /// Closing the pipe is the whole request, and it is what happens by itself
    /// if this never runs. Waiting keeps the daemon from outliving the test.
    fn drop(&mut self) {
        if let Some(mut owner) = self.owner.take() {
            drop(owner.stdin.take());
            let _ = owner.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_dirty_worktree_and_a_wrong_root_name_are_refused_with_the_confirmation_word() {
    let mut s = Scratch::new();
    let repos = s.home.join("repos");
    let demo = repos.join("demo");
    std::fs::create_dir_all(&demo).unwrap();
    git(&demo, &["init", "-q"]);
    git(&demo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let engine = s.home.join("h/workflow/tree-wf1-cli-test");
    std::fs::create_dir_all(engine.parent().unwrap()).unwrap();
    git(&demo, &["worktree", "add", "-q", "-b", "engine", engine.to_str().unwrap()]);

    s.start_daemon();
    let (ok, _, e) = s.run(&["root", "add", repos.to_str().unwrap()]);
    assert!(ok, "{e}");
    let (ok, _, e) = s.run(&["repo", "register", demo.to_str().unwrap()]);
    assert!(ok, "{e}");
    let (ok, out, e) = s.run(&["--json", "worktree", "list"]);
    assert!(ok, "{e}");
    let listed: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let canonical_engine = engine.canonicalize().unwrap();
    let row = listed["worktrees"].as_array().unwrap().iter()
        .find(|row| row["worktree"].as_str() == canonical_engine.to_str()).unwrap();
    let engine_short = row["short"].as_str().unwrap();
    let (ok, _, e) = s.run(&["worktree", "remove", engine_short, "--confirm", "tree wf1 cli test"]);
    assert!(!ok, "an engine tree must survive manual removal");
    assert!(e.contains("managed by the workflow engine"), "{e}");
    assert!(engine.is_dir());
    assert_eq!(row["engine_owned"], true);
    let (ok, out, e) = s.run(&["worktree", "list"]);
    assert!(ok && out.contains("managed by the workflow engine"), "{out}\n{e}");

    let (ok, out, e) = s.run(&["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"]);
    assert!(ok, "{e}");
    let made: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let short = made["short"].as_str().unwrap().to_string();
    std::fs::write(PathBuf::from(made["worktree"].as_str().unwrap()).join("dirty.txt"), "x").unwrap();

    // The first call of the Mac's remove sheet: no name yet.
    let (ok, _, e) = s.run(&["--json", "worktree", "remove", &short]);
    assert!(!ok);
    assert!(e.lines().any(|l| l.trim() == "code: confirmation-required"), "no word in:\n{e}");

    // A root removed with the wrong name.
    let (_, roots, _) = s.run(&["root", "list"]);
    let root = roots.split_whitespace().next().unwrap().to_string();
    let (ok, _, e) = s.run(&["--json", "root", "remove", &root, "--confirm", "wrong"]);
    assert!(!ok);
    assert!(e.lines().any(|l| l.trim() == "code: confirmation-required"), "no word in:\n{e}");

    // And the right name goes through, so the word is the whole difference.
    let (ok, _, e) = s.run(&["--json", "worktree", "remove", &short, "--confirm", "wt1"]);
    assert!(ok, "{e}");
}

/// Alive, and not an exited process still waiting to be reaped.
fn running(pid: &str) -> bool {
    let out = Command::new("ps").args(["-p", pid, "-o", "stat="]).output().expect("run ps");
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Not a test of its own: the process the test below ends. It starts a scratch
/// daemon, reports its pid and home, and ends the way it's told to.
#[test]
#[ignore = "run only by every_way_a_test_ends_stops_its_daemon"]
fn daemon_owner_fixture() {
    let Ok(report) = std::env::var("FC_FIXTURE_REPORT") else { return };
    let ending = std::env::var("FC_FIXTURE_END").unwrap_or_default();
    let mut s = Scratch::new();
    // "early" fails while the daemon is still coming up.
    let pid = if ending == "early" { s.spawn_daemon() } else { s.start_daemon() };
    let pending = format!("{report}.tmp");
    std::fs::write(&pending, format!("{pid}\n{}", s.home.display())).unwrap();
    std::fs::rename(pending, &report).unwrap();
    if ending == "panic" || ending == "early" {
        panic!("the injected test failure");
    }
    // Held until the parent ends this process. If the parent dies instead,
    // stdin closes and this fails, which cleans up as any failure does.
    let _ = std::io::stdin().read_line(&mut String::new());
    panic!("the parent went away");
}

/// ov-1522: a daemon this file's test started outlived it and held a builder
/// slot for an hour. A failed test unwinds; a cancelled or timed-out one is
/// killed and runs no destructor, so each is tried on a real process.
#[test]
fn every_way_a_test_ends_stops_its_daemon() {
    let mut left = Vec::new();
    // `-KILL` on the group is how a runner ends a test and all it started.
    for (ending, signal) in [("panic", None), ("early", None), ("hold", Some(("-TERM", false))), ("hold", Some(("-KILL", false))), ("hold", Some(("-KILL", true)))] {
        let reports = tempfile::tempdir().unwrap();
        let report = reports.path().join("daemon");
        let mut fixture = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "daemon_owner_fixture", "--ignored", "--nocapture"])
            .env("FC_FIXTURE_REPORT", &report)
            .env("FC_FIXTURE_END", ending)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the fixture");
        let held = fixture.stdin.take();
        assert!(within(Duration::from_secs(90), || report.exists()), "{ending} {signal:?}: no daemon started");
        let told = std::fs::read_to_string(&report).unwrap();
        let (pid, home) = told.split_once('\n').unwrap();
        if let Some((signal, group)) = signal {
            let target = if group { format!("-{}", fixture.id()) } else { fixture.id().to_string() };
            assert!(Command::new("kill").args([signal, "--", &target]).status().unwrap().success());
        }
        assert!(!fixture.wait().unwrap().success());
        drop(held);
        if !within(Duration::from_secs(20), || !running(pid)) {
            left.push(format!("{ending} {signal:?}: daemon {pid} still running"));
            // The daemon this test's own fixture started, by its recorded pid.
            let _ = Command::new("kill").args(["-KILL", pid]).status();
        }
        if !within(Duration::from_secs(5), || !Path::new(home).exists()) {
            left.push(format!("{ending} {signal:?}: {home} was left behind"));
            let _ = std::fs::remove_dir_all(home);
        }
    }
    assert!(left.is_empty(), "{left:#?}");
}

fn within(bound: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

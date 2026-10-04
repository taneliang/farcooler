//! The mobile client core, against a real daemon.
//!
//! This is the code iOS and Android will run, so it is worth proving against
//! the actual daemon binary rather than a stub: the JSON shapes a phone decodes
//! have to match what a host actually sends, and a mock would agree with
//! whatever this file believed on the day it was written.
//!
//! The transport here is the local socket rather than SSH, because SSH is not
//! what can go wrong at this layer — it is a byte pipe, and `ssh.rs` puts an
//! `AsyncRead`/`AsyncWrite` on either end of it. What is under test is
//! everything above that: the handshake, the calls, and the shapes.

use std::path::PathBuf;

use farcooler_client::session::Session;

/// Where cargo put `farcoolerd`.
///
/// `CARGO_BIN_EXE_*` only covers binaries in the same crate, and the daemon
/// lives in another one. The test executable sits in `target/<profile>/deps/`,
/// so the binary is two levels up — which is a cargo layout detail, but a
/// stable one, and the alternative is a dev-dependency cycle between these two
/// crates.
fn daemon_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop(); // deps/
    path.pop(); // <profile>/
    path.push("farcoolerd");
    assert!(
        path.is_file(),
        "no farcoolerd at {} — run `cargo build -p farcooler-daemon` first",
        path.display()
    );
    path
}

/// A daemon on a private socket with a private database.
struct Daemon {
    dir: tempfile::TempDir,
    socket: PathBuf,
    process: std::process::Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();

        // And the tmux server that daemon started, which killing the daemon does
        // not touch. It sits on a socket named after this runtime directory's
        // install id, so the moment the `TempDir` is deleted nothing on the
        // machine can work out what it was called: it stays up until the machine
        // is restarted, holding a session, a pane and an interactive shell.
        //
        // The last of four fixtures with this hole. Same guard as
        // `rpc_over_socket.rs`, `stdio_transport.rs` and the daemon's own
        // `test_support.rs`.
        let Ok(install) = std::fs::read_to_string(self.dir.path().join("install-id")) else {
            return;
        };
        let socket = format!("farcooler-{}", install.trim());
        farcooler_tmux::reap_server(&socket);
    }
}

async fn start() -> Daemon {
    spawn(false).await
}

/// A daemon whose `authorized_keys` is a scratch file rather than the
/// developer's own, with the path to that file.
///
/// `client.enroll` writes SSH keys into `~/.ssh/authorized_keys`, and the
/// daemon resolves that from `HOME` — so a test that did not move `HOME` would
/// be a test that can lock whoever ran the suite out of their own machine.
/// `FARCOOLER_HOME` does not cover it, deliberately: that is the daemon's state
/// directory, and the file sshd reads is not part of any state this program
/// owns.
///
/// The redirection is PROVEN before anything writes, by `the_scratch_file_is_
/// the_one_being_read`. A check afterwards would be an assertion about damage
/// already done.
async fn start_with_a_scratch_home() -> (Daemon, PathBuf) {
    let daemon = spawn(true).await;
    let authorized_keys = daemon.dir.path().join(".ssh").join("authorized_keys");
    (daemon, authorized_keys)
}

/// A daemon every agent launch of which runs a stand-in named by absolute
/// path, never a `claude` found by searching.
///
/// For the one test that opens an agent pane for a task: the daemon refuses a
/// task on a shell (`takes_a_task`), so the pane has to be an agent, and a test
/// must never start the developer's real Claude Code on a prompt.
///
/// **Not by PATH.** A pane runs `<login shell> -ilc 'claude …'`, and that
/// login shell's search order — `/etc/paths` ahead of anything inherited on
/// macOS, then its own config — is not this test's to control. So the daemon
/// is started with `FARCOOLER_STAND_IN_AGENT` naming the stand-in by absolute
/// path (see `agent_program` in `crates/daemon/src/service.rs`), and the
/// launch never says a bare `claude` at all.
///
/// **And proven, not assumed.** The scratch `HOME` gives every login shell a
/// config that puts a TRAP directory first on its PATH, holding a `claude`
/// that only writes a marker. `bare_claude_resolves_to` shows a bare
/// `claude` from that shell would have hit it; the test then asserts the trap
/// never ran and the stand-in did.
///
/// The absolute path and the trap are the protection, and the only one that
/// matters. The daemon also starts without any variable an agent reads
/// credentials or config from (`stripped_agent_environment`), so nothing in
/// the ENVIRONMENT would sign a stray agent in — but Claude Code on macOS
/// keeps its sign-in in the login Keychain, which no HOME swap or strip
/// reaches, so a real `claude` started by a regression would still be signed
/// in. That is why the launch must never search for it.
async fn start_with_a_stand_in_agent() -> Daemon {
    spawn_with(true, Some(Launch::StandIn)).await
}

/// A daemon fenced off from the real agent exactly as
/// `start_with_a_stand_in_agent`'s is (the scratch HOME, the trap first on
/// every login shell's PATH, the stripped environment) but with no stand-in
/// named: its agent launches are left to `FARCOOLER_TEST_STUB_AGENTS`, which
/// every daemon here is started with.
async fn start_with_only_the_stub_switch() -> Daemon {
    spawn_with(true, Some(Launch::StubOnly)).await
}

/// A daemon like `start_with_a_stand_in_agent`'s, whose stand-in runs `body`
/// (a `/bin/sh` script, after the shebang) instead of only leaving its marker.
/// `@OUT@` in `body` is the daemon's scratch directory.
async fn spawn_with_stand_in(body: &str) -> Daemon {
    spawn_with(true, Some(Launch::StandInRunning(body.to_string()))).await
}

/// What a trapped daemon's agent launches run. See `spawn_with`.
enum Launch {
    /// `FARCOOLER_STAND_IN_AGENT` names the stand-in.
    StandIn,
    /// The same, with the stand-in running this body.
    StandInRunning(String),
    /// Nothing but `FARCOOLER_TEST_STUB_AGENTS`.
    StubOnly,
}

/// Variables removed from the stand-in daemon's environment, and so from every
/// pane's: everything a real agent reads credentials, a provider or a config
/// directory from — by prefix, so a provider this file has never heard of
/// under a known prefix goes too — and the ones that point a login shell at a
/// config other than the scratch HOME's (`ZDOTDIR`, `BASH_ENV`, `ENV`,
/// `XDG_CONFIG_HOME`), which would put the user's own PATH ahead of the trap.
fn stripped_agent_environment() -> Vec<std::ffi::OsString> {
    const PREFIXES: &[&str] = &[
        "ANTHROPIC_", "CLAUDE_", "OPENAI_", "CURSOR_", "CODEX_", "AWS_", "GOOGLE_", "VERTEX_",
        "BEDROCK_",
    ];
    const NAMES: &[&str] = &["CLOUD_ML_REGION", "XDG_CONFIG_HOME", "ZDOTDIR", "BASH_ENV", "ENV"];
    std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| {
            let name = name.to_string_lossy();
            PREFIXES.iter().any(|p| name.starts_with(p)) || NAMES.contains(&name.as_ref())
        })
        .collect()
}

/// The stand-in, the trap, and the two markers, under a daemon's scratch dir.
struct StandIn {
    program: PathBuf,
    ran: PathBuf,
    trap_dir: PathBuf,
    trapped: PathBuf,
}

impl StandIn {
    fn under(dir: &std::path::Path) -> StandIn {
        StandIn {
            program: dir.join("stand-in").join("claude"),
            ran: dir.join("stand-in-ran"),
            trap_dir: dir.join("trap-bin"),
            trapped: dir.join("REAL-CLAUDE-WAS-RESOLVED"),
        }
    }

    /// Write the stand-in, the trap, and a login-shell config for every shell
    /// this could be that puts the trap first.
    ///
    /// `body` is the stand-in's script after its shebang; `None` is the one
    /// that only leaves its marker and sleeps.
    fn install(&self, home: &std::path::Path, body: Option<&str>) {
        use std::os::unix::fs::PermissionsExt;
        let exec = |path: &std::path::Path, body: String| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let body = match body {
            Some(body) => body.replace("@OUT@", &home.display().to_string()),
            None => "exec sleep 60\n".to_string(),
        };
        exec(&self.program, format!("#!/bin/sh\ntouch '{}'\n{body}", self.ran.display()));
        exec(
            &self.trap_dir.join("claude"),
            format!("#!/bin/sh\ntouch '{}'\nexit 97\n", self.trapped.display()),
        );
        let trap = self.trap_dir.display();
        for (path, line) in [
            (".config/fish/conf.d/00-trap.fish", format!("set -gx PATH '{trap}' $PATH\n")),
            (".zshenv", format!("export PATH='{trap}':$PATH\n")),
            (".zprofile", format!("export PATH='{trap}':$PATH\n")),
            (".bash_profile", format!("export PATH='{trap}':$PATH\n")),
            (".profile", format!("export PATH='{trap}':$PATH\n")),
        ] {
            let path = home.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, line).unwrap();
        }
    }
}

/// The PATH the stand-in daemon runs with: the trap first, then what tmux and
/// git need. The real agent's usual homes are left on it deliberately — the
/// test is that nothing searches them, not that they are missing.
fn stand_in_path(stand_in: &StandIn) -> String {
    format!("{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin", stand_in.trap_dir.display())
}

async fn spawn(scratch_home: bool) -> Daemon {
    spawn_with(scratch_home, None).await
}

/// `trapped`, when given, installs the trap and the stand-in under the
/// daemon's directory and starts it in their environment; only
/// `Launch::StandIn` also names the stand-in.
async fn spawn_with(scratch_home: bool, trapped: Option<Launch>) -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("farcoolerd.sock");

    let mut command = std::process::Command::new(daemon_binary());
    command
        .env("FARCOOLER_HOME", dir.path())
        // Never the real agent: the daemon stubs every launch and refuses
        // one it cannot vouch for (`agent_program`). A stand-in, below,
        // still wins over the stub.
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if scratch_home {
        command.env("HOME", dir.path());
    }
    if let Some(launch) = trapped {
        let stand_in = StandIn::under(dir.path());
        let body = match &launch {
            Launch::StandInRunning(body) => Some(body.as_str()),
            _ => None,
        };
        stand_in.install(dir.path(), body);
        for name in stripped_agent_environment() {
            command.env_remove(name);
        }
        command.env("PATH", stand_in_path(&stand_in));
        if matches!(launch, Launch::StandIn | Launch::StandInRunning(_)) {
            command.env("FARCOOLER_STAND_IN_AGENT", &stand_in.program);
        }
    }
    let process = command.spawn().expect("spawn farcoolerd");

    // Wait for the socket rather than sleeping a fixed amount: a slow machine
    // would otherwise make this flaky and a fast one would waste the time.
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&socket).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    Daemon { dir, socket, process }
}

/// What a bare `claude` resolves to in the login shell a pane runs. The
/// control for the trap: if this is not the trap, "the trap never ran" would
/// prove nothing.
///
/// In the environment the pane gets, built the way the daemon's is — this
/// process's, minus `stripped_agent_environment`, with the scratch HOME and
/// the fixture's PATH — and NOT a clean one. A clean control would answer
/// "trap" for a machine whose exported `ZDOTDIR` or `XDG_CONFIG_HOME` points
/// the pane's shell at the user's own config, where the trap is not first.
fn bare_claude_resolves_to(daemon: &Daemon, stand_in: &StandIn) -> String {
    let shell = farcooler_core::shell::login_shell();
    let mut probe = std::process::Command::new(&shell);
    probe.args(["-ilc", "command -v claude"]);
    for name in stripped_agent_environment() {
        probe.env_remove(name);
    }
    probe.env("HOME", daemon.dir.path()).env("PATH", stand_in_path(stand_in));
    let out = probe.output().expect("run the login shell");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_client_connects_and_learns_the_daemon_version() {
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(!session.daemon_version().is_empty());
}

/// **A runner says when its agents run a stand-in** (ov-20 R-M3), over the
/// real wire: the daemon started with `FARCOOLER_STAND_IN_AGENT` names the
/// program in `Host`, and one started without it names nothing. A value leaked
/// into a real daemon's environment was otherwise visible only in its log.
#[tokio::test]
async fn a_runner_says_when_its_agents_run_a_stand_in() {
    let daemon = start_with_a_stand_in_agent().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    let host = session.host().await.expect("host");
    assert!(host.stand_in_agent.starts_with('/'), "no stand-in named: {:?}", host.stand_in_agent);
    // And every agent reads as one it can start (ov-205): the stand-in answers
    // each launch, so no harness is greyed out on a runner that runs one.
    assert!(session.can(farcooler_protocol::capability::AGENTS_FOUND));
    assert_eq!(host.agents_found, ["claude", "codex", "cursor-agent"]);

    let plain = start().await;
    let mut session = Session::connect_local(&plain.socket).await.expect("connect");
    let host = session.host().await.expect("host");
    assert_eq!(host.stand_in_agent, "", "a daemon with no stand-in named one");
}

#[tokio::test]
async fn a_client_learns_what_the_runner_can_do_before_asking_it_anything() {
    // The mechanism a newer app uses to degrade against an older runner. It
    // has to be answered by the handshake rather than by a call, because the
    // app decides what to draw before it has made one.
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");

    assert!(session.can(farcooler_protocol::capability::WORKTREES));
    assert!(session.can(farcooler_protocol::capability::CHANGES));
    assert!(!session.can("time-travel"), "a runner must not claim what it cannot do");
}

#[tokio::test]
async fn the_fleet_shape_is_the_one_a_phone_decodes() {
    // These key names are the app's contract. Changing one breaks a client that
    // cannot be updated at the same moment, which is the whole hazard of having
    // a phone in the picture.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let fleet = session.fleet().await.expect("fleet");
    assert!(fleet.get("runtime_healthy").is_some_and(|v| v.is_boolean()));
    assert!(fleet.get("live_panes").is_some_and(|v| v.is_number()));
    assert!(fleet.get("worktrees").is_some_and(|v| v.is_array()));
}

#[tokio::test]
async fn a_daemon_that_goes_away_mid_session_reads_as_a_dropped_link() {
    // The case both phones had no answer for: a session that connected fine
    // and then stopped being a session. It has to be distinguishable from the
    // daemon refusing a request, because one of those is fixed by
    // reconnecting and the other is fixed by not sending it again.
    let mut daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    session.fleet().await.expect("the session works before the daemon goes away");

    daemon.process.kill().expect("kill");
    daemon.process.wait().expect("reap");

    let error = session.fleet().await.expect_err("a dead daemon cannot answer");
    assert!(
        error.is_disconnect(),
        "a closed socket has to read as a dropped link, not as a protocol error: {error}"
    );
}

#[tokio::test]
async fn a_worktree_created_through_the_client_comes_back_in_the_fleet() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    // A real repository, because worktree creation makes a real worktree.
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }

    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    assert_eq!(repositories.len(), 1, "the repository must be visible to a second session");

    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "phone task", "feat/phone", "HEAD", "", false)
        .await
        .expect("create_worktree");
    assert_eq!(worktree.task_name, "phone task");

    let fleet = session.fleet().await.expect("fleet");
    let worktrees = fleet["worktrees"].as_array().unwrap();
    // Two: the one just created, plus the main checkout that registering the
    // repository adopts automatically.
    assert_eq!(worktrees.len(), 2);
    let created =
        worktrees.iter().find(|w| w["task"] == "phone task").expect("created worktree present");
    assert_eq!(created["branch"], "feat/phone");
    // Derived, never stored — and a fresh worktree with no terminals is ready.
    assert_eq!(created["state"], "ready");
    assert!(created["terminals"].as_array().unwrap().is_empty());
    // Claimed for the repository's Main as it's made, not left Unclaimed:
    // a pane opened in an unclaimed worktree has no workspace, so nothing
    // done there would ever claim it.
    let main = fleet["workspaces"]
        .as_array()
        .expect("the fleet names its workstreams")
        .iter()
        .find(|w| w["is_main"] == true)
        .expect("a Main");
    assert_eq!(created["workspace"], main["id"], "{created}");
    assert_eq!(created["claim_source"], "explicit", "{created}");
}

#[tokio::test]
async fn hiding_and_unhiding_round_trips_through_the_client() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "reversible", "feat/rev", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&worktree.id);

    session.hide_worktree(id).await.expect("hide");
    let fleet = session.fleet().await.expect("fleet");
    let worktrees = fleet["worktrees"].as_array().unwrap();
    let reversible =
        worktrees.iter().find(|w| w["task"] == "reversible").expect("its own worktree present");
    assert_eq!(reversible["state"], "hidden");

    session.unhide_worktree(id).await.expect("unhide");
    let fleet = session.fleet().await.expect("fleet");
    let worktrees = fleet["worktrees"].as_array().unwrap();
    let reversible =
        worktrees.iter().find(|w| w["task"] == "reversible").expect("its own worktree present");
    assert_eq!(reversible["state"], "ready");
}

/// The push path, end to end, with a number on it.
///
/// This is the claim the whole event surface rests on: a change made on the
/// runner reaches a client in one round trip rather than at the next poll. It
/// is asserted as a LATENCY, not merely as an arrival, because "an event
/// arrives eventually" was already true of the three-second poll it replaces.
///
/// `worktree.hide` is the trigger because it announces synchronously in the
/// handler — see `crates/daemon/src/rpc.rs:1088` — so what is measured is the
/// path and not a reconcile pass's timer.
#[tokio::test]
async fn fleet_news_reaches_a_subscriber_in_a_round_trip_not_a_poll_interval() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "pushed", "feat/push", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&worktree.id);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let subscription = session
        .subscribe(std::sync::Arc::new(move |what| {
            let _ = tx.send(what);
        }))
        .await
        .expect("subscribe");

    // Drain whatever the reconcile pass has already said, so the measurement
    // below times the hide and nothing that happened before it.
    while tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await.is_ok() {}

    let started = std::time::Instant::now();
    session.hide_worktree(id).await.expect("hide");
    let news = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("fleet news must arrive without waiting out a poll interval")
        .expect("the subscription is still open");
    let elapsed = started.elapsed();

    assert_eq!(news, farcooler_client::session::FleetEvent::Fleet);
    // Well under the three-second poll this replaces. Not a tight bound: a
    // loaded CI runner took 653 ms here (2026-09-27), so 500 ms flaked. Two
    // seconds still fails a regression back to "it arrives on the next timer".
    assert!(elapsed.as_millis() < 2000, "fleet news took {elapsed:?}");
    eprintln!("fleet news arrived {elapsed:?} after the change");

    // And the subscription is still open afterwards: one event does not end it.
    session.unhide_worktree(id).await.expect("unhide");
    tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("a second change is pushed too")
        .expect("the subscription is still open");
    assert!(!subscription.is_finished());
}

#[tokio::test]
async fn subscribing_to_a_terminal_with_no_agent_session_is_empty_not_an_error() {
    // A client attaches to a PANE, not to a session. A terminal that has never
    // been in agent mode must answer "nothing yet" rather than fail, or the
    // UI cannot open a chat view before the first turn.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "agent test", "feat/agent-empty", "HEAD", "", false)
        .await
        .expect("create_worktree");
    let worktree_id = farcooler_client::session::uuid_of(&worktree.id);

    let terminal = session
        .create_terminal(worktree_id, "shell", "shell", false)
        .await
        .expect("create_terminal");
    let terminal_id = farcooler_client::session::uuid_of(&terminal.id);

    let batch = session.agent_subscribe(terminal_id, 0, 0).await.expect("subscribe succeeds");
    assert!(batch.events.is_empty());
}

#[tokio::test]
async fn a_failed_call_arrives_as_an_error_not_a_dropped_session() {
    // A phone on a train needs the session to survive a refusal; reconnecting
    // over SSH for every rejected request would be unusable.
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let missing = uuid::Uuid::now_v7();
    assert!(session.hide_worktree(missing).await.is_err());

    // Still usable.
    assert!(session.fleet().await.is_ok());
}

/// A real refusal, from a real daemon, still naming its reason by the time it
/// reaches the layer a phone reads.
///
/// `crates/core` maps every domain error onto a stable code and the daemon puts
/// that code on the wire — and then, until now, `From<ClientError> for
/// SessionError` collapsed it into `Protocol(message)` and the code was gone
/// one layer above the wire. Twenty-eight machine words crossed the FFI and
/// every one of them became the same generic apology, which is why both phones
/// ended up matching substrings of a Rust `Display` string.
///
/// Three different refusals rather than one, because the value is in TELLING
/// THEM APART: a phone that cannot distinguish "that folder can never be
/// allowlisted" from "that folder overlaps one you already added" has to
/// apologize the same way for both, and the two have opposite next moves.
#[tokio::test]
async fn a_refusal_keeps_the_reason_the_runner_named_it_by() {
    use farcooler_client::session::SessionError;

    let word = |e: SessionError| -> String {
        match e {
            SessionError::Refused { code, .. } => {
                farcooler_core::error::word_for(code).to_string()
            }
            other => panic!("expected a named refusal, got {other:?}"),
        }
    };

    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    // Nothing by that id. "Refresh, it's gone" — not "try again".
    let missing = uuid::Uuid::now_v7();
    let e = session.hide_worktree(missing).await.expect_err("no such worktree");
    assert_eq!(word(e), "not-found");

    // A location that can never be allowlisted. "Pick a folder inside it."
    //
    // `/etc` as well as `/usr`, and the second one is the whole end-to-end
    // point: `Service::add_root` canonicalizes BEFORE it asks
    // `reject_sensitive_root`, and on macOS `/etc` is a symlink, so the guard
    // is really handed `/private/etc`. This asks over a real socket, so it is
    // the resolved path being refused and not the typed one. `/usr` is the
    // same before and after canonicalizing and holds the other half.
    for system in ["/usr", "/etc"] {
        let e = session
            .add_repository_root(system)
            .await
            .expect_err("a system path is never allowlistable");
        assert_eq!(word(e), "sensitive-root", "{system} is a system path");
    }

    // A folder that overlaps one already added. Same screen, same control, and
    // an entirely different thing to do about it.
    let dir = tempfile::tempdir().unwrap();
    let inside = dir.path().join("nested");
    std::fs::create_dir(&inside).unwrap();
    session
        .add_repository_root(&dir.path().to_string_lossy())
        .await
        .expect("the outer folder is addable");
    let e = session
        .add_repository_root(&inside.to_string_lossy())
        .await
        .expect_err("nested inside a root already added");
    assert_eq!(word(e), "path-not-allowed");

    // The session survived all three, which is the property
    // `a_failed_call_arrives_as_an_error_not_a_dropped_session` guards and this
    // must not have broken.
    assert!(session.fleet().await.is_ok());
}

#[tokio::test]
async fn removing_a_clean_worktree_needs_no_typed_name() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "clean removal", "feat/clean-removal", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&worktree.id);

    use farcooler_client::actions::RemoveWorktreeOutcome;
    let outcome = session.remove_worktree(id, "").await.expect("remove");
    assert_eq!(outcome, RemoveWorktreeOutcome::Removed);
}

#[tokio::test]
async fn removing_a_dirty_worktree_needs_the_task_name_typed() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;

    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "dirty removal", "feat/dirty-removal", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&worktree.id);

    // Find the worktree on disk and dirty it. The daemon derives "dirty" from
    // git status, so this has to be a real uncommitted change, not a flag.
    let fleet = session.fleet().await.expect("fleet");
    let worktrees = fleet["worktrees"].as_array().unwrap();
    let created =
        worktrees.iter().find(|w| w["task"] == "dirty removal").expect("worktree present");
    let worktree_path = created["worktree"].as_str().expect("worktree path");
    std::fs::write(std::path::Path::new(worktree_path).join("untracked.txt"), "uncommitted")
        .unwrap();

    use farcooler_client::actions::RemoveWorktreeOutcome;
    let outcome = session.remove_worktree(id, "").await.expect("first attempt");
    assert_eq!(outcome, RemoveWorktreeOutcome::ConfirmationRequired);

    let outcome =
        session.remove_worktree(id, "dirty removal").await.expect("confirmed attempt");
    assert_eq!(outcome, RemoveWorktreeOutcome::Removed);
}

#[tokio::test]
async fn adding_a_root_and_registering_a_repository_round_trips() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }

    let root = session
        .add_repository_root(&dir.path().to_string_lossy())
        .await
        .expect("add_repository_root");
    assert_eq!(root.repository_count, 0);

    let registered = session
        .register_repository(&repo.to_string_lossy())
        .await
        .expect("register_repository");
    assert!(!registered.display_name.is_empty());

    let repositories = session.repositories().await.expect("repositories");
    assert_eq!(repositories.len(), 1);
}

/// Defect 1, from the side that shipped it.
///
/// The daemon's own coverage — `rpc_over_socket.rs` — builds the request by
/// hand: `request("repository_root.remove")` with a `TypedConfirmation` payload
/// attached in the test body. That proves the daemon's RULE, and it passes no
/// matter what any client sends, which is exactly how a client that sent no
/// payload at all went unnoticed. This one calls `Session::remove_repository_root`
/// — the function the FFI arm calls, which is the function iOS and Android call
/// — so the payload under test is the one a phone actually builds.
///
/// Before the fix it fails at the first `assert`, with the daemon answering
/// `InvalidArgument { what: "payload" }` before it looks at the scope, the root
/// or the name.
#[tokio::test]
async fn a_root_removed_through_the_client_is_actually_removed() {
    use farcooler_client::actions::RemoveRootOutcome;

    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let dir = tempfile::tempdir().unwrap();
    let watched = dir.path().join("watched");
    std::fs::create_dir(&watched).unwrap();
    let name = watched.file_name().unwrap().to_string_lossy().into_owned();

    session
        .add_repository_root(&watched.to_string_lossy())
        .await
        .expect("add_repository_root");
    let root = session.roots().await.expect("roots").pop().expect("one root");
    let id = uuid::Uuid::from_slice(&root.id).expect("a root id");

    // A near miss is refused, and comes back as the domain outcome rather than
    // as an error a UI would have to show raw. Removing a root revokes Far
    // Cooler's permission over a whole tree, so this must not go through.
    assert_eq!(
        session.remove_repository_root(id, "not-its-name").await.expect("a refusal, not a failure"),
        RemoveRootOutcome::NameDidNotMatch
    );
    assert_eq!(session.roots().await.expect("roots").len(), 1, "a near miss must remove nothing");

    assert_eq!(
        session.remove_repository_root(id, &name).await.expect("remove_repository_root"),
        RemoveRootOutcome::Removed
    );
    assert!(session.roots().await.expect("roots").is_empty(), "the root is still watched");

    // And nothing on disk was touched, which is the promise both apps' footers
    // make on this control's behalf.
    assert!(watched.exists(), "removing a root must not delete anything");
}

/// Defect 2: what this session may ask for, not what the fence says about it.
///
/// `crates/transport` has always computed this from the session's real grant and
/// sent it in `ServerHello`; nothing above the transport read it, so no FFI
/// client could tell `control` from `host_admin` and both phones had to ask and
/// report the refusal. The daemon's own `stdio_transport.rs` asserts the same
/// value off `client.server_hello()` directly — this asserts it through the
/// accessor an app reaches, which is the part that did not exist.
#[tokio::test]
async fn a_client_learns_what_this_session_may_ask_for() {
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");

    // A local socket session is the runner talking to itself.
    assert_eq!(session.granted_scope(), "host_admin");
}

// MARK: - Device enrollment
//
// The last step of the ceremony, and the only one that changes anything: a key
// a person approved on a screen becomes a line in the file sshd reads. Against
// a real daemon rather than a stub, because the entire feature IS that file's
// contents — a stub would agree with whatever this test believed on the day it
// was written, including about a shape no daemon ever produces.

/// A device's public key, as a phone's `farcooler_client_generate_key` emits
/// one. A real ed25519 key: the daemon rebuilds the line from decoded key
/// material, so a plausible-looking string enrolls nothing.
const A_DEVICE_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 iPhone";

/// A Mac's SECOND key — Key B, the one Zed, git and Terminal offer.
///
/// A different key from `A_DEVICE_KEY`, which is the point rather than an
/// incidental detail: sshd matches a key against the file and takes the FIRST
/// line that matches, so one key written both plain and restricted would make
/// "does this device get a shell" a question about line order in a text file.
/// The daemon refuses that, and a Mac has two keys so that it never has to ask.
const A_MAC_S_SHELL_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOGnBC7LVPZ04GZqOGcT1pAAac8+IPuzrJazcZqkAv1P shell";

/// A key somebody added to their own `authorized_keys` by hand, inside the
/// block. Far Cooler carries it through every write and reports it as foreign.
///
/// The marker text is duplicated from `crates/fence/src/lib.rs` rather than
/// imported: this crate does not depend on the daemon, and a test that reached
/// for the constant would be asserting that the constant equals itself. What
/// matters is that a file written with THESE bytes is one the daemon reads.
const FENCE_BEGIN: &str = "# BEGIN FAR COOLER — do not edit inside this block";
const FENCE_END: &str = "# END FAR COOLER";
const A_HAND_WRITTEN_LINE: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ \
     scratch-home-marker";

/// Prove the daemon is reading the scratch file, before anything writes to one.
///
/// Not politeness: if `HOME` were not honoured, every test below would enroll
/// and revoke keys in the `authorized_keys` that decides whether the person
/// running the suite can still log in to their own machine. So the file is
/// planted with a line only this test knows, and `client.list` has to come back
/// holding it.
async fn the_scratch_file_is_the_one_being_read(session: &mut Session, path: &std::path::Path) {
    let ssh = path.parent().expect("a parent directory");
    std::fs::create_dir_all(ssh).expect("create .ssh");
    std::fs::write(path, format!("{FENCE_BEGIN}\n{A_HAND_WRITTEN_LINE}\n{FENCE_END}\n"))
        .expect("plant the marker");

    let listed = session.enrolled_clients().await.expect("client.list");
    let clients = listed["clients"].as_array().expect("clients is an array");
    assert!(
        clients.iter().any(|c| c["label"] == "scratch-home-marker"),
        "the daemon is not reading {}, so a write would land in the developer's own \
         authorized_keys — refusing to go on",
        path.display()
    );
}

/// The shape three apps decode, and the file underneath it.
#[tokio::test]
async fn a_device_enrolled_through_the_client_lands_in_the_runner_s_own_file() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    let answer = session
        .enroll_client(A_DEVICE_KEY, "Ada's iPhone", "phone-7", "read", false, "")
        .await
        .expect("client.enroll");
    assert_eq!(answer["alreadyEnrolled"], false);

    // These key names are the app's contract; a rename breaks a phone that
    // cannot be updated in the same moment.
    let client = &answer["client"];
    assert_eq!(client["clientId"], "phone-7");
    assert_eq!(client["scope"], "read", "a word, because Swift and Kotlin have no enum for it");
    assert!(client["fingerprint"].as_str().unwrap().starts_with("SHA256:"));
    assert!(client["label"].as_str().unwrap().contains("Ada"), "{client}");
    assert_eq!(client["foreign"], false);
    assert!(client["enrolledAt"].as_i64().unwrap() > 0, "the one moment a time can be stamped");

    // The file is the authority, so the file is what is checked.
    let written = std::fs::read_to_string(&authorized_keys).expect("read back");
    assert!(written.contains("--client phone-7"), "{written}");
    assert!(written.contains("--scope read"), "{written}");
    assert!(written.contains("restrict,command="), "an enrolled key is a restricted key");
    assert!(written.contains("scratch-home-marker"), "a hand-written line was deleted");

    // And the listing agrees with it, foreign line included.
    let listed = session.enrolled_clients().await.expect("client.list");
    let clients = listed["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 2);
    let foreign = clients.iter().find(|c| c["foreign"] == true).expect("the hand-written line");
    assert_eq!(foreign["clientId"], "", "nothing in a foreign line names a device");
    assert_eq!(foreign["scope"], "unspecified", "and nothing in one grants anything");
}

/// **A Mac's two keys, and the thing that decides whether Zed works.**
///
/// This is the end of the Key B story and the only place it can be checked
/// honestly: what makes Zed able to open `ssh://box/path` is that sshd finds a
/// line for that key with NO forced command on it, so the shell it wants exists.
/// Every layer above — the block in `~/.ssh/config`, the `IdentityFile`, the
/// alias — is inert if this line is restricted, and a passing unit test about
/// JSON shapes would not notice.
///
/// Two calls, one client id, in order. Order is what this test is about — Key B
/// is only meaningful once Key A exists — and no longer a concurrency
/// requirement: `fence::update` holds the lock across the read, and
/// `rpc_over_socket.rs`'s `a_macs_two_enrollments_may_land_at_the_same_moment`
/// covers the concurrent case directly.
#[tokio::test]
async fn a_mac_enrolls_twice_and_gets_a_line_with_a_shell_behind_it() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    // Key A: restricted, at the scope the ceremony grants.
    let a = session
        .enroll_client(A_DEVICE_KEY, "MacBook Air", "mac-9", "control", false, "")
        .await
        .expect("Key A");
    assert_eq!(a["client"]["shellAccess"], false);
    assert_eq!(a["client"]["scope"], "control");

    // Key B: plain, host_admin, same client id.
    let b = session
        .enroll_client(A_MAC_S_SHELL_KEY, "MacBook Air", "mac-9", "host_admin", true, "")
        .await
        .expect("Key B");
    assert_eq!(b["client"]["shellAccess"], true, "the field Settings draws two rows from");
    assert_eq!(b["client"]["clientId"], "mac-9", "one device, so one id, so one revoke");
    assert_eq!(
        b["client"]["scope"], "unspecified",
        "a plain line carries no forced command, so there is nowhere to put a scope"
    );
    assert_ne!(
        a["client"]["fingerprint"], b["client"]["fingerprint"],
        "one key in both shapes would make a shell a question about line order"
    );

    // The file is the authority. The plain line is the whole feature: no
    // `restrict`, no `command=`, so sshd offers a shell and Zed, git and Terminal
    // work over the alias `~/.ssh/config` names.
    let written = std::fs::read_to_string(&authorized_keys).expect("read back");
    let key_b_line = written
        .lines()
        .find(|line| line.contains("farcooler-shell-"))
        .expect("a plain line marked as ours");
    assert!(
        !key_b_line.contains("restrict") && !key_b_line.contains("command="),
        "Key B carries a forced command, so sshd has no shell to offer and Zed is locked out: \
         {key_b_line}"
    );
    assert!(key_b_line.starts_with("ssh-ed25519 "), "an options field would precede the key type");
    assert!(key_b_line.ends_with(".mac-9"), "the comment is what lets one revoke find it");
    assert!(
        written.lines().any(|l| l.contains("--client mac-9") && l.contains("--scope control")),
        "Key A's restricted line is gone: {written}"
    );

    // Both rows are on screen, told apart by the one field that can tell them
    // apart — and the hand-written line is still there beside them.
    let listed = session.enrolled_clients().await.expect("client.list");
    let clients = listed["clients"].as_array().unwrap();
    let ours: Vec<_> = clients.iter().filter(|c| c["clientId"] == "mac-9").collect();
    assert_eq!(ours.len(), 2, "a Mac is two lines under one id: {listed}");
    assert_eq!(ours.iter().filter(|c| c["shellAccess"] == true).count(), 1);
    assert!(ours.iter().all(|c| c["foreign"] == false), "both lines are ours and both are managed");

    // And one revoke takes both, which is what the removal copy promises.
    let left = session.revoke_client("mac-9").await.expect("client.revoke");
    assert!(left["clients"].as_array().unwrap().iter().all(|c| c["clientId"] != "mac-9"));
    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!written.contains("farcooler-shell-"), "Key B outlived the revoke: {written}");
    assert!(!written.contains("--client mac-9"));
    assert!(written.contains("scratch-home-marker"), "a hand-written line was deleted");
}

/// A shell line is refused at any scope but host_admin, from this side too.
///
/// The daemon owns this rule and `enroll_client` deliberately does NOT re-check
/// it — one rule, in the place that writes the file. This is the test that the
/// arrangement actually refuses, rather than a client-side copy quietly doing the
/// refusing while the daemon would have accepted.
#[tokio::test]
async fn a_plain_line_is_refused_at_any_scope_but_host_admin() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    for scope in ["read", "control"] {
        assert!(
            session
                .enroll_client(A_MAC_S_SHELL_KEY, "MacBook Air", "mac-9", scope, true, "")
                .await
                .is_err(),
            "a shell at {scope} does not agree with itself and was accepted"
        );
    }
    // And a refusal is not a dropped link.
    assert!(session.enrolled_clients().await.is_ok());
    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!written.contains("farcooler-shell-"), "a refused pair wrote a plain line: {written}");
}

/// Enrolling twice reports the grant that is already there, and writes nothing.
///
/// Not an error: it is the ordinary outcome of a ceremony offered a runner the
/// device can already reach. What it must never be is a second line for one
/// key, or a silent widening of an existing device's access.
#[tokio::test]
async fn enrolling_a_device_that_is_already_enrolled_reports_the_grant_it_has() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    session
        .enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "read", false, "")
        .await
        .expect("first enrollment");
    let again = session
        .enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "host_admin", false, "")
        .await
        .expect("second enrollment");

    assert_eq!(again["alreadyEnrolled"], true);
    assert_eq!(
        again["client"]["scope"], "read",
        "the scope it HAS, never the one that was asked for"
    );
    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert_eq!(written.matches("--client phone-7").count(), 1, "two lines for one device");
}

/// Revoking answers with what is left, read back out of the file.
#[tokio::test]
async fn revoking_a_device_removes_its_line_and_answers_with_the_rest() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "read", false, "").await.expect("enroll");

    let remaining = session.revoke_client("phone-7").await.expect("client.revoke");
    let clients = remaining["clients"].as_array().unwrap();
    assert!(clients.iter().all(|c| c["clientId"] != "phone-7"), "{remaining}");
    assert_eq!(clients.len(), 1, "the hand-written line survives a revocation");

    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!written.contains("--client phone-7"));
    assert!(written.contains("scratch-home-marker"));

    // Revoking what is not there is NOT a cheerful success: "revoked" from a
    // runner that revoked nothing is the one answer a person must never be
    // given about a device they are trying to cut off.
    assert!(session.revoke_client("phone-7").await.is_err());
}

/// A scope this build does not have is refused before the request is sent.
///
/// Refused rather than defaulted, for the reason the daemon refuses an
/// unspecified one: a key with no scope means host_admin to sshd, so rounding a
/// typo up would turn a misspelling into the whole runner.
#[tokio::test]
async fn a_scope_word_nobody_has_is_refused_rather_than_guessed_at() {
    let (daemon, authorized_keys) = start_with_a_scratch_home().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    the_scratch_file_is_the_one_being_read(&mut session, &authorized_keys).await;

    assert!(session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "admin", false, "").await.is_err());
    assert!(session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "", false, "").await.is_err());

    // And the session is still usable, because a refusal is not a dropped link.
    assert!(session.enrolled_clients().await.is_ok());
    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!written.contains("--client phone-7"), "a refused scope enrolled something");
}

/// A board read through the client is the board the Mac reads through the
/// CLI, and a pane opened for a task says which task in the fleet.
///
/// Against the real daemon because both halves of the phone's board are
/// shapes someone else decodes: `TaskBoardModel.decode` reads `task.list`, and
/// `TaskRow.livePanes` matches `taskId` against a row's `id`. A stub would
/// agree with whatever this file believed.
#[tokio::test]
async fn a_board_and_the_pane_working_it_come_back_through_the_client() {
    use farcooler_protocol::v1::request::Payload;

    let daemon = start_with_a_stand_in_agent().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(session.can(farcooler_protocol::capability::TASKS), "this daemon keeps a board");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;
    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);

    // An empty board is an empty list, not an error: the phone reads every
    // repository and draws a row only for the ones with something on them.
    let empty = session.tasks(repository, None).await.expect("an empty board");
    assert_eq!(empty["tasks"], serde_json::json!([]));

    let mut raw = raw_client(&daemon.socket).await;
    let mut create = farcooler_transport::request("task.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(repository.as_bytes()));
    create.payload = Some(Payload::TaskCreate(farcooler_protocol::v1::TaskCreate {
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        title: "A board on the phone".into(),
        intent: "so the phone can see it".into(),
        acceptance: vec![
            farcooler_protocol::v1::TaskAcceptanceItem { text: "one".into(), ..Default::default() },
            farcooler_protocol::v1::TaskAcceptanceItem { text: "two".into(), ..Default::default() },
        ],
        actor: "user".into(),
        ..Default::default()
    }));
    let created = raw.call(create).await.expect("task.create");
    let Some(farcooler_protocol::v1::result::Value::Task(task)) = created.value else {
        panic!("task.create answered with something else");
    };
    let task_id = farcooler_client::session::uuid_of(&task.id);

    let board = session.tasks(repository, None).await.expect("task.list");
    let rows = board["tasks"].as_array().expect("tasks");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], task_id.to_string());
    assert_eq!(rows[0]["key"], task.key);
    assert_eq!(rows[0]["title"], "A board on the phone");
    assert_eq!(rows[0]["acceptance"].as_array().map(Vec::len), Some(2));
    assert!(rows[0]["status"].is_string());

    let detail = session.task(task_id).await.expect("task.get");
    assert_eq!(detail["task"]["id"], task_id.to_string());
    assert!(detail["notes"].is_array());
    assert!(detail["blocks"].is_array());

    // The card's time line, through the phone's own route. Filed with an
    // intent and acceptance, which `task.create` writes as a second store
    // call after the row -- and the card must still read "Added", so the two
    // clocks agree.
    let created_at = rows[0]["created_at"].as_i64().expect("task.list carries created_at");
    assert!(created_at > 0, "{created_at}");
    assert_eq!(rows[0]["updated_at"], created_at, "a card filed with an intent read as updated");
    assert_eq!(detail["task"]["created_at"], created_at, "task.get carries it too");
    assert_eq!(detail["task"]["updated_at"], created_at);

    // A note is a change to the card. Milliseconds apart at least, so
    // "later" cannot tie.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut note = farcooler_transport::request("task.note");
    note.payload = Some(Payload::TaskNoteAppend(farcooler_protocol::v1::TaskNoteAppend {
        task_id: task.id.clone(),
        kind: farcooler_protocol::v1::TaskNoteKind::Finding as i32,
        body: "the phone can see it".into(),
        actor: "user".into(),
        ..Default::default()
    }));
    let noted = raw.call(note).await.expect("task.note");
    let Some(farcooler_protocol::v1::result::Value::TaskNote(written)) = noted.value else {
        panic!("task.note answered with something else");
    };
    let board = session.tasks(repository, None).await.expect("task.list after a note");
    assert_eq!(board["tasks"][0]["created_at"], created_at, "creation does not move");
    assert_eq!(board["tasks"][0]["updated_at"], written.at, "a note moves updated_at");
    assert!(written.at > created_at);
    let detail = session.task(task_id).await.expect("task.get after a note");
    assert_eq!(detail["task"]["updated_at"], written.at);

    // A worktree with two panes: one opened for the task, one not.
    let worktree = session
        .create_worktree(repository, "board lane", "feat/board", "HEAD", "", false)
        .await
        .expect("create_worktree");
    let worktree_id = farcooler_client::session::uuid_of(&worktree.id);
    let mut open = farcooler_transport::request("terminal.create");
    open.target_resource_id = Some(bytes::Bytes::copy_from_slice(worktree_id.as_bytes()));
    open.required_capabilities =
        vec![farcooler_protocol::capability::TERMINAL_TASK.to_string()];
    open.payload = Some(Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
        title: "on the task".into(),
        command_preset: "claude".into(),
        task_key: Some(task.key.clone()),
        ..Default::default()
    }));
    let opened = raw.call(open).await.expect("terminal.create for a task");
    let Some(farcooler_protocol::v1::result::Value::Terminal(on_task)) = opened.value else {
        panic!("terminal.create answered with something else");
    };
    let plain = session
        .create_terminal(worktree_id, "not on it", "shell", false)
        .await
        .expect("a pane nobody dispatched");

    let fleet = session.fleet().await.expect("fleet");
    let terminals: Vec<&serde_json::Value> = fleet["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| w["terminals"].as_array().unwrap())
        .collect();
    let find = |id: &[u8]| {
        let id = farcooler_client::session::uuid_of(id).to_string();
        *terminals.iter().find(|t| t["id"] == id.as_str()).expect("the pane is in the fleet")
    };
    assert_eq!(find(&on_task.id)["taskId"], task_id.to_string());
    assert!(find(&plain.id)["taskId"].is_null(), "a pane nobody dispatched names no task");

    // What ran in the agent pane. A bare `claude` in this pane's login shell
    // would have been the trap (the control), so the stand-in running and the
    // trap not is the launch naming its program by absolute path.
    let stand_in = StandIn::under(daemon.dir.path());
    assert_eq!(
        bare_claude_resolves_to(&daemon, &stand_in),
        stand_in.trap_dir.join("claude").display().to_string(),
        "the trap is not first for a bare claude, so its silence would prove nothing"
    );
    for _ in 0..100 {
        if stand_in.ran.exists() || stand_in.trapped.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        !stand_in.trapped.exists(),
        "the agent pane resolved `claude` by searching, and found the trap"
    );
    assert!(stand_in.ran.exists(), "the agent pane never ran the stand-in");
}

/// **A decision reaches Needs You, and a phone's answer takes it away**
/// (ov-55 1C). Through the phone's own routes: a worktree made from a
/// workspace's screen is claimed for that workspace, one made from nowhere
/// for Main; the worktree names its open task; a QUESTION written through
/// `task_note` on a task in Needs Decision is a decision item on
/// `needs_you`; and the ANSWER, written as the user, is what removes it.
#[tokio::test]
async fn a_decision_answered_from_a_phone_leaves_needs_you() {
    use farcooler_client::session::{task_note_append, uuid_of};
    use farcooler_protocol::v1::request::Payload;
    use farcooler_protocol::v1::{TaskNoteKind, result::Value};

    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(session.can(farcooler_protocol::capability::NEEDS_YOU), "this daemon has the rollup");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;
    let repositories = session.repositories().await.expect("repositories");
    let repository = uuid_of(&repositories[0].id);

    let mut raw = raw_client(&daemon.socket).await;
    let mut create = farcooler_transport::request("workspace.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(repository.as_bytes()));
    create.payload = Some(Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
        name: "Billing".into(),
        task_prefix: "bil".into(),
    }));
    let Some(Value::Workspace(billing)) = raw.call(create).await.expect("workspace.create").value else {
        panic!("workspace.create answered with something else");
    };
    let main = session
        .workspaces()
        .await
        .expect("workspaces")
        .into_iter()
        .find(|w| w.is_main && w.repository_id == repositories[0].id)
        .expect("the repository's Main");

    let lane = session
        .create_worktree_in(repository, "billing lane", "feat/billing", "HEAD", "", false, Some(uuid_of(&billing.id)))
        .await
        .expect("a worktree made from Billing's screen");
    let plain = session
        .create_worktree(repository, "plain lane", "feat/plain", "HEAD", "", false)
        .await
        .expect("a worktree made from nowhere in particular");

    let mut create = farcooler_transport::request("task.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(repository.as_bytes()));
    create.payload = Some(Payload::TaskCreate(farcooler_protocol::v1::TaskCreate {
        repository_id: repositories[0].id.clone(),
        title: "Pick the queue".into(),
        worktree_id: Some(lane.id.clone()),
        workspace_id: Some(billing.id.clone()),
        actor: "user".into(),
        ..Default::default()
    }));
    let Some(Value::Task(task)) = raw.call(create).await.expect("task.create").value else {
        panic!("task.create answered with something else");
    };
    let task_id = uuid_of(&task.id);

    let fleet = session.fleet().await.expect("fleet");
    let row = |id: &[u8]| {
        fleet["worktrees"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == uuid_of(id).to_string().as_str())
            .cloned()
            .expect("the worktree is in the fleet")
    };
    assert_eq!(row(&lane.id)["workspace"], uuid_of(&billing.id).to_string(), "claimed for the workspace named");
    assert_eq!(row(&plain.id)["workspace"], uuid_of(&main.id).to_string(), "claimed for Main");
    assert_eq!(row(&lane.id)["open_tasks"][0]["key"], task.key, "{}", row(&lane.id));
    assert_eq!(row(&plain.id)["open_tasks"], serde_json::json!([]));

    // Into Needs Decision as an agent puts it there: the move, then the
    // question. A question alone moves nothing.
    let mut decide = farcooler_transport::request("task.set_status");
    decide.target_resource_id = Some(task.id.clone());
    decide.payload = Some(Payload::TaskSetStatus(farcooler_protocol::v1::TaskSetStatus {
        task_id: task.id.clone(),
        status: farcooler_protocol::v1::TaskStatus::NeedsDecision as i32,
        actor: "manager".into(),
    }));
    raw.call(decide).await.expect("task.set_status");
    let asked = session
        .task_note(task_note_append(task_id, TaskNoteKind::Question, "Postgres or SQLite?"))
        .await
        .expect("a question");
    assert_eq!((asked["kind"].as_str(), asked["actor"].as_str()), (Some("question"), Some("user")));

    let decision = format!("decision:{task_id}");
    let listed = session.needs_you().await.expect("needs_you");
    let item = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == decision.as_str())
        .unwrap_or_else(|| panic!("no decision item: {listed}"));
    assert_eq!(item["kind"], "decision");
    assert_eq!(item["question"], "Postgres or SQLite?");
    assert_eq!(item["task"]["key"], task.key);
    assert_eq!(item["workspace_id"], uuid_of(&billing.id).to_string());

    let answered = session
        .task_note(task_note_append(task_id, TaskNoteKind::Answer, "Postgres"))
        .await
        .expect("the answer");
    assert_eq!(answered["actor"], "user");
    let listed = session.needs_you().await.expect("needs_you after the answer");
    assert!(
        !listed["items"].as_array().unwrap().iter().any(|i| i["id"] == decision.as_str()),
        "the answered decision is still there: {listed}"
    );
}

/// **A daemon started with `FARCOOLER_TEST_STUB_AGENTS` never starts a real
/// agent** (ov-10). Integration tests build the daemon without `cfg(test)`,
/// so a `claude` pane here would run whatever `claude` its login shell
/// finds. With the switch, which every harness here sets, it runs the unit
/// tests' stub instead.
///
/// The `claude` a bare name would find is the trap, a fake that only leaves a
/// marker, and that is checked BEFORE anything is launched: if the trap were
/// not first, a regression would reach the developer's real Claude Code.
/// Take the switch out of the daemon and this goes red on the trap's marker.
#[tokio::test]
async fn a_daemon_under_the_stub_switch_never_starts_claude() {
    let daemon = start_with_only_the_stub_switch().await;
    let stand_in = StandIn::under(daemon.dir.path());
    assert_eq!(
        bare_claude_resolves_to(&daemon, &stand_in),
        stand_in.trap_dir.join("claude").display().to_string(),
        "the trap is not first for a bare claude, so launching one could start the real agent"
    );

    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;
    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "stub lane", "feat/stub", "HEAD", "", false)
        .await
        .expect("create_worktree");
    let worktree_id = farcooler_client::session::uuid_of(&worktree.id);
    let opened = session.create_terminal(worktree_id, "claude", "claude", false).await;

    // Until a process under the pane is the stub's `sleep` (its last word:
    // nothing after it can start claude) or the trap has run. The pane's
    // process OR one under it: with bash as the login shell, every shell in
    // the chain (tmux's `-c` wrapper, `env`, `bash -ilc`, the stub's `sh -c
    // exec`) runs a single command and so execs it, and the pane's own
    // process becomes `sleep`. fish forks instead and keeps the foreground,
    // so there `sleep` is a grandchild.
    let install = std::fs::read_to_string(daemon.dir.path().join("install-id")).expect("install id");
    let tmux = farcooler_core::programs::find("tmux").expect("tmux");
    let mut panes = String::new();
    let mut stub_running = false;
    for _ in 0..200 {
        if stand_in.trapped.exists() {
            break;
        }
        let out = std::process::Command::new(&tmux)
            .args(["-L", &format!("farcooler-{}", install.trim()), "list-panes", "-a", "-F"])
            .arg("#{pane_pid} #{pane_start_command}")
            .output()
            .expect("tmux list-panes");
        panes = String::from_utf8_lossy(&out.stdout).into_owned();
        let stubs: Vec<&str> = panes
            .lines()
            .filter(|l| l.contains("farcooler-test-stub-agent"))
            .filter_map(|l| l.split(' ').next())
            .collect();
        stub_running = stubs.iter().any(|pid| runs_under(pid, "sleep"));
        if stub_running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!stand_in.trapped.exists(), "the claude pane started `claude`, and found the trap");
    assert!(!stand_in.ran.exists(), "no stand-in was named, so none may run");
    opened.expect("a claude pane opens, on the stub");
    assert!(stub_running, "the claude pane is not running the stub: {panes}");
}

/// Whether process `pid`, or some descendant of it, is running `program`.
/// Compared by basename: `comm` is `/bin/sleep` on macOS and `sleep` on
/// Linux.
fn runs_under(pid: &str, program: &str) -> bool {
    let out = std::process::Command::new("ps").args(["-ax", "-o", "pid=,ppid=,comm="]).output().expect("ps");
    let table: Vec<(String, String, String)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            Some((w.next()?.to_string(), w.next()?.to_string(), w.collect::<Vec<_>>().join(" ")))
        })
        .collect();
    let is_program = |comm: &str| comm.rsplit('/').next() == Some(program);
    if table.iter().any(|(p, _, comm)| p == pid && is_program(comm)) {
        return true;
    }
    let mut parents = vec![pid.to_string()];
    while let Some(parent) = parents.pop() {
        for (child, ppid, comm) in &table {
            if *ppid == parent {
                if is_program(comm) {
                    return true;
                }
                parents.push(child.clone());
            }
        }
    }
    false
}

/// Two workstreams in one repository come back through the client apart: in
/// the fleet, on their boards, and in the news a board is keyed by.
///
/// Against the real daemon for the reason the board test above gives: the
/// fleet's `workspaces`, a worktree's `workspace` and `claim_source`, a
/// terminal's `workspace` and `role`, a task row's `workspace`, and a task
/// event's `workspace` and `from_workspace` are all shapes AgentKit and
/// Android decode, and a stub would agree with whatever this file believed.
#[tokio::test]
async fn two_workspaces_in_one_repository_come_back_apart_through_the_client() {
    use farcooler_client::session::{FleetEvent, uuid_of};
    use farcooler_protocol::capability::WORKSTREAMS;
    use farcooler_protocol::v1::request::Payload;
    use farcooler_protocol::v1::result::Value;

    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(session.can(WORKSTREAMS), "this daemon keeps workstreams");

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, dir.path(), &repo).await;
    let repositories = session.repositories().await.expect("repositories");
    let repository = uuid_of(&repositories[0].id);

    // Registering a repository gives it Main, and the fleet says so.
    let fleet = session.fleet().await.expect("fleet");
    let listed = fleet["workspaces"].as_array().expect("the fleet names its workstreams");
    assert_eq!(listed.len(), 1, "{fleet}");
    assert_eq!(listed[0]["is_main"], true);
    assert_eq!(listed[0]["repository"], repository.to_string());
    let main: uuid::Uuid = listed[0]["id"].as_str().unwrap().parse().unwrap();

    let mut raw = raw_client(&daemon.socket).await;
    let mut create = farcooler_transport::request("workspace.create");
    create.target_resource_id = Some(bytes::Bytes::copy_from_slice(repository.as_bytes()));
    create.required_capabilities = vec![WORKSTREAMS.to_string()];
    create.payload = Some(Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
        name: "Billing".into(),
        task_prefix: "bil".into(),
    }));
    let Some(Value::Workspace(made)) = raw.call(create).await.expect("workspace.create").value
    else {
        panic!("workspace.create answered with something else");
    };
    let billing = uuid_of(&made.id);

    // A worktree made for Billing is Billing's, and so is the shell in it.
    let mut lane = farcooler_transport::request("worktree.create");
    lane.target_resource_id = Some(bytes::Bytes::copy_from_slice(repository.as_bytes()));
    lane.required_capabilities = vec![WORKSTREAMS.to_string()];
    lane.payload = Some(Payload::WorktreeCreate(farcooler_protocol::v1::WorktreeCreate {
        task_name: "billing lane".into(),
        branch: "feat/billing".into(),
        base_revision: "HEAD".into(),
        terminal_preset: "shell".into(),
        workspace_id: Some(bytes::Bytes::copy_from_slice(billing.as_bytes())),
        ..Default::default()
    }));
    raw.call(lane).await.expect("worktree.create for Billing");

    let fleet = session.fleet().await.expect("fleet");
    let names: Vec<&str> =
        fleet["workspaces"].as_array().unwrap().iter().map(|w| w["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Main", "Billing"], "Main first, then the rest");
    let row = fleet["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["task"] == "billing lane")
        .expect("Billing's worktree is in the fleet");
    assert_eq!(row["workspace"], billing.to_string(), "{row}");
    assert_eq!(row["claim_source"], "explicit");
    assert_eq!(row["foreign_writers"], serde_json::json!([]));
    let shell = &row["terminals"][0];
    assert_eq!(shell["workspace"], billing.to_string(), "{shell}");
    assert_eq!(shell["role"], "shell");

    // A task filed on Billing is on Billing's board and not on Main's.
    let mut file = farcooler_transport::request("task.create");
    file.required_capabilities = vec![WORKSTREAMS.to_string()];
    file.payload = Some(Payload::TaskCreate(farcooler_protocol::v1::TaskCreate {
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        workspace_id: Some(bytes::Bytes::copy_from_slice(billing.as_bytes())),
        title: "Invoice run".into(),
        actor: "user".into(),
        ..Default::default()
    }));
    let Some(Value::Task(task)) = raw.call(file).await.expect("task.create").value else {
        panic!("task.create answered with something else");
    };
    assert!(task.key.starts_with("bil-"), "{}", task.key);

    let on_billing = session.tasks(repository, Some(billing)).await.expect("Billing's board");
    assert_eq!(on_billing["tasks"].as_array().map(Vec::len), Some(1), "{on_billing}");
    assert_eq!(on_billing["tasks"][0]["workspace"], billing.to_string());
    let on_main = session.tasks(repository, Some(main)).await.expect("Main's board");
    assert_eq!(on_main["tasks"], serde_json::json!([]), "Billing's task is on Main's board");
    let whole = session.tasks(repository, None).await.expect("the repository");
    assert_eq!(whole["tasks"].as_array().map(Vec::len), Some(1));

    // Moving it to Main is news that names both boards.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _subscription = session
        .subscribe(std::sync::Arc::new(move |what| {
            let _ = tx.send(what);
        }))
        .await
        .expect("subscribe");
    while tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await.is_ok() {}

    let mut shift = farcooler_transport::request("task.move");
    shift.required_capabilities = vec![WORKSTREAMS.to_string()];
    shift.payload = Some(Payload::TaskMove(farcooler_protocol::v1::TaskMove {
        task_ids: vec![task.id.clone()],
        workspace_id: bytes::Bytes::copy_from_slice(main.as_bytes()),
        actor: "user".into(),
    }));
    raw.call(shift).await.expect("task.move");

    let moved = loop {
        let news = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a move is news")
            .expect("the subscription is still open");
        if let FleetEvent::Task { .. } = news {
            break news;
        }
    };
    assert_eq!(
        moved,
        FleetEvent::Task {
            repository,
            workspace: Some(main),
            from_workspace: Some(billing),
            actor: "user".into(),
        }
    );
    let on_main = session.tasks(repository, Some(main)).await.expect("Main's board");
    assert_eq!(on_main["tasks"][0]["workspace"], main.to_string(), "{on_main}");
}

/// A raw protocol client on the daemon's socket, for the writes the phone's
/// session does not make.
async fn raw_client(
    socket: &std::path::Path,
) -> farcooler_transport::Client<
    Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
> {
    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (read, write) = stream.into_split();
    farcooler_transport::Client::over(
        Box::new(read) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box::new(write) as Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        "test",
        "0.0.0",
    )
    .await
    .unwrap()
}

/// Add a root and register a repository, over a throwaway session.
async fn register_root_and_repository(
    socket: &std::path::Path,
    root: &std::path::Path,
    repo: &std::path::Path,
) {
    use farcooler_protocol::v1::request::Payload;

    let stream = tokio::net::UnixStream::connect(socket).await.unwrap();
    let (read, write) = stream.into_split();
    let mut client = farcooler_transport::Client::over(
        Box::new(read) as Box<dyn tokio::io::AsyncRead + Unpin + Send>,
        Box::new(write) as Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
        "test",
        "0.0.0",
    )
    .await
    .unwrap();

    let mut add = farcooler_transport::request("repository_root.add");
    add.payload = Some(Payload::RepositoryRootAdd(farcooler_protocol::v1::RepositoryRootAdd {
        absolute_path: root.to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("root add");

    let mut register = farcooler_transport::request("repository.register");
    register.payload = Some(Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo.to_string_lossy().into_owned(),
        },
    ));
    client.call(register).await.expect("register");
}

// ---- A claude TUI's permission, answered from a phone (ov-14) --------------

/// A stand-in claude that asks one permission the way claude 2.1.283 does.
///
/// Launched as `<stand-in> claude --session-id S --settings F …`. It:
/// - takes its `PermissionRequest` hook out of `F`, the settings Far Cooler
///   wrote for it (pretty-printed JSON with the command on a line of its own);
/// - draws the spike's dialog, banner included, so the watcher knows it is
///   claude;
/// - runs the hook in the background, the way claude runs it while its dialog
///   is up, with the payload claude sends, and keeps what the hook printed;
/// - answers at the keyboard on a line of input, which takes the dialog down
///   and draws the working footer, as claude does.
const ASKING_CLAUDE: &str = r#"
OUT='@OUT@'
S=; F=
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) S=$2; shift ;;
    --settings) F=$2; shift ;;
  esac
  shift
done
cmd=$(grep -e '--event PermissionRequest' "$F" | sed -e 's/^ *"command": "//' -e 's/",*$//')
printf '%s
' "$cmd" > "$OUT/hook-command"
clear
cat "$OUT/dialog.txt"
(
  printf '{"session_id":"%s","cwd":"%s","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"touch x"}}' "$S" "$PWD"     | sh -c "$cmd" > "$OUT/hook-stdout" 2> "$OUT/hook-stderr"
  touch "$OUT/hook-exited"
  if [ -s "$OUT/hook-stdout" ]; then
    clear
    printf '  ⎿  Answered by PermissionRequest hook
'
  fi
) &
read answer
clear
printf '  <spinner>
❯ 
  ⏸ manual mode on · esc to interrupt
'
touch "$OUT/tui-answered"
exec sleep 600
"#;

/// A daemon running `ASKING_CLAUDE` in a `claude` pane, the pane, and the id
/// of the ask its hook is being held on.
struct Asking {
    daemon: Daemon,
    session: Session,
    terminal: uuid::Uuid,
    id: String,
    _repo: tempfile::TempDir,
}

impl Asking {
    fn out(&self, name: &str) -> PathBuf {
        self.daemon.dir.path().join(name)
    }

    /// Every event in the pane's ring, as JSON.
    async fn ring(&mut self) -> Vec<serde_json::Value> {
        let batch = self.session.agent_subscribe(self.terminal, 0, 0).await.expect("subscribe");
        batch
            .events
            .iter()
            .map(|e| serde_json::from_str(&e.payload_json).expect("an event is json"))
            .collect()
    }

    /// Waits for the ring to end in this ask's `Resolved`, and returns what
    /// was chosen.
    async fn resolved(&mut self) -> String {
        for _ in 0..200 {
            let ring = self.ring().await;
            if let Some(last) = ring.last() {
                if last["Resolved"]["id"] == self.id.as_str() {
                    return last["Resolved"]["chosen"].as_str().expect("chosen").to_string();
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("the ring never ended in this ask's Resolved: {:?}", self.ring().await);
    }

    /// Waits for `name` under the daemon's directory to exist.
    async fn exists(&self, name: &str, within: std::time::Duration) -> bool {
        let until = std::time::Instant::now() + within;
        while std::time::Instant::now() < until {
            if self.out(name).exists() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    fn hook_printed(&self) -> String {
        std::fs::read_to_string(self.out("hook-stdout")).unwrap_or_default()
    }

    /// The trap never ran: nothing searched for a real `claude`.
    fn never_trapped(&self) {
        let stand_in = StandIn::under(self.daemon.dir.path());
        assert!(stand_in.ran.exists(), "the pane never ran the stand-in");
        assert!(!stand_in.trapped.exists(), "the pane searched for claude and found the trap");
    }
}

/// Start the daemon, open a `claude` pane, and wait for its ask to reach the
/// ring as a `Permission`.
async fn a_claude_asking() -> Asking {
    // The hook the daemon writes into claude's settings is the `farcooler`
    // beside `farcoolerd` (`shim_binary`). Without one there, it would be
    // whatever `farcooler` is on PATH, which is not this build.
    let cli = daemon_binary().with_file_name("farcooler");
    assert!(cli.is_file(), "no farcooler at {} — run `cargo build -p farcooler-cli` first", cli.display());
    assert!(farcooler_core::programs::find("tmux").is_some(), "tmux is required for a pane");

    let daemon = spawn_with_stand_in(ASKING_CLAUDE).await;
    std::fs::write(
        daemon.dir.path().join("dialog.txt"),
        include_str!("../../core/captures/claude-permission-hook-waiting.txt"),
    )
    .unwrap();
    let stand_in = StandIn::under(daemon.dir.path());
    assert_eq!(
        bare_claude_resolves_to(&daemon, &stand_in),
        stand_in.trap_dir.join("claude").display().to_string(),
        "the trap is not first for a bare claude, so its silence would prove nothing"
    );
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    let repo_dir = tempfile::tempdir().unwrap();
    let repo = repo_dir.path().join("demo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap();
    }
    register_root_and_repository(&daemon.socket, repo_dir.path(), &repo).await;
    let repositories = session.repositories().await.expect("repositories");
    let repository = farcooler_client::session::uuid_of(&repositories[0].id);
    let worktree = session
        .create_worktree(repository, "asks", "feat/asks", "HEAD", "", false)
        .await
        .expect("create_worktree");
    let worktree_id = farcooler_client::session::uuid_of(&worktree.id);
    let terminal = session.create_terminal(worktree_id, "claude", "claude", false).await.expect("a claude pane");
    let terminal = farcooler_client::session::uuid_of(&terminal.id);

    let mut asking = Asking { daemon, session, terminal, id: String::new(), _repo: repo_dir };
    for _ in 0..400 {
        let ring = asking.ring().await;
        if let Some(id) = ring.iter().find_map(|e| e["Permission"]["id"].as_str()) {
            asking.id = id.to_string();
            let options: Vec<&str> = ring
                .iter()
                .find_map(|e| e["Permission"]["options"].as_array())
                .unwrap()
                .iter()
                .filter_map(|o| o["id"].as_str())
                .collect();
            assert_eq!(options, ["allow", "deny"]);
            return asking;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "no Permission reached the ring. hook command: {:?}, hook stderr: {:?}",
        std::fs::read_to_string(asking.out("hook-command")),
        std::fs::read_to_string(asking.out("hook-stderr")),
    );
}

#[tokio::test]
async fn a_permission_answered_from_a_phone_reaches_the_held_hook() {
    let mut asking = a_claude_asking().await;
    let (terminal, id) = (asking.terminal, asking.id.clone());
    asking.session.agent_answer(terminal, &id, "allow").await.expect("the answer landed");

    assert!(asking.exists("hook-exited", std::time::Duration::from_secs(5)).await, "the hook never exited");
    let printed: serde_json::Value =
        serde_json::from_str(asking.hook_printed().trim()).expect("the hook printed json");
    assert_eq!(
        printed,
        serde_json::json!({
            "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } }
        })
    );
    assert_eq!(asking.resolved().await, "allow");
    asking.never_trapped();
}

/// A held ask reaches `needs_you` spelled exactly as the shared fixture
/// (`test/fixtures/needs-you.json`) spells the studio's ask, so the phones'
/// decoders are pinned to what a daemon writes, not to what a test imagined.
#[tokio::test]
async fn a_held_ask_reaches_needs_you_as_the_shared_fixture_spells_it() {
    let mut asking = a_claude_asking().await;
    let (terminal, id) = (asking.terminal, asking.id.clone());
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../test/fixtures/needs-you.json")).expect("the fixture");
    let spelled = &fixture["runners"][0]["needs_you"]["items"][0];
    assert_eq!(spelled["kind"], "ask", "the fixture's first item is its ask");

    let mut item = serde_json::Value::Null;
    for _ in 0..100 {
        let listed = asking.session.needs_you().await.expect("needs_you");
        if let Some(found) = listed["items"].as_array().unwrap().iter().find(|i| i["ask_id"] == id.as_str()) {
            item = found.clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!item.is_null(), "the held ask never reached needs_you");
    assert_eq!(item["id"], format!("ask:{id}"));
    assert!(id.starts_with("hook-ask-"), "{id}");
    assert!(spelled["ask_id"].as_str().unwrap().starts_with("hook-ask-"), "the fixture's ask id isn't a hook ask's");
    assert_eq!(item["terminal"]["id"], terminal.to_string());
    assert_eq!(item["terminal"]["label"], spelled["terminal"]["label"]);
    assert_eq!(item["terminal"]["role"], spelled["terminal"]["role"]);
    assert_eq!(item["terminal"]["pane_mode"], spelled["terminal"]["pane_mode"]);
    assert!(item["detail"].is_null() && spelled["detail"].is_null(), "an ask has no detail");
    assert!(item["worktree"].is_object(), "Control sees the worktree: {item}");
    // The buttons, in order, with the allow option's own name as both the
    // question and its title: only the tool's words differ.
    let buttons = |i: &serde_json::Value| -> Vec<(String, bool, bool)> {
        i["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| (a["id"].as_str().unwrap().to_string(), a["destructive"] == true, a["primary"] == true))
            .collect()
    };
    assert_eq!(buttons(&item), buttons(spelled));
    assert_eq!(item["question"], item["actions"][0]["title"]);
    assert_eq!(spelled["question"], spelled["actions"][0]["title"]);
    assert_eq!(item["actions"][1]["title"], spelled["actions"][1]["title"]);
    asking.session.agent_answer(terminal, &id, "deny").await.expect("the answer landed");
    asking.never_trapped();
}

/// The local socket has no client id, so the device is the runner itself:
/// "Mac" on macOS, "this computer" elsewhere.
#[tokio::test]
async fn a_permission_denied_from_the_mac_says_so_to_the_model() {
    let mut asking = a_claude_asking().await;
    let (terminal, id) = (asking.terminal, asking.id.clone());
    asking.session.agent_answer(terminal, &id, "deny").await.expect("the answer landed");

    assert!(asking.exists("hook-exited", std::time::Duration::from_secs(5)).await, "the hook never exited");
    let printed: serde_json::Value =
        serde_json::from_str(asking.hook_printed().trim()).expect("the hook printed json");
    assert_eq!(printed["hookSpecificOutput"]["decision"]["behavior"], "deny");
    let local = if cfg!(target_os = "macos") { "Mac" } else { "this computer" };
    assert_eq!(printed["hookSpecificOutput"]["decision"]["message"], format!("Denied from {local}"));
    assert_eq!(asking.resolved().await, "deny");
    asking.never_trapped();
}

/// The keyboard answers first. The hook is released with nothing to say
/// (claude already has its answer), every surface is told the ask is over,
/// and a phone that answers late is told something else changed it.
#[tokio::test]
async fn a_permission_answered_at_the_keyboard_releases_the_held_hook_and_the_phone() {
    use farcooler_client::session::SessionError;

    let mut asking = a_claude_asking().await;
    let terminal = asking.terminal;
    // The dialog has to have been SEEN before it can be seen to leave (a
    // keyboard answer no sample saw is left to the turn's end or the hold's),
    // so wait for the watcher to call the pane blocked.
    let mut blocked = false;
    for _ in 0..100 {
        let fleet = asking.session.fleet().await.expect("fleet");
        blocked = fleet["worktrees"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|w| w["terminals"].as_array().unwrap())
            .any(|t| t["id"] == terminal.to_string().as_str() && t["activity"] == "blocked");
        if blocked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(blocked, "the watcher never saw the dialog");

    asking.session.write(terminal, b"1\n".to_vec()).await.expect("typed");
    assert!(asking.exists("tui-answered", std::time::Duration::from_secs(5)).await, "the keyboard answer never landed");
    assert!(
        asking.exists("hook-exited", std::time::Duration::from_secs(5)).await,
        "the hook was still held 5 s after the dialog left"
    );
    assert_eq!(asking.hook_printed(), "", "the keyboard decided; the hook has nothing to say");
    assert_eq!(asking.resolved().await, "");

    let id = asking.id.clone();
    match asking.session.agent_answer(terminal, &id, "allow").await {
        Err(SessionError::Refused { code, what, .. }) => {
            assert_eq!(farcooler_core::error::word_for(code), "resource-conflict");
            // Named, so a phone can say "Someone already answered this."
            assert_eq!(what, "not_held");
        }
        other => panic!("a late answer was not refused as a conflict: {other:?}"),
    }
    asking.never_trapped();
}

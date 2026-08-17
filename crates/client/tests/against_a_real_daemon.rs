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
        let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
        let _ = std::process::Command::new(tmux)
            .args(["-L", &socket, "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
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

async fn spawn(scratch_home: bool) -> Daemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("farcoolerd.sock");

    let mut command = std::process::Command::new(daemon_binary());
    command
        .env("FARCOOLER_HOME", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if scratch_home {
        command.env("HOME", dir.path());
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

#[tokio::test]
async fn a_client_connects_and_learns_the_daemon_version() {
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");
    assert!(!session.daemon_version().is_empty());
}

#[tokio::test]
async fn a_client_learns_what_the_runner_can_do_before_asking_it_anything() {
    // The mechanism a newer app uses to degrade against an older runner. It
    // has to be answered by the handshake rather than by a call, because the
    // app decides what to draw before it has made one.
    let daemon = start().await;
    let session = Session::connect_local(&daemon.socket).await.expect("connect");

    assert!(session.can(farcooler_protocol::capability::WORKSPACES));
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
    assert!(fleet.get("workspaces").is_some_and(|v| v.is_array()));
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
async fn a_workspace_created_through_the_client_comes_back_in_the_fleet() {
    let daemon = start().await;
    let mut session = Session::connect_local(&daemon.socket).await.expect("connect");

    // A real repository, because workspace creation makes a real worktree.
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
    let workspace = session
        .create_workspace(repository, "phone task", "feat/phone", "HEAD", "", false)
        .await
        .expect("create_workspace");
    assert_eq!(workspace.task_name, "phone task");

    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    // Two: the one just created, plus the main checkout that registering the
    // repository adopts automatically.
    assert_eq!(workspaces.len(), 2);
    let created =
        workspaces.iter().find(|w| w["task"] == "phone task").expect("created workspace present");
    assert_eq!(created["branch"], "feat/phone");
    // Derived, never stored — and a fresh workspace with no terminals is ready.
    assert_eq!(created["state"], "ready");
    assert!(created["terminals"].as_array().unwrap().is_empty());
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
    let workspace = session
        .create_workspace(repository, "reversible", "feat/rev", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

    session.hide_workspace(id).await.expect("hide");
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let reversible =
        workspaces.iter().find(|w| w["task"] == "reversible").expect("its own workspace present");
    assert_eq!(reversible["state"], "hidden");

    session.unhide_workspace(id).await.expect("unhide");
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let reversible =
        workspaces.iter().find(|w| w["task"] == "reversible").expect("its own workspace present");
    assert_eq!(reversible["state"], "ready");
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
    let workspace = session
        .create_workspace(repository, "agent test", "feat/agent-empty", "HEAD", "", false)
        .await
        .expect("create_workspace");
    let workspace_id = farcooler_client::session::uuid_of(&workspace.id);

    let terminal = session
        .create_terminal(workspace_id, "shell", "shell", false)
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
    assert!(session.hide_workspace(missing).await.is_err());

    // Still usable.
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
    let workspace = session
        .create_workspace(repository, "clean removal", "feat/clean-removal", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

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
    let workspace = session
        .create_workspace(repository, "dirty removal", "feat/dirty-removal", "HEAD", "", false)
        .await
        .expect("create");
    let id = farcooler_client::session::uuid_of(&workspace.id);

    // Find the worktree on disk and dirty it. The daemon derives "dirty" from
    // git status, so this has to be a real uncommitted change, not a flag.
    let fleet = session.fleet().await.expect("fleet");
    let workspaces = fleet["workspaces"].as_array().unwrap();
    let created =
        workspaces.iter().find(|w| w["task"] == "dirty removal").expect("workspace present");
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
        .enroll_client(A_DEVICE_KEY, "Ada's iPhone", "phone-7", "read", false)
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
        .enroll_client(A_DEVICE_KEY, "MacBook Air", "mac-9", "control", false)
        .await
        .expect("Key A");
    assert_eq!(a["client"]["shellAccess"], false);
    assert_eq!(a["client"]["scope"], "control");

    // Key B: plain, host_admin, same client id.
    let b = session
        .enroll_client(A_MAC_S_SHELL_KEY, "MacBook Air", "mac-9", "host_admin", true)
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
                .enroll_client(A_MAC_S_SHELL_KEY, "MacBook Air", "mac-9", scope, true)
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
        .enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "read", false)
        .await
        .expect("first enrollment");
    let again = session
        .enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "host_admin", false)
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

    session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "read", false).await.expect("enroll");

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

    assert!(session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "admin", false).await.is_err());
    assert!(session.enroll_client(A_DEVICE_KEY, "iPhone", "phone-7", "", false).await.is_err());

    // And the session is still usable, because a refusal is not a dropped link.
    assert!(session.enrolled_clients().await.is_ok());
    let written = std::fs::read_to_string(&authorized_keys).unwrap();
    assert!(!written.contains("--client phone-7"), "a refused scope enrolled something");
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

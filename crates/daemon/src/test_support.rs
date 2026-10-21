//! Shared test fixtures for the daemon crate.
//!
//! Extracted from `reconcile.rs`'s test module so `service.rs`'s tests can
//! reach the same real-git-repo fixture without a second, drifting copy.

use std::sync::Arc;

use uuid::Uuid;

use crate::git;
use crate::service::Service;

/// The fixture's temporary directory, plus the tmux server that came with it.
///
/// A `Service` starts a private tmux server the first time a test asks it for a
/// terminal, and nothing ever stopped one. The `TempDir` went away when the test
/// ended and the server did not: it holds a session, a pane and an interactive
/// shell, on a socket whose name died with the directory that produced it, so
/// nothing could ever find it again to clean it up.
///
/// They accumulate. On the machine this was written for there were 362 live tmux
/// servers and 2 454 sockets under `/tmp`, and they are not inert — tmux is
/// single-threaded per server and they compete for the same CPU. `capture-pane`
/// against the real fleet measured 50ms at rest and 740ms with that crowd
/// running, which is slow enough to have broken timing assumptions in the tmux
/// crate's own suite.
///
/// Deref rather than a new type at every call site: two dozen tests already say
/// `dir.path()`, and none of them should have to know this exists.
pub(crate) struct ScratchDir {
    dir: tempfile::TempDir,
    socket: String,
}

impl std::ops::Deref for ScratchDir {
    type Target = tempfile::TempDir;
    fn deref(&self) -> &tempfile::TempDir {
        &self.dir
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Blocking, and not the async `kill_server`. `Drop` cannot await, and a
        // task spawned here would want a runtime that is being torn down to poll
        // it. Killing a server that never started fails harmlessly and costs a
        // couple of milliseconds; leaving one running costs the machine for as
        // long as it stays on.
        farcooler_tmux::reap_server(&self.socket);
    }
}

/// A service with one registered repository, on a real git repo on disk.
///
/// Real repositories rather than hand-written fixtures: `reconcile.rs`'s
/// tests are about the shape of `git worktree list --porcelain`, and a
/// hand-written fixture would only prove the author can copy it. Sharing it
/// with `service.rs`'s tests means both get a worktree row that came from
/// the same adoption path a live daemon uses.
pub(crate) async fn fixture() -> (ScratchDir, Arc<Service>, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
            vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git::git(&repo, &args).await.unwrap();
    }

    let svc = Arc::new(Service::open_in(state).await.unwrap());
    svc.add_root(dir.path()).await.unwrap();
    let registered = svc.register_repository(&repo).await.unwrap();
    // Read from the service rather than recomputed, so the reaper cannot end up
    // aiming at a socket name the service does not actually use.
    let socket = svc.tmux.socket().to_string();
    (ScratchDir { dir, socket }, svc, registered.id)
}

/// Two services at two roots, both registering the SAME repository.
///
/// This is what two channels on one host are. A channel's only job is to
/// choose a runtime directory, so two roots is two channels — and it is the
/// same shape `rpc_over_socket.rs` already relies on, where every test gets "a
/// daemon on a private socket with a private database" at an explicit
/// directory rather than through the process-global `FARCOOLER_HOME`.
///
/// One repository on purpose. Two installs that never touched the same repo
/// would be isolated by having nothing in common, which proves nothing; git
/// reports every worktree of a repository regardless of which daemon made it,
/// and that is the case worth testing.
pub(crate) async fn two_daemons()
-> (tempfile::TempDir, Arc<Service>, Arc<Service>, Uuid, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git::git(&repo, &args).await.unwrap();
    }

    let mut svcs = Vec::new();
    let mut repos = Vec::new();
    for tag in ["a", "b"] {
        let state = dir.path().join(tag);
        std::fs::create_dir_all(&state).unwrap();
        let svc = Arc::new(Service::open_in(state).await.unwrap());
        svc.add_root(dir.path()).await.unwrap();
        let registered = svc.register_repository(&repo).await.unwrap();
        svcs.push(svc);
        repos.push(registered.id);
    }

    let b = svcs.pop().unwrap();
    let a = svcs.pop().unwrap();
    (dir, a, b, repos[0], repos[1])
}

/// How long a test waits for a real pane to do something before failing.
///
/// Long, because the wait returns the moment the thing happens: what this
/// bounds is a loaded machine, where a dozen test processes each start tmux
/// servers and shells at once, and a bound of a few seconds failed tests
/// whose pane was merely slow (ov-547).
pub(crate) const PANE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Wait until `terminal`'s shell has drawn something, its prompt, so keys
/// typed after this are read by it rather than lost to a shell still starting.
pub(crate) async fn shell_ready(svc: &Service, terminal: Uuid) {
    let started = std::time::Instant::now();
    while started.elapsed() < PANE_PATIENCE {
        if svc.runtime().screen(terminal).await.is_ok_and(|(s, _, _)| !s.trim().is_empty()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the pane's shell drew nothing in {PANE_PATIENCE:?}");
}

/// A process outside the runner, orphaned to pid 1, as the seat walk meets
/// one (`caller_seat`, ov-649): `pid` is a `sleep`, and `parent` the
/// stand-in above it, while one stays. Both ended on drop.
pub(crate) struct Outside {
    pub(crate) pid: i32,
    pub(crate) parent: Option<i32>,
    /// The terminal's master side, for one with a terminal: closed on drop.
    master: Option<i32>,
    /// The stand-in taken for root's, forgotten on drop.
    root: Option<i32>,
}

impl Outside {
    /// A copy of perl at `at`, to stand in for a program by its name and
    /// place (a copied `/bin/sh` is killed on macOS for its signature).
    pub(crate) fn stand_in(at: &std::path::Path) -> std::path::PathBuf {
        std::fs::create_dir_all(at.parent().expect("a directory")).unwrap();
        let perl = farcooler_core::programs::find("perl").expect("perl");
        assert!(std::process::Command::new("cp").arg(perl).arg(at).status().unwrap().success());
        at.to_path_buf()
    }

    /// A `sleep` under `program`, which stays, the two orphaned to pid 1 with
    /// no terminal: the CLI the Mac app spawns, or one under a user's own
    /// `sshd`.
    pub(crate) fn under(program: &std::path::Path, seconds: u32) -> Outside {
        Self::run(program, &Self::forks_a_sleep(seconds, false), None)
    }

    /// `under`, with `program` standing in for a process of root's
    /// (`caller_seat::is_roots`) until dropped: a person's session under
    /// root's `sshd`.
    pub(crate) fn under_root(program: &std::path::Path, seconds: u32) -> Outside {
        let mut outside = Self::under(program, seconds);
        let parent = outside.parent.expect("the stand-in stays");
        crate::caller_seat::ROOT_IN_TESTS.lock().unwrap().insert(parent);
        outside.root = Some(parent);
        outside
    }

    /// `under`, with `program` leading a session on a terminal of its own,
    /// as a person's Terminal shell does: a pty that's none of the runner's
    /// panes, the `sleep`'s controlling terminal.
    pub(crate) fn in_a_terminal(program: &std::path::Path, seconds: u32) -> Outside {
        Self::on_a_pty(program, seconds, false)
    }

    /// `in_a_terminal`, with the `sleep` in a session of its own and no
    /// terminal, as a tool command of a claude run in Terminal is: Claude
    /// Code's Bash tool `setsid`s.
    pub(crate) fn tool_of_one_in_a_terminal(program: &std::path::Path, seconds: u32) -> Outside {
        Self::on_a_pty(program, seconds, true)
    }

    fn on_a_pty(program: &std::path::Path, seconds: u32, detached: bool) -> Outside {
        // SAFETY: plain libc pty setup; each return value is checked.
        let (master, slave) = unsafe {
            let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            assert!(master >= 0 && libc::grantpt(master) == 0 && libc::unlockpt(master) == 0, "a pty");
            // Not inherited by what other tests spawn meanwhile (a tmux server).
            assert_eq!(libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC), 0);
            let slave = libc::open(libc::ptsname(master), libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
            assert!(slave >= 0, "the pty's slave");
            (master, slave)
        };
        let outside = Self::run(program, &Self::forks_a_sleep(seconds, detached), Some((master, slave)));
        // SAFETY: the slave this opened; the session holds its own as stdio.
        unsafe { libc::close(slave) };
        outside
    }

    /// A script that forks a `sleep` (in a session of its own when
    /// `detached`), writes `<its pid> <the sleep's>` to its one argument once
    /// the `sleep` runs, and waits.
    fn forks_a_sleep(seconds: u32, detached: bool) -> String {
        let setsid = if detached { "POSIX::setsid(); " } else { "" };
        let sleep = Self::execs_a_sleep(seconds, setsid);
        format!(
            "use POSIX ();
             {sleep}
             open(my $f, '>', \"$ARGV[0].part\") or die; print $f \"$$ $p\\n\"; close $f;
             rename(\"$ARGV[0].part\", $ARGV[0]) or die; waitpid($p, 0);"
        )
    }

    /// Perl that forks `$p`, which runs `then` and execs `sleep`, and returns
    /// only once it has (ov-668): until the exec, `$p` is a copy of the
    /// stand-in, which the seat walk rightly reads as the program it stands
    /// in for (a claude, an agent), and a test that asks about `$p` under
    /// load asks about that. Perl's pipes are close-on-exec, so the read
    /// ends when the exec closes the child's end.
    fn execs_a_sleep(seconds: u32, then: &str) -> String {
        format!(
            "pipe(my $execd, my $exec) or die;
             my $p = fork(); if (!$p) {{ close $execd; {then}exec 'sleep', '{seconds}'; POSIX::_exit(127); }}
             close $exec; <$execd>; close $execd;"
        )
    }

    /// A `sleep` that `program` started through a tool shell that's gone, so
    /// nothing is above it but pid 1: a command an agent left behind with
    /// `(cmd &)`. `program` (a stand-in claude) exits too.
    pub(crate) fn left_by(program: &std::path::Path, seconds: u32) -> Outside {
        // The tool shell says the sleep's pid once it runs `sleep`, and
        // exits; the pid is written only once it's gone, so the sleep is
        // orphaned by then.
        let sleep = Self::execs_a_sleep(seconds, "close $w; ");
        let script = format!(
            "pipe(my $r, my $w) or die;
             my $tool = fork();
             if (!$tool) {{
                 close $r; {sleep}
                 print $w \"$p\\n\"; close $w; POSIX::_exit(0);
             }}
             close $w; my $p = <$r>; chomp $p; waitpid($tool, 0);
             open(my $f, '>', \"$ARGV[0].part\") or die; print $f \"0 $p\\n\"; close $f;
             rename(\"$ARGV[0].part\", $ARGV[0]) or die;"
        );
        Self::run(program, &format!("use POSIX ();\n{script}"), None)
    }

    /// A person's process outside the runner, as a device's ssh session is:
    /// a `sleep` under a stand-in for root's `sshd`, made in `dir`. No agent
    /// this test may itself run under is its ancestor.
    pub(crate) fn a_person(dir: &std::path::Path, seconds: u32) -> Outside {
        Self::under_root(&Self::stand_in(&dir.join("ssh").join("sshd")), seconds)
    }

    /// `program` running `script` with the pid file as its one argument,
    /// orphaned to pid 1 and leading a session of its own: with no terminal
    /// (as an agent's tool shell and a launchd job have none), or on the pty
    /// `tty` (master, slave).
    fn run(program: &std::path::Path, script: &str, tty: Option<(i32, i32)>) -> Outside {
        use std::os::unix::process::CommandExt as _;
        let dir = tempfile::tempdir().unwrap();
        let (code, pids) = (dir.path().join("script.pl"), dir.path().join("pids"));
        std::fs::write(&code, format!("$| = 1;\n{script}\n")).unwrap();
        let mut command = std::process::Command::new(program);
        command.arg(&code).arg(&pids).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        let slave = tty.map(|(_, slave)| slave);
        // SAFETY: between fork and exec, only async-signal-safe calls. The
        // first child exits at once, so its child (which goes on to exec
        // `program`) is orphaned to pid 1; `spawn` reads that exit as a
        // success, since nothing failed before an exec.
        unsafe {
            command.pre_exec(move || {
                match libc::fork() {
                    -1 => return Err(std::io::Error::last_os_error()),
                    0 => {}
                    _ => libc::_exit(0),
                }
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The terminal as its stdio too, as a shell's is: a pty no
                // process holds open is no one's controlling terminal.
                if let Some(slave) = slave
                    && (libc::ioctl(slave, libc::TIOCSCTTY as _, 0) < 0 || (0..3).any(|fd| libc::dup2(slave, fd) < 0))
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let _ = command.spawn().expect("the stand-in").wait();
        let since = std::time::Instant::now();
        loop {
            if let Ok(said) = std::fs::read_to_string(&pids) {
                let mut f = said.split_whitespace().filter_map(|w| w.parse::<i32>().ok());
                let (parent, pid) = (f.next().unwrap_or(0), f.next().expect("the sleep's pid"));
                return Outside { pid, parent: (parent > 0).then_some(parent), master: tty.map(|(master, _)| master), root: None };
            }
            assert!(since.elapsed() < PANE_PATIENCE, "{} never started", program.display());
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        for pid in std::iter::once(self.pid).chain(self.parent) {
            // SAFETY: a signal to a process this test started.
            unsafe { libc::kill(pid, libc::SIGTERM) };
        }
        if let Some(root) = self.root {
            crate::caller_seat::ROOT_IN_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&root);
        }
        if let Some(master) = self.master {
            // SAFETY: the descriptor `in_a_terminal` opened.
            unsafe { libc::close(master) };
        }
    }
}


/// Put `handle`'s Claude directory in the state `claude auth login` leaves
/// it: what `rep.start` and a rep's wake ask for (ov-587).
pub(crate) fn sign_in_member(svc: &crate::service::Service, handle: &str) {
    let id = svc.store.ensure_member_login(handle).unwrap();
    let dir = crate::claude_logins::ensure_login_dir(svc.root_dir(), &id).unwrap();
    std::fs::write(std::path::Path::new(&dir).join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"member@example.com"}}"#).unwrap();
}

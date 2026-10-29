//! The runner half of the real-sshd tests: a daemon, a real sshd in front of
//! it, and one device enrolled through the shipped fence API.
//!
//! Shared by `a_real_sshd_forces_the_scope.rs`, whose module notes say why
//! these run on every commit, and `a_real_sshd_admits_a_passkey.rs` (ov-765),
//! so the passkey's line is written and served exactly as an ed25519 one is.

// Each test binary uses a different subset of this.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::Scope;
use tokio::process::Command;

use crate::common::DaemonChild;

/// One runner: a daemon, an sshd in front of it, and one enrolled device.
///
/// **Field order is drop order, and it matters here.** `_dir` deletes the
/// directory that `DaemonChild`'s Drop reads `install-id` out of to find the tmux
/// server it has to reap, so `_dir` is declared last and dropped last. Declared
/// first, every run of this file would leak a tmux server — the exact leak the
/// comment on `DaemonChild` was written about.
pub struct Runner {
    /// The daemon holding this runtime directory's socket, so the session sshd
    /// starts is a RELAY into it. That is what a real runner looks like, and it
    /// is also the only arrangement in which a local `client.revoke` and a
    /// remote session share a session registry at all.
    _daemon: DaemonChild,
    _sshd: Sshd,
    /// FARCOOLER_HOME, handed to the daemon on both sides.
    pub runtime: PathBuf,
    /// The private half of the key the fence enrolled.
    pub identity: PathBuf,
    pub port: u16,
    /// sshd's own log, read back when a wait gives up.
    pub log: PathBuf,
    _dir: tempfile::TempDir,
}

/// A real sshd, and the rule for ending it.
///
/// The same discipline `common::DaemonChild` carries and for the same reason: a
/// listener left behind on a random port, once per test run, is not an idle
/// passenger — it is an authenticating sshd on this developer's machine that
/// nothing can now find by name, holding a fenced `authorized_keys` inside a
/// directory that has been deleted.
struct Sshd {
    /// Read from `PidFile` at startup, while the directory still exists. Drop
    /// must not depend on reading a temporary directory that may already be
    /// gone — which is precisely how the tmux leak above happens.
    pid: i32,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // By pid, never by pattern. This machine runs the developer's own sshd,
        // and a pattern kill here would take their remote access with it.
        //
        // Only the listener is signalled. Each connection is served by a
        // separate `sshd-session` child, and each of those ends when its `ssh`
        // does — which `kill_on_drop` on the client has already seen to.
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
    }
}

/// Enroll one device through the real fence API and put a real sshd in front.
pub async fn start(client_id: &str, scope: Scope) -> Runner {
    start_with(None, client_id, scope).await
}

/// The same, enrolling `received` when given (a browser's passkey, ov-765)
/// rather than an ed25519 key `ssh-keygen` makes, whose private half is then
/// `Runner::identity`.
pub async fn start_with(received: Option<&str>, client_id: &str, scope: Scope) -> Runner {
    // A missing sshd fails loudly. Skipping quietly would make this file report
    // "ok" on a machine where none of it ran, and the release gate reads that
    // report.
    let sshd_binary = Path::new("/usr/sbin/sshd");
    assert!(
        sshd_binary.exists(),
        "no {} on this machine, so nothing here was tested",
        sshd_binary.display()
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    // The account home, whose `.ssh` the fence writer creates at 0700 itself —
    // what it needs is the directory ABOVE, which it anchors its openat to.
    let home = root.join("home");
    std::fs::create_dir(&home).expect("the account home");
    let runtime = root.join("rt");
    std::fs::create_dir(&runtime).expect("the runtime directory");
    let bin = root.join("bin");
    std::fs::create_dir(&bin).expect("the bin directory");

    keygen(&root.join("host_ed25519"), "host");
    let received = match received {
        Some(received) => received.to_string(),
        None => {
            keygen(&root.join("id"), "device");
            std::fs::read_to_string(root.join("id.pub")).expect("the device's public key")
        }
    };

    // The line, from the shipped renderer. Not a string in this file: what is on
    // trial is whether OpenSSH accepts what `render` writes.
    let line =
        fence::render(received.trim(), "Test Device", client_id, scope, Grant::FarCooler, None)
            .expect("render a Key A line");
    let authorized_keys = home.join(".ssh").join("authorized_keys");
    fence::write(
        &authorized_keys,
        fence::AUTHORIZED_KEYS,
        std::slice::from_ref(&line),
        &[],
        Placement::Last,
    )
    .expect("write the fence");
    // 0600 because the writer chose it, asserted rather than imposed: sshd under
    // `StrictModes` refuses a file anybody else can write, so this is a property
    // of the shipped code that a real sshd is entitled to check.
    assert_eq!(mode_of(&authorized_keys), 0o600, "the fence writer left the file too open");

    // The daemon the forced command runs needs a private runtime directory, and
    // there is no room for one in a forced command that must stay byte-for-byte
    // the shipped line. sshd's `environment=` is how it gets one without a
    // wrapper script: it coexists with `restrict`, and it reaches the forced
    // command's environment.
    //
    // **Everything after the scaffolding prefix is `fence::render`'s output
    // verbatim** — the `restrict`, the quoting, the `--client`, the `--scope`,
    // the key and the comment. Only the two `environment=` options are this
    // test's, and they carry no privilege: `PermitUserEnvironment` is off by
    // default and no runner turns it on.
    //
    // PATH is scaffolding of a different kind, and it is scaffolding this test
    // should not have needed. See the module header: the shipped forced command
    // names a bare `farcoolerd`, so it can only be found on PATH, so a test of
    // the shipped line has to arrange a PATH on which it is findable.
    // HOME too, and not only so a `~/`-anchored forced command expands to
    // something this test controls. The daemon reads `$HOME/.ssh/authorized_keys`
    // with no override, so a forced command that inherited the real HOME would
    // have this test's ssh session reading — and on revoke, rewriting — the
    // developer's own fence.
    let scaffolding = format!(
        "environment=\"HOME={}\",environment=\"FARCOOLER_HOME={}\",\
             environment=\"FARCOOLER_TEST_STUB_AGENTS=1\",environment=\"PATH={}\"",
        home.display(),
        runtime.display(),
        path_for_forced_command(&home, &bin, &line)
    );
    let written = std::fs::read_to_string(&authorized_keys).expect("read back the fence");
    let scaffolding = scaffolding + &tmux_tmpdir_option();
    let scaffolded = written.replacen(&line, &format!("{scaffolding},{line}"), 1);
    assert_ne!(scaffolded, written, "the rendered line was not in the file that was just written");
    std::fs::write(&authorized_keys, &scaffolded).expect("prefix the test scaffolding");
    assert_eq!(mode_of(&authorized_keys), 0o600, "the scaffolding rewrite widened the mode");

    let port = free_port();
    let config = root.join("sshd_config");
    let log = root.join("sshd.log");
    let pid_file = root.join("sshd.pid");
    // Absolute paths throughout: sshd resolves a relative one against the
    // invoking directory, which for a test binary is nobody's business.
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
    // sshd's own complaint rather than as a connection that never happens.
    let checked = std::process::Command::new(sshd_binary)
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

    let started = std::process::Command::new(sshd_binary)
        .arg("-f")
        .arg(&config)
        .arg("-E")
        .arg(&log)
        .status()
        .expect("run sshd");
    assert!(started.success(), "sshd did not start; see {}", log.display());

    // A daemon on the socket before the first connection, so that the session
    // sshd starts takes the relay branch — the branch a real runner takes,
    // because a real runner has a daemon running.
    let daemon = listening_daemon_in(&runtime, &home).await;
    let sshd = Sshd { pid: await_sshd(&pid_file, port, &log).await };

    Runner { _daemon: daemon, _sshd: sshd, runtime, identity: root.join("id"), port, log, _dir: dir }
}

/// The daemon that owns this runtime directory, with a home it may write into.
///
/// `common::listening_daemon` cannot be used as it stands: the daemon's
/// `authorized_keys` is `$HOME/.ssh/authorized_keys` and nothing overrides it, so
/// a `client.revoke` from a daemon that inherited the real HOME would edit the
/// developer's own file — and the point of that call here is that it edits the
/// same file sshd just authenticated against.
///
/// `DaemonChild` itself is constructed rather than reimplemented. Its Drop is the
/// part worth not copying: the daemon's private tmux server is named after this
/// runtime directory's install id, so once the directory is gone nothing on the
/// machine can work out what the socket was called and the server survives until
/// a restart.
async fn listening_daemon_in(runtime: &Path, home: &Path) -> DaemonChild {
    let child = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .env("FARCOOLER_HOME", runtime)
        .env("FARCOOLER_CONFIG", runtime.join("config.toml"))
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .env("HOME", home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd");
    let daemon = DaemonChild { child, home: runtime.to_path_buf() };

    // Polled rather than slept on, for the reason `common::listening_daemon`
    // gives: binding happens after the service opens and the tmux inventory is
    // taken, which is fast on an idle machine and not on a busy one.
    let socket = runtime.join("farcoolerd.sock");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if tokio::net::UnixStream::connect(&socket).await.is_ok() {
            return daemon;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no daemon ever accepted on {}", socket.display());
}

/// An sshd that has written its pid and is answering on its port.
///
/// Both, because they are different facts: `sshd -f` forks and returns straight
/// away, so its exit status says only that the parent got as far as forking. A
/// fixed sleep here would be flaky in the direction that costs an hour — the
/// failure would be a refused connection in a test about authentication.
async fn await_sshd(pid_file: &Path, port: u16, log: &Path) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(pid) =
            std::fs::read_to_string(pid_file).ok().and_then(|t| t.trim().parse::<i32>().ok())
        {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no sshd ever listened on port {port}; see {}", log.display());
}

/// Put the built daemon where the rendered forced command will look for it.
///
/// The name is read out of the rendered line rather than written down here, so
/// this follows the product wherever the spelling goes instead of quietly testing
/// one it no longer writes. Two shapes are handled because the line has had both:
/// a bare name, found only on PATH, and today's `~/`-anchored path.
///
/// The real directories are on the PATH too, after ours. Not padding: sshd runs
/// a forced command through the account's LOGIN SHELL, whose startup files run
/// first, and a shell that cannot find `tty` or `mktemp` writes pages of its own
/// errors down the session's stderr. Which is itself worth knowing about a
/// mechanism that depends on a bare name: what resolves it is not sshd, it is
/// whatever the user's shell config leaves in PATH.
///
/// Both spellings are handled, so this test does not have to change on the same
/// commit that fixes the defect. A `~/`-anchored name is deliberately resolved
/// against the OVERRIDDEN `HOME` rather than the developer's real one: this
/// machine has a live Far Cooler install, so a link placed in the real
/// `~/.local/bin` would clobber it, and a lookup that found it would silently
/// test the INSTALLED daemon instead of the one just built.
fn path_for_forced_command(home: &Path, bin: &Path, line: &str) -> String {
    let program = forced_program(line);
    let target = match program.strip_prefix("~/") {
        Some(rest) => {
            let path = home.join(rest);
            let parent = path.parent().expect("a path-bearing program name has a parent");
            std::fs::create_dir_all(parent).expect("the directory the forced command names");
            path
        }
        // An absolute path names a real location on this machine, and this test
        // will not write into one. Better to stop with the reason than to litter
        // somebody's filesystem or, worse, exercise a daemon nobody built here.
        None if program.starts_with('/') => {
            panic!("the forced command names the absolute path {program}, which this test will not create")
        }
        None => bin.join(program),
    };
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_farcoolerd"), &target)
        .expect("link the daemon under the name the forced command asks for");
    format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
}

/// The program a rendered line asks sshd to run.
fn forced_program(line: &str) -> &str {
    let (_, after) = line.split_once("command=\"").expect("a Key A line carries a forced command");
    let (command, _) = after.split_once('"').expect("a forced command is quoted");
    command.split_whitespace().next().expect("a forced command names a program")
}

/// A port nothing was listening on a moment ago.
///
/// Bound and released rather than chosen from a range, so two of these tests
/// running at once — which `cargo test` does by default — cannot pick the same
/// number. The race that remains is the small one: something else on this
/// machine may take the port between the drop and sshd's bind, and then sshd
/// fails to start and `await_sshd` says so. That is the right way round; a fixed
/// port would collide reliably instead of rarely.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a scratch port");
    let port = listener.local_addr().expect("the scratch port's address").port();
    drop(listener);
    port
}

/// An ed25519 keypair, from OpenSSH's own tool.
///
/// `ssh-keygen` rather than a Rust keypair, because the key is going into a file
/// OpenSSH parses: a key this test generated by another route and sshd then
/// refused would be indistinguishable from the fence writing a line sshd
/// refuses, which is one of the things being tested.
fn keygen(path: &Path, comment: &str) {
    let status = std::process::Command::new("/usr/bin/ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen did not write {}", path.display());
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

/// How long any one wait on sshd, the forced command or the daemon may take.
pub const WAIT: Duration = Duration::from_secs(30);

/// `step` bounded by `WAIT`: a stuck sshd or daemon fails the test with the
/// step's name and the tail of sshd's log, not at the runner's 600 s ceiling
/// (ov-857).
pub async fn within<T>(runner: &Runner, what: &str, step: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(WAIT, step).await {
        Ok(done) => done,
        Err(_) => {
            let log = std::fs::read_to_string(&runner.log).unwrap_or_default();
            let tail: Vec<&str> = log.lines().rev().take(15).collect();
            let tail: Vec<&str> = tail.into_iter().rev().collect();
            panic!("{what} did not finish within {}s; sshd log tail:\n{}", WAIT.as_secs(), tail.join("\n"))
        }
    }
}

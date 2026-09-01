//! A real sshd, one pane, and the same client against both kinds of key.
//!
//! The defect this file exists for could not be seen from either side alone.
//! `crates/client` opened a second ssh channel and exec'd
//! `farcoolerd --stream <id>` on it; `crates/fence` enrolled every FarCooler key
//! with `restrict,command="~/.local/bin/farcoolerd --stdio …"`; and a forced
//! command is OpenSSH discarding what the client asked to run. So on every
//! enrolled device the second channel ran a `--stdio` protocol relay, which says
//! nothing until it is spoken to. The channel opened, the client read success,
//! and no byte ever arrived — for the whole life of the pane.
//!
//! Nothing caught it. The Mac was fine, because a Mac enrolls a plain shell key
//! beside its restricted one and the exec runs on that. Every test was fine, for
//! the same reason: a test key forces nothing. The bug lived entirely in the
//! difference between two lines of `authorized_keys`, which is why **that
//! difference is the only variable here.** One sshd, one daemon, one pane, one
//! client, two keys.
//!
//! So the assertion is a measurement: both keys must reach the first replayed
//! byte, and reach it in the time a capture takes rather than never. A test that
//! only ran on a plain key would pass today, would have passed on every day this
//! product was broken, and would pass again the next time somebody makes the
//! client depend on which line it holds.
//!
//! **Scheduled lane, not per-commit,** for the reason
//! `a_real_sshd_forces_the_scope.rs` gives: this drives a real sshd, and a test
//! that quietly detected a missing one and printed "ok" would be a lie told to a
//! release gate. Each test is `#[ignore]`d and Rust reports an ignored test as
//! ignored.
//!
//! **The harness below is deliberately not shared with
//! `a_real_sshd_forces_the_scope.rs`.** That file is the security proof that the
//! forced command cannot be talked past, and it is the one file this change must
//! not touch: reaching into it to extract a helper is how a refactor quietly
//! becomes an edit to the thing under test. The recipe is the same and the
//! comments there explain the parts that are subtle; what is here is the second
//! runner it did not need — a plain key on the same sshd, so both lines are
//! authenticated by one process against one daemon.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_client::session::Session;
use farcooler_client::ssh::{Destination, HostKeyPolicy};
use farcooler_fence::{self as fence, Grant, Placement};
use farcooler_protocol::v1::{Scope, request, result};
use farcooler_transport::{Client, request as rpc};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

mod common;
use common::DaemonChild;

/// How long a first byte may take before this is a failure rather than a
/// measurement.
///
/// Enormously more than the answer — the numbers this prints are tens of
/// milliseconds — and chosen that way on purpose. The bug was not "slow", it was
/// "never": twenty-five seconds and nothing. A tight bound here would make the
/// test fail on a loaded machine for reasons that have nothing to do with the
/// property, and the property is that a byte arrives at all on a key that forces
/// a command. The per-key numbers are printed so a person reading a run can see
/// the real cost.
const FIRST_BYTE_CEILING: Duration = Duration::from_secs(5);

/// Both kinds of key reach the first byte, and neither is the odd one out.
///
/// The whole regression, in one assertion pair. `Grant::FarCooler` is the line
/// every phone and every Android device is enrolled with — `restrict`, a forced
/// command, a client id and a scope — and `Grant::Shell` is the plain line a Mac
/// also holds and every test used to hold exclusively. Same client, same daemon,
/// same pane.
#[tokio::test]
#[ignore = "drives a real loopback sshd, so the scheduled lane only: \
    cargo test -p farcooler-daemon --test a_forced_command_still_streams -- --ignored"]
async fn both_kinds_of_key_reach_the_first_byte() {
    let runner = start().await;
    let terminal = a_pane(&runner).await;

    let forced = first_byte(&runner, &runner.forced_key, terminal).await;
    let plain = first_byte(&runner, &runner.plain_key, terminal).await;

    eprintln!(
        "first byte: forced command {}ms, plain key {}ms",
        forced.as_millis(),
        plain.as_millis()
    );

    assert!(
        forced < FIRST_BYTE_CEILING,
        "a key with a forced command took {}ms to deliver a byte, which is the defect: \
         the second channel is running something that does not stream",
        forced.as_millis()
    );
    assert!(
        plain < FIRST_BYTE_CEILING,
        "a plain key took {}ms to deliver a byte",
        plain.as_millis()
    );
}

/// The replay carries the pane's scrollback, on the key that could not stream.
///
/// The user-visible half, and the reason the defect was reported as a scroll
/// bug: a pane on the polling fallback is fed a `capture-pane` of the visible
/// screen alone, so its emulator has no history and swiping moves nothing. A
/// stream replays the scrollback first. This writes a line, scrolls it off the
/// screen, and asserts it comes back — through the forced-command key, which is
/// the one that used to get nothing at all.
#[tokio::test]
#[ignore = "drives a real loopback sshd, so the scheduled lane only: \
    cargo test -p farcooler-daemon --test a_forced_command_still_streams -- --ignored"]
async fn a_forced_command_key_replays_the_scrollback() {
    let runner = start().await;
    let terminal = a_pane(&runner).await;

    // A marker, then enough blank lines to push it off a screen of any ordinary
    // height. Through the daemon's own socket, so this is the pane's real
    // history and not something written into the wire.
    let mut admin = local(&runner).await;
    write_to(&mut admin, terminal, "echo far-cooler-scrollback-marker\n").await;
    write_to(&mut admin, terminal, "for i in $(seq 1 200); do echo .; done\n").await;
    // The shell has to actually run it before the capture can contain it. Polled
    // rather than slept through: what this waits for is a program's output, and
    // how long that takes belongs to the machine.
    let mut replay = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        replay = replay_over(&runner, &runner.forced_key, terminal).await;
        if contains(&replay, b"far-cooler-scrollback-marker") {
            // And it arrives as one picture. The replay is a clear followed by
            // a redraw and the transport splits it — a 440 KiB replay written
            // to a pipe in one `write_all` comes back out in seven reads — so
            // a client that is already showing this pane would draw the cleared
            // screen between them. `runtime::synchronized` wraps it in DECSET
            // 2026 to stop that, and this is the only test that sees the bytes
            // after a real sshd has carried them rather than as the function
            // returned them.
            assert!(
                replay.starts_with(b"\x1b[?2026h"),
                "the replay did not open a synchronized update: {:?}",
                &replay[..replay.len().min(16)]
            );
            assert!(
                contains(&replay, b"\x1b[?2026l"),
                "the replay opened a synchronized update it never closed, which is a pane \
                 held on its last frame until the client's own deadline fires"
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "the replay through a forced-command key carried no scrollback in {} bytes",
        replay.len()
    );
}

/// `--stream` still works, and this is the only test that can say so.
///
/// The wire method is a SECOND way in, not a replacement. `--stream` is what the
/// Mac's CLI runs, and it is the only path a client has against a runner too old
/// to advertise `terminal_stream` — so the client only stops exec'ing it when the
/// runner says it need not. Exec'd directly here, because a client talking to a
/// runner that has the capability will never choose it.
#[tokio::test]
#[ignore = "drives a real loopback sshd, so the scheduled lane only: \
    cargo test -p farcooler-daemon --test a_forced_command_still_streams -- --ignored"]
async fn the_stream_command_still_streams_on_a_key_that_forces_nothing() {
    let runner = start().await;
    let terminal = a_pane(&runner).await;

    let mut child = ssh_command(&runner, &runner.plain_key)
        .arg("--")
        .arg(format!("~/.local/bin/farcoolerd --stream {terminal}"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn ssh");
    // Held, never written to: closing it is how the far side learns nobody is
    // watching, and it would end the stream this is trying to read.
    let _stdin = child.stdin.take().expect("ssh stdin");
    let mut stdout = child.stdout.take().expect("ssh stdout");

    let mut buf = [0u8; 8 * 1024];
    let read = tokio::time::timeout(FIRST_BYTE_CEILING, stdout.read(&mut buf))
        .await
        .expect("`--stream` on a plain key delivered nothing")
        .expect("read the stream");
    assert!(read > 0, "`--stream` opened and ended without a byte");
}

// MARK: - the client, against one key

/// How long `Session::open_stream` takes to produce a byte, through this key.
///
/// The product's own client, not a hand-rolled one. What broke was the decision
/// `open_stream` makes about what to run on the second channel, so a test that
/// made that decision itself would be testing its own opinion.
async fn first_byte(runner: &Runner, key: &Path, terminal: uuid::Uuid) -> Duration {
    let started = Instant::now();
    let mut session = Session::connect_ssh(&destination(runner, key))
        .await
        .expect("connect the product's client over a real sshd");
    let mut stream = session.open_stream(terminal).await.expect("open a stream");

    let mut buf = [0u8; 8 * 1024];
    let read = tokio::time::timeout(FIRST_BYTE_CEILING, stream.read(&mut buf)).await;
    match read {
        Ok(Ok(n)) if n > 0 => started.elapsed(),
        // Distinguished from the timeout on purpose: end of stream means the
        // channel opened and closed, which is what a pane that finished looks
        // like, and reporting that as slowness would send somebody to measure
        // the network.
        Ok(Ok(_)) => panic!("the stream ended without a byte"),
        Ok(Err(e)) => panic!("the stream failed: {e}"),
        Err(_) => FIRST_BYTE_CEILING + Duration::from_millis(1),
    }
}

/// Everything the stream sends in its first moment, through this key.
async fn replay_over(runner: &Runner, key: &Path, terminal: uuid::Uuid) -> Vec<u8> {
    let mut session = Session::connect_ssh(&destination(runner, key))
        .await
        .expect("connect the product's client over a real sshd");
    let mut stream = session.open_stream(terminal).await.expect("open a stream");

    // Read until it goes quiet rather than until it ends: a live pane never
    // ends, and the replay is whatever arrives before the pane says anything
    // new.
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    while let Ok(Ok(n)) =
        tokio::time::timeout(Duration::from_millis(600), stream.read(&mut buf)).await
    {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn destination(runner: &Runner, key: &Path) -> Destination {
    Destination {
        host: "127.0.0.1".into(),
        port: runner.port,
        user: whoami(),
        private_key: std::fs::read_to_string(key).expect("the private key"),
        passphrase: None,
        // A throwaway host key on a throwaway port. Pinning it would mean
        // reading it back out of a directory this test is about to delete.
        host_key: HostKeyPolicy::Accept,
    }
}

/// The account sshd will authenticate as, which is whoever is running the test.
fn whoami() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).expect("a user name")
}

// MARK: - a pane to watch

/// One workspace, one terminal, through the daemon's own socket.
///
/// Created locally rather than over ssh so that what is being measured is the
/// stream and not the ceremony in front of it.
async fn a_pane(runner: &Runner) -> uuid::Uuid {
    let mut client = local(runner).await;

    let repo = runner.work.join("demo");
    std::fs::create_dir_all(&repo).expect("the repository directory");
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .expect("git");
    }

    let mut add = rpc("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: runner.work.to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("repository_root.add");

    let mut register = rpc("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo.to_string_lossy().into_owned(),
        },
    ));
    let result = client.call(register).await.expect("repository.register");
    let Some(result::Value::Repository(repository)) = result.value else { panic!("wrong result") };

    let mut create = rpc("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(
        farcooler_protocol::v1::WorkspaceCreate {
            task_name: "streaming".into(),
            branch: "feat/streaming".into(),
            base_revision: "HEAD".into(),
            terminal_preset: String::new(),
            adopt_existing: false,
        },
    ));
    let result = client.call(create).await.expect("workspace.create");
    let Some(result::Value::Workspace(workspace)) = result.value else { panic!("wrong result") };

    let mut terminal = rpc("terminal.create");
    terminal.target_resource_id = Some(workspace.id.clone());
    terminal.payload = Some(request::Payload::TerminalCreate(
        farcooler_protocol::v1::TerminalCreate {
            title: "watched".into(),
            command_preset: "shell".into(),
            join_active_group: false,
        },
    ));
    let result = client.call(terminal).await.expect("terminal.create");
    let Some(result::Value::Terminal(terminal)) = result.value else { panic!("wrong result") };
    uuid::Uuid::from_slice(&terminal.id).expect("a terminal id")
}

async fn write_to(
    client: &mut Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>,
    terminal: uuid::Uuid,
    text: &str,
) {
    let mut write = rpc("terminal.write");
    write.target_resource_id = Some(bytes::Bytes::copy_from_slice(terminal.as_bytes()));
    write.payload = Some(request::Payload::TerminalWrite(farcooler_protocol::v1::TerminalWrite {
        payload: bytes::Bytes::copy_from_slice(text.as_bytes()),
    }));
    client.call(write).await.expect("terminal.write");
}

/// This runner's own socket, which is the owning user and therefore host_admin.
async fn local(
    runner: &Runner,
) -> Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf> {
    Client::connect(runner.runtime.join("farcoolerd.sock"), "test-admin", "0.0.0")
        .await
        .expect("handshake on the runner's own socket")
}

// MARK: - the runner

/// One runner: a daemon, an sshd in front of it, and two enrolled keys.
///
/// Field order is drop order, and it matters for the reason
/// `a_real_sshd_forces_the_scope.rs` spells out: `_dir` deletes the directory
/// `DaemonChild`'s Drop reads `install-id` out of to find the tmux server it has
/// to reap, so `_dir` is declared last and dropped last.
struct Runner {
    _daemon: DaemonChild,
    _sshd: Sshd,
    /// FARCOOLER_HOME, handed to the daemon on both sides.
    runtime: PathBuf,
    /// Where this test's repository lives. Outside the account home, so that
    /// adding a repository root does not hand the daemon the `.ssh` directory
    /// sshd is authenticating against.
    work: PathBuf,
    /// The private half of the key enrolled with a forced command — a phone.
    forced_key: PathBuf,
    /// The private half of the plain shell key — what a Mac also holds.
    plain_key: PathBuf,
    port: u16,
    _dir: tempfile::TempDir,
}

/// A real sshd, and the rule for ending it: by pid, never by pattern. This
/// machine runs the developer's own sshd, and a pattern kill would take their
/// remote access with it.
struct Sshd {
    pid: i32,
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // Only the listener is signalled. Each connection is served by a
        // separate `sshd-session` child, and each of those ends when its client
        // does.
        unsafe { libc::kill(self.pid, libc::SIGTERM) };
    }
}

async fn start() -> Runner {
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
    let work = root.join("work");
    std::fs::create_dir(&work).expect("the work directory");
    let bin = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin).expect("the bin directory");

    keygen(&root.join("host_ed25519"), "host");
    keygen(&root.join("forced"), "phone");
    keygen(&root.join("plain"), "mac");

    // Both lines from the shipped renderer, for the reason
    // `a_real_sshd_forces_the_scope.rs` gives: what is on trial is whether
    // OpenSSH accepts what `render` writes, so a hand-rolled line would be this
    // file asserting against its own idea of the format.
    //
    // `control`, because that is the scope `terminal.attach` requires and the
    // scope a phone that can type into a pane holds. A read-enrolled phone gets
    // no stream, and that is the table in `rpc.rs` working: a stream is the
    // screen continuously, and the screen is `control`.
    let forced = fence::render(
        std::fs::read_to_string(root.join("forced.pub")).expect("the phone's key").trim(),
        "Test Phone",
        "phone-7",
        Scope::Control,
        Grant::FarCooler,
        None,
    )
    .expect("render a Key A line");
    let plain = fence::render(
        std::fs::read_to_string(root.join("plain.pub")).expect("the Mac's key").trim(),
        "Test Mac",
        "mac-1",
        Scope::HostAdmin,
        Grant::Shell,
        None,
    )
    .expect("render a Key B line");

    let authorized_keys = home.join(".ssh").join("authorized_keys");
    fence::write(
        &authorized_keys,
        fence::AUTHORIZED_KEYS,
        &[forced.clone(), plain],
        &[],
        Placement::Last,
    )
    .expect("write the fence");
    assert_eq!(mode_of(&authorized_keys), 0o600, "the fence writer left the file too open");

    // The scaffolding, and only on the forced line, because only that line runs
    // a command sshd chose. See `a_real_sshd_forces_the_scope.rs`: `environment=`
    // coexists with `restrict` and carries no privilege, and it is the only way
    // to give the forced command a private runtime directory and a HOME without
    // a wrapper script. Everything after it is `render`'s output verbatim.
    //
    // The plain line needs none of it: sshd runs the account's login shell,
    // which inherits this test's own environment through neither — so `--stream`
    // and `--stdio` on that key would find the developer's real HOME. That is
    // what `PermitUserEnvironment` plus a second `environment=` prefix is for
    // here too.
    // The real directories follow ours, and they are not padding: sshd runs a
    // forced command through the account's LOGIN SHELL, whose startup files run
    // first, and a shell that cannot find `tty` or `mktemp` writes pages of its
    // own errors down the session's stderr.
    let scaffolding = format!(
        "environment=\"HOME={}\",environment=\"FARCOOLER_HOME={}\",\
         environment=\"PATH={}:/usr/bin:/bin:/usr/sbin:/sbin\"",
        home.display(),
        runtime.display(),
        bin.display(),
    );
    let written = std::fs::read_to_string(&authorized_keys).expect("read back the fence");
    let scaffolded: String = written
        .lines()
        .map(|line| {
            if line.starts_with('#') || line.trim().is_empty() {
                return line.to_string();
            }
            // Comma or space, and getting it wrong costs an hour. An
            // `authorized_keys` line is an OPTIONAL options field, then the key
            // — so options are joined to each other with commas and to the key
            // with a space. A Key A line already has options (`restrict`, the
            // forced command) and takes a comma; a Key B line is a bare key and
            // takes a space. A comma there makes sshd read `ssh-ed25519` as an
            // option name, reject the whole line, and answer the connection
            // with "Permission denied (publickey)" — which reads exactly like a
            // key that was never enrolled.
            let joiner = if line.starts_with("ssh-") || line.starts_with("ecdsa-")
                || line.starts_with("sk-")
            {
                ' '
            } else {
                ','
            };
            format!("{scaffolding}{joiner}{line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(scaffolded, written, "the rendered lines were not in the file just written");
    std::fs::write(&authorized_keys, &scaffolded).expect("prefix the test scaffolding");
    assert_eq!(mode_of(&authorized_keys), 0o600, "the scaffolding rewrite widened the mode");

    // The daemon the forced command runs, where the rendered line looks for it.
    // Resolved against the OVERRIDDEN home rather than the developer's real one:
    // this machine has a live Far Cooler install, and a link in the real
    // `~/.local/bin` would clobber it — or worse, be found and silently test it.
    let program = forced_program(&forced);
    let target = home.join(program.strip_prefix("~/").expect("a tilde-anchored forced command"));
    std::fs::create_dir_all(target.parent().expect("a parent")).expect("the bin directory");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_farcoolerd"), &target)
        .expect("link the daemon just built");
    // And under the name the plain key's `--stream` asks for, which is the same
    // path: a plain-key client names the binary the same way, it just is not
    // forced to.
    if !target.ends_with("farcoolerd") {
        let _ =
            std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_farcoolerd"), bin.join("farcoolerd"));
    }

    let port = free_port();
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

    // A daemon on the socket before the first connection, so the session sshd
    // starts takes the relay branch — the branch a real runner takes, and the
    // one where an attachment reaches the process that owns the tmux inventory.
    let daemon = listening_daemon_in(&runtime, &home).await;
    let sshd = Sshd { pid: await_sshd(&pid_file, port, &log).await };

    Runner {
        _daemon: daemon,
        _sshd: sshd,
        runtime,
        work,
        forced_key: root.join("forced"),
        plain_key: root.join("plain"),
        port,
        _dir: dir,
    }
}

/// The daemon that owns this runtime directory, with a home it may write into.
///
/// `common::listening_daemon` cannot be used as it stands: the daemon reads
/// `$HOME/.ssh/authorized_keys` with no override, so one that inherited the real
/// HOME would read the developer's own fence.
async fn listening_daemon_in(runtime: &Path, home: &Path) -> DaemonChild {
    let child = Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .env("FARCOOLER_HOME", runtime)
        .env("HOME", home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn farcoolerd");
    let daemon = DaemonChild { child, home: runtime.to_path_buf() };

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

/// An sshd that has written its pid and is answering on its port. Both, because
/// `sshd -f` forks and returns straight away.
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

/// `ssh`, configured the way `crates/cli/src/remote.rs::ssh_args` configures it.
fn ssh_command(runner: &Runner, key: &Path) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-p", &runner.port.to_string()])
        .arg("-i")
        .arg(key)
        .args(["-o", "IdentitiesOnly=yes"])
        .args(["-o", "ControlMaster=no"])
        .args(["-o", "BatchMode=yes"])
        .args(["-o", "StrictHostKeyChecking=no"])
        .args(["-o", "UserKnownHostsFile=/dev/null"])
        .arg("127.0.0.1");
    command
}

/// The program a rendered Key A line makes sshd run, read out of the line rather
/// than written down, so this follows the product's spelling wherever it goes.
fn forced_program(line: &str) -> String {
    let start = line.find("command=\"").expect("a forced command") + "command=\"".len();
    let rest = &line[start..];
    let end = rest.find('"').expect("a closing quote");
    rest[..end].split_whitespace().next().expect("a program").to_string()
}

fn keygen(path: &Path, comment: &str) {
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .status()
        .expect("run ssh-keygen");
    assert!(status.success(), "ssh-keygen failed for {}", path.display());
}

fn mode_of(path: &Path) -> u32 {
    std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
}

/// A port nothing is listening on, by asking the kernel for one and giving it
/// back. A fixed number would collide with whatever else this machine runs.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("addr").port()
}

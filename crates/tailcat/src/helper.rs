//! The backend that runs the tunnel as its own process.
//!
//! Linux runners use this instead of `linked.rs`. A Go c-archive linked into a
//! musl binary segfaults inside Go's runtime startup before it can print a
//! word — measured, and not this code; `crates/tailcat/go/exports_cgo.go`
//! carries the numbers, including the thread-stack sizes that do not fix it. A
//! standalone Go program built `CGO_ENABLED=0` links no C library at all, so
//! the problem does not arise rather than being worked around, and the binary
//! still runs on every distribution, which is the property musl was chosen for.
//!
//! **The data path is unchanged and the daemon is not in it either way.** Bytes
//! go from the helper's netstack straight to sshd on loopback, exactly as they
//! did from inside the daemon. What crosses the pipe here is control: start
//! serving, admit one more device, what is my token. Three messages a runner
//! sends a handful of times in its life — spawned when a tunnel is configured,
//! never per connection.
//!
//! On runtime cost this is arguably the better shape: the Go runtime is
//! resident only on a runner that actually serves a tunnel. A linked archive
//! carries it in every daemon, and most runners never configure a reach.
//!
//! ## What is deliberately not here
//!
//! `dial` is not implemented, and this is not an oversight to be tidied later.
//! Dialing hands its caller a FILE DESCRIPTOR — `linked.rs` adopts one out of
//! Go's `socketpair(2)`, and `crates/tailcat/go/tailcat.go`'s own header
//! explains why a descriptor rather than a loopback port — and a descriptor
//! cannot cross this pipe as a word. Whoever wires Linux dialing has to answer
//! the fd-passing question (`SCM_RIGHTS` over a Unix socketpair, presumably)
//! rather than extending the line protocol. Nothing on Linux dials one today:
//! `scripts/build-linux.sh` records that `farcooler-cli` does not link tailcat
//! at all, and this backend keeps that honest by answering `no_tailcat`.
//!
//! ## Why a helper that cannot be reached is `NoTailcatLinked`
//!
//! Two failures are worth telling apart, and the error each gets is what makes
//! `scripts/tunnel-smoke.sh` a real gate for this design rather than a proxy:
//!
//! - The helper could not be found, could not be started, or died without
//!   answering — a tarball that forgot to ship it, a binary for the wrong
//!   architecture, a file that is not executable. This build has no working
//!   tunnel, which is exactly what `NoTailcatLinked` means, so it reports
//!   `no_tailcat` and the smoke check goes red on the packaged artifact.
//! - The helper answered with an errno. It ran, it reached tailcat, and
//!   something about the runner's key or the network was wrong. That is `Io`,
//!   the same answer `linked.rs` gives for the same errno from the same call.

use super::TunnelError;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

/// Where to find the helper, when it is not next to this executable.
///
/// For a checkout and for the tests, which run a daemon out of `target/` with
/// nothing beside it. Named the way `FARCOOLER_HOME` is, and it is no wider a
/// hole than that one: anyone who can set this daemon's environment can
/// already run anything as this user.
const HELPER_PATH_ENV: &str = "FARCOOLER_TUNNEL_HELPER";

/// The running helper, or none.
///
/// One per process, mirroring the Go side's one-server-per-process: a daemon
/// has one runner and one sshd to front.
fn helper() -> &'static Mutex<Option<Helper>> {
    static HELPER: OnceLock<Mutex<Option<Helper>>> = OnceLock::new();
    HELPER.get_or_init(|| Mutex::new(None))
}

/// The DERP map URL to hand the next helper this process starts.
///
/// Recorded here as well as sent, because `set_derp_map_url` is deployment
/// configuration that may be set before anything serves — and the helper that
/// would receive it does not exist yet at that point.
fn derp_map_url() -> &'static Mutex<String> {
    static URL: OnceLock<Mutex<String>> = OnceLock::new();
    URL.get_or_init(|| Mutex::new(String::new()))
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Helper {
    /// Killed by PID and reaped, never left behind.
    ///
    /// Dropping the pipes alone would be enough eventually — the helper exits
    /// on EOF, which is its whole supervision story — but "eventually" is not
    /// good enough for the one caller that does this: `serve` replaces a
    /// running helper precisely so a revoked device stops being admitted, and
    /// two helpers briefly serving two allowlists is the state that would
    /// undo.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Helper {
    /// Sends one command and returns what came back, without the trailing
    /// newline.
    fn ask(&mut self, command: &str) -> Result<String, TunnelError> {
        writeln!(self.stdin, "{command}")?;
        self.stdin.flush()?;
        let mut reply = String::new();
        // Zero bytes is the helper having exited rather than answered: it
        // crashed, or it was killed. `UnexpectedEof` rather than a silent
        // empty string, so the caller cannot mistake it for a reply.
        if self.stdout.read_line(&mut reply)? == 0 {
            return Err(TunnelError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the tunnel helper exited without answering",
            )));
        }
        Ok(reply.trim_end_matches(['\r', '\n']).to_string())
    }
}

/// `ok`, `ok <payload>`, or `err <errno>` — the helper's whole vocabulary.
fn parse(reply: &str) -> Result<Option<&str>, TunnelError> {
    if let Some(rest) = reply.strip_prefix("err ") {
        let errno: i32 = rest.trim().parse().map_err(|_| unreadable(reply))?;
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(errno)));
    }
    if reply == "ok" {
        return Ok(None);
    }
    reply.strip_prefix("ok ").map(Some).ok_or_else(|| unreadable(reply))
}

fn unreadable(reply: &str) -> TunnelError {
    TunnelError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("the tunnel helper answered something this cannot read: {reply:?}"),
    ))
}

/// Where this runner's tunnel helper is.
///
/// Next to the running executable, the way `farcooler` finds `farcoolerd` next
/// to itself and for the same reason: the pair ship together, so a daemon
/// started from anywhere at all finds the helper that matches it rather than
/// whatever is on `PATH`.
///
/// By channel name first, then cargo's bare name — `farcooler_protocol`'s
/// `tunnel_binary_candidates` owns that order, and it is the same isolation
/// property the daemon's own lookup has: four channels share one
/// `~/.local/bin`, and a canary daemon must not spawn a stable helper.
fn helper_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os(HELPER_PATH_ENV) {
        if !from_env.is_empty() {
            return Some(PathBuf::from(from_env));
        }
    }
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    farcooler_protocol::CHANNEL
        .tunnel_binary_candidates()
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// Starts a helper and hands it whatever configuration it has to know before
/// it serves anything.
fn spawn(key_path: &Path) -> Result<Helper, TunnelError> {
    let Some(path) = helper_path() else {
        // Not `Io(ENOENT)`: this build has no tunnel it can serve, which is
        // the one thing `NoTailcatLinked` means, and reporting it as such is
        // what makes a tarball missing the helper fail `tunnel-smoke.sh`
        // rather than pass it.
        tracing::warn!(
            channel = farcooler_protocol::CHANNEL.as_str(),
            "no tunnel helper beside this daemon; this runner can serve no tunnel"
        );
        return Err(TunnelError::NoTailcatLinked);
    };

    let mut key_arg = OsString::from("--key=");
    key_arg.push(key_path.as_os_str());

    tracing::info!(helper = %path.display(), "starting the tunnel helper");
    let mut child = Command::new(&path)
        .arg(key_arg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Inherited, not piped. The helper logs what tailcat has to say, and
        // it belongs in the same place as everything else this daemon logs. A
        // pipe nobody drains would also block the helper once it filled.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| {
            tracing::warn!(helper = %path.display(), %error, "the tunnel helper would not start");
            TunnelError::NoTailcatLinked
        })?;

    let stdin = child.stdin.take().expect("stdin was piped");
    let stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));
    let mut helper = Helper { child, stdin, stdout };

    let url = derp_map_url().lock().expect("the DERP map URL lock").clone();
    if !url.is_empty() {
        // A helper that cannot be told this is a helper that cannot be
        // trusted to serve either, so the failure is not swallowed.
        parse(&helper.ask(&format!("derpmap {url}"))?)?;
    }
    Ok(helper)
}

pub async fn dial(
    _: &str,
    _: &str,
    _: u16,
) -> Result<tokio::net::UnixStream, TunnelError> {
    // See this module's header. A descriptor cannot cross the helper's pipe as
    // a word, so dialing is not something this backend can grow by adding a
    // command — and nothing on Linux dials today.
    Err(TunnelError::NoTailcatLinked)
}

pub fn serve(key_path: &Path, ssh_port: u16, allow: &[String]) -> Result<(), TunnelError> {
    // Refused BEFORE anything is spawned, and that ordering is the assertion
    // `crates/daemon/tests/an_empty_allowlist_starts_no_tunnel.rs` makes: no
    // allowlist, no helper process. Tailcat reads an empty `AllowedClients` as
    // "admit everyone", so a runner that got this far with nobody admitted
    // must end with no tunnel at all rather than one it then has to refuse.
    // The helper refuses it a second time, and so does the Go `serve` under
    // that — three guards on the one mistake that opens somebody's sshd.
    if allow.is_empty() {
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(libc::EINVAL)));
    }
    // A node key with whitespace in it would be read as several keys by the
    // helper's field split, which is a way to admit a device nobody enrolled.
    // The fence's own `usable_node_key` already forbids it upstream of here;
    // this is the same refusal at the boundary that would be exploited.
    if allow.iter().any(|key| key.split_whitespace().count() != 1) {
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(libc::EINVAL)));
    }

    let mut slot = helper().lock().expect("the tunnel helper lock");
    // Torn down first, exactly as the Go side's `serve` does: the allowlist is
    // read at Start and never again, so a helper left running keeps admitting
    // every device that was in the file when it started. Rebuilding is not
    // merely the available way to revoke a route, it is the only one.
    *slot = None;

    let mut helper = spawn(key_path)?;
    let outcome = helper
        .ask(&format!("serve {ssh_port} {}", allow.join(" ")))
        .and_then(|reply| parse(&reply).map(|_| ()));
    match outcome {
        Ok(()) => {
            *slot = Some(helper);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

pub fn conn_blob() -> Result<String, TunnelError> {
    let mut slot = helper().lock().expect("the tunnel helper lock");
    // ENOTCONN, the same errno the Go side answers `fc_tailcat_conn_blob` with
    // when no server is running, so both backends fail identically.
    let helper = slot.as_mut().ok_or_else(not_serving)?;
    let reply = helper.ask("blob")?;
    match parse(&reply)? {
        Some(blob) if !blob.is_empty() => Ok(blob.to_string()),
        // "ok" with nothing after it is not a token, and returning an empty
        // string would put this runner's manifest into the field carrying one.
        _ => Err(unreadable(&reply)),
    }
}

pub fn allow_add(node_key: &str) -> Result<(), TunnelError> {
    if node_key.split_whitespace().count() != 1 {
        return Err(TunnelError::Io(std::io::Error::from_raw_os_error(libc::EINVAL)));
    }
    let mut slot = helper().lock().expect("the tunnel helper lock");
    let helper = slot.as_mut().ok_or_else(not_serving)?;
    let reply = helper.ask(&format!("allow {node_key}"))?;
    parse(&reply).map(|_| ())
}

fn not_serving() -> TunnelError {
    TunnelError::Io(std::io::Error::from_raw_os_error(libc::ENOTCONN))
}

pub fn set_derp_map_url(url: &str) {
    if url.split_whitespace().count() > 1 {
        tracing::warn!("tailcat: the DERP map URL has whitespace in it; ignoring it");
        return;
    }
    *derp_map_url().lock().expect("the DERP map URL lock") = url.to_string();
    // A helper already running gets it now as well as at its next start.
    // Nothing rebuilds a live server for it — the URL is read when the server
    // is built — so this only matters for a `serve` that follows.
    let mut slot = helper().lock().expect("the tunnel helper lock");
    if let Some(helper) = slot.as_mut() {
        match helper.ask(&format!("derpmap {url}")).and_then(|r| parse(&r).map(|_| ())) {
            Ok(()) => {}
            Err(error) => tracing::warn!(%error, "tailcat: setting the DERP map URL failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the tests that touch this module's PROCESS-WIDE state — the
    /// running-helper slot and `FARCOOLER_TUNNEL_HELPER`. `cargo test` runs
    /// these in one process on many threads, so a test that starts a helper
    /// and one that asserts none is running would otherwise take turns
    /// failing.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// A helper that is a shell script: it appends every command it is given
    /// to `log`, and answers each with `ok`. Enough to be spawned, talked to,
    /// and asked what it heard.
    ///
    /// SAFETY for the `set_var`: every test that reads this variable holds
    /// `SERIAL`, and nothing else in this crate touches the environment.
    fn fake_helper(dir: &Path, log: &Path) {
        let script = dir.join("fake-tunnel-helper");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nwhile IFS= read -r line; do\n  printf '%s\\n' \"$line\" >> {}\n  echo ok\ndone\n",
                log.display()
            ),
        )
        .expect("the fake helper is written");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("the fake helper is executable");
        unsafe { std::env::set_var(HELPER_PATH_ENV, &script) };
    }

    /// The guard that matters, at the boundary that would be exploited: an
    /// allowlist admitting nobody must not reach a helper, and it must not
    /// start one either.
    ///
    /// This runs in a plain `cargo test --features helper` — no Go toolchain,
    /// no helper binary anywhere — which is the point. The helper path is
    /// pointed at something that cannot exist, so a `serve` that got as far as
    /// spawning would come back `NoTailcatLinked`. `EINVAL` is the only answer
    /// that proves it never tried.
    #[test]
    fn an_allowlist_admitting_nobody_never_reaches_a_helper() {
        let out = serve(Path::new("/nonexistent/tailcat.key"), 22, &[]);
        let Err(TunnelError::Io(error)) = out else {
            panic!("an empty allowlist was not refused: {out:?}");
        };
        assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{error:?}");
    }

    /// A node key carrying whitespace would arrive at the helper as two
    /// fields, which is two admitted devices — one of them nobody enrolled.
    #[test]
    fn a_node_key_with_whitespace_in_it_is_refused() {
        let out = serve(Path::new("/nonexistent/tailcat.key"), 22, &["a b".to_string()]);
        let Err(TunnelError::Io(error)) = out else {
            panic!("a node key with whitespace was not refused: {out:?}");
        };
        assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{error:?}");
        assert!(matches!(
            allow_add("a b"),
            Err(TunnelError::Io(ref e)) if e.raw_os_error() == Some(libc::EINVAL)
        ));
    }

    /// The whole vocabulary, including the answers that are not answers.
    #[test]
    fn a_reply_says_ok_a_payload_or_an_errno() {
        assert_eq!(parse("ok").unwrap(), None);
        assert_eq!(parse("ok tc-token").unwrap(), Some("tc-token"));
        let Err(TunnelError::Io(error)) = parse("err 22") else {
            panic!("an errno reply was not an error");
        };
        assert_eq!(error.raw_os_error(), Some(22));
        for nonsense in ["", "yes", "err", "err nope", "OK"] {
            assert!(parse(nonsense).is_err(), "{nonsense:?} was read as a reply");
        }
    }

    /// Asking about a tunnel nothing is serving is `ENOTCONN`, the same errno
    /// the C entry points answer with, and it must never be a spawn.
    #[test]
    fn asking_about_a_tunnel_that_is_not_serving_says_so() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *helper().lock().expect("the tunnel helper lock") = None;
        let Err(TunnelError::Io(error)) = conn_blob() else {
            panic!("conn_blob answered for a runner with no helper");
        };
        assert_eq!(error.raw_os_error(), Some(libc::ENOTCONN), "{error:?}");
    }

    /// Admitting one more device REACHES the running tunnel, rather than
    /// answering `ENOTCONN` because the tunnel now lives in another process.
    ///
    /// This is the question the migration path turns on. `client.set_node_key`
    /// lets a device register its node key over the SSH access it already
    /// holds, and calls `allow_add` so the route opens immediately instead of
    /// at the next tunnel start — the Go side's `AddAllowedClient` mutates a
    /// live server on purpose, because rebuilding would drop every other
    /// device's tunnel. Moving the tunnel into a child process is exactly the
    /// change that could have turned that into a silent no-op on Linux, so it
    /// is asserted here rather than reasoned about: after a successful
    /// `serve`, the helper is held open, and `allow_add` is a command down the
    /// same pipe.
    ///
    /// The fake helper records what it was actually sent, so this cannot pass
    /// on a call that returned `Ok` without saying anything.
    #[test]
    fn admitting_one_more_device_reaches_the_running_tunnel() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("a scratch directory");
        let log = dir.path().join("commands");
        fake_helper(dir.path(), &log);

        let key = dir.path().join("tailcat.key");
        serve(&key, 22, &["a".repeat(43)]).expect("the fake helper accepted serve");
        allow_add(&"b".repeat(43)).expect("allow_add reached the running helper");

        let heard = std::fs::read_to_string(&log).expect("the helper recorded what it heard");
        let lines: Vec<&str> = heard.lines().collect();
        assert_eq!(
            lines,
            [format!("serve 22 {}", "a".repeat(43)), format!("allow {}", "b".repeat(43))],
            "the helper did not hear both commands on one connection: {lines:?}"
        );

        // And the slot really is torn down between serves, which is what makes
        // a revocation revoke: the allowlist is read at Start and never again.
        *helper().lock().expect("the tunnel helper lock") = None;
        unsafe { std::env::remove_var(HELPER_PATH_ENV) };
    }

    /// Nothing on Linux dials a tunnel yet, and this backend cannot grow the
    /// ability by adding a command — see this module's header.
    #[tokio::test]
    async fn dialing_is_not_something_this_backend_does() {
        let out = dial("tc-anything", "key", 22).await;
        assert!(matches!(out, Err(TunnelError::NoTailcatLinked)), "{out:?}");
    }
}

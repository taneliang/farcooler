//! Reaching another runner.
//!
//! There is no Far Cooler network protocol. `farcooler --runner you@box ...` runs
//! `ssh you@box farcoolerd --stdio` and speaks the exact framing it speaks over
//! a local socket, because sshd has already authenticated both directions and
//! the caller is the same Unix user who owns the database there. Far Cooler adds
//! no certificate authority, no pairing flow, and no second set of credentials
//! to lose.
//!
//! Two consequences worth stating, because they shape everything here:
//!
//! - **The daemon binds no port, and there are two ways to reach one anyway.**
//!   This module is the first and the older one: SSH, where a runner reachable
//!   by SSH is reachable by Far Cooler and one that is not, is not — Tailscale,
//!   a bastion, or an `~/.ssh/config` alias is the answer to addressing, and
//!   not something Far Cooler reimplements. The second is the tunnel
//!   (`crates/tailcat`): an OUTBOUND connection a runner holds open, which an
//!   enrolled device dials to reach that runner's own sshd on loopback. It
//!   carries the same SSH, so nothing about authentication changes — but a
//!   tunneled runner needs no address and no inbound path, which is the half
//!   the sentence above no longer covers. That route is dialed in
//!   `crates/client`, which the apps embed; nothing in this module reaches it.
//! - **Live terminal bytes do not go through this.** Streaming runs the remote
//!   `farcooler` CLI over its own ssh session, which is a byte pipe already.
//!   Wrapping a multi-hour stream inside a request/response conversation would
//!   be the wrong shape for both.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use farcooler_transport::{Client, ClientError};
use tokio::process::{Child, Command};

use crate::runner_install;

type Reader = Box<dyn tokio::io::AsyncRead + Unpin + Send>;
type Writer = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

/// A daemon reached over ssh, plus the session carrying it.
pub struct RemoteLink {
    pub client: Client<Reader, Writer>,
    /// Handed to the caller to hold. The session must live exactly as long as
    /// the link, and `kill_on_drop` then ends it rather than leaving an ssh
    /// process behind.
    pub child: Child,
}

/// Options that apply to every ssh invocation.
///
/// `BatchMode=yes` matters: without it a host whose key has changed, or which
/// wants a password, hangs waiting for input that a GUI client will never
/// provide. Failing immediately with ssh's own message is far better than a
/// command that never returns.
///
/// The keepalives matter for the same reason in slower motion. A peer that
/// vanishes without closing its socket leaves ssh waiting forever on a
/// connection that will never carry another byte, and a client watching for the
/// process to exit waits just as long. `ServerAliveInterval` probes; three
/// unanswered probes end the session, so a dead host is noticed in about
/// forty-five seconds instead of never.
///
/// Fifteen seconds where `crates/client/src/ssh.rs` uses thirty: that path is
/// the phone, which pays for every probe in radio wake-ups. A Mac on mains
/// power does not, and halving the detection time matters for a tool whose
/// purpose is noticing when an agent needs you.
///
/// `ConnectTimeout` bounds the one failure `ControlMaster` can introduce: a
/// stale control socket left by a master that died badly, which a new
/// connection would otherwise wait on indefinitely.
fn ssh_args(target: &str, identity: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-o".into(), "BatchMode=yes".into()];

    match identity {
        // A named identity does NOT multiplex.
        //
        // The obvious fix — keep multiplexing and put the key in the
        // `ControlPath` — does not work, twice over. Hashing the key's PATH
        // does not identify the KEY, so replacing a key at the same path
        // silently reuses the master authenticated with the old one. And
        // `ControlPersist` keeps that master answering for two minutes after
        // the key is revoked: sshd reads `authorized_keys` at authentication
        // and a surviving master never authenticates again. A runner cannot
        // end it either, because `ssh -O exit` targets a socket on the
        // CLIENT's disk.
        //
        // So the cost of naming an identity is a handshake per connection.
        // That is the price of revocation meaning what it says.
        Some(path) => {
            args.push("-o".into());
            args.push("ControlMaster=no".into());
            args.push("-i".into());
            args.push(path.to_string_lossy().into_owned());
            // Bound to the key we named. Without this an agent holding a dozen
            // keys offers all of them and can exhaust `MaxAuthTries` before it
            // ever reaches this one.
            args.push("-o".into());
            args.push("IdentitiesOnly=yes".into());
        }
        // Multiplex over one connection so a burst of commands does not mean a
        // burst of TCP handshakes and key exchanges.
        //
        // Only when there is somewhere to put the socket. See `control_path`:
        // a home directory long enough to overflow `sun_path` leaves this
        // opening a connection per command, which is slower and works, rather
        // than naming a socket ssh cannot bind and failing every command with
        // what reads as an unreachable host.
        None => {
            if let Some(path) = control_path() {
                args.push("-o".into());
                args.push("ControlMaster=auto".into());
                args.push("-o".into());
                args.push(format!("ControlPath={path}"));
                args.push("-o".into());
                args.push("ControlPersist=120".into());
            }
        }
    }

    args.extend([
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        // Everything after this is a destination, never a flag.
        //
        // ssh reads a leading `-` as an option wherever it appears, so a
        // destination of `-oProxyCommand=...` is command execution on this
        // client, not on the runner. Every destination is typed by a human
        // today, which is why this has never mattered; onboarding has them
        // arrive in a scanned manifest, which is the transition that makes it
        // reachable.
        "--".into(),
        target.to_string(),
    ]);
    args
}

/// Where the unnamed-identity multiplexed master lives.
///
/// Carries the channel, like everything else a channel owns: a canary client
/// and a release one reaching the same destination must not share a master, any
/// more than they share a daemon or a runtime directory.
///
/// `%C` — ssh's own hash of `%l%h%p%r`, a fixed 40 hex characters — and NOT the
/// `%r@%h:%p` it stands for, because that spelling made the path a function of
/// how long the user and host names happen to be. A unix socket path is bounded
/// at 104 bytes on macOS, ssh appends a 17-character suffix of its own while
/// the master is coming up, and a real destination reached that ceiling:
///
///     unix_listener: path "/Users/…/.ssh/farcooler-canary-\
///     eliang_paraform_com@parasky1.intern.eliang.work:22.Gtlfh3xo5RhbkoaO"
///     too long for Unix domain socket
///
/// What made it worth finding twice: ssh reports that and exits non-zero, so
/// every command through this path reads as "could not reach the host". During
/// onboarding that is the command which appends the device's key to the
/// runner's `authorized_keys` — so the QR exchange completes, the key is never
/// written, and the failure names the network instead of the socket.
///
/// `%C` hashes the destination rather than spelling it, so the length no
/// longer grows with the host name. What it cannot make constant is the home
/// directory the socket sits under, so this answers `None` when even the
/// hashed path will not fit — and `ssh_args` then simply does not multiplex.
///
/// Degrading rather than failing is the point. Multiplexing is an optimization;
/// reaching the runner is not. A client whose home directory is long enough to
/// overflow the socket path should open one connection per command and be
/// slightly slower, which is a thing nobody notices, rather than be unable to
/// enroll a device, which is a thing that looks like a broken network.
///
/// Absolute rather than `~`, because a length this code cannot measure is a
/// length this code cannot bound.
fn control_path() -> Option<String> {
    control_path_under(&std::env::var("HOME").ok()?)
}

/// The socket path under `home`, or `None` if it cannot fit in one.
///
/// Split out from `control_path` so the bound can be tested against a home
/// directory this machine does not have — the real one is whatever it is, and
/// a test that only ever sees a short one proves nothing about the case that
/// broke.
fn control_path_under(home: &str) -> Option<String> {
    /// Characters usable in a `sun_path`, which is a 104-BYTE BUFFER on macOS
    /// and one of those bytes is the NUL terminator. Linux allows 108, so the
    /// smaller of the two is the one to hold to.
    ///
    /// 103 and not 104, and the difference is not academic: the path that
    /// produced the bug report measured exactly 104 with ssh's suffix on it and
    /// was refused. A bound of 104 would have called that one fine.
    const SUN_PATH_USABLE: usize = 103;
    /// What ssh appends to the path while the master is coming up, as in
    /// `….Gtlfh3xo5RhbkoaO`.
    const SSH_TEMP_SUFFIX: usize = 17;
    /// `%C` is a hash, so it is this long for every destination there is.
    const HASH: usize = 40;

    let path = format!("{home}/.ssh/fc-{}-%C", farcooler_protocol::CHANNEL.as_str());
    let expanded = path.len() - "%C".len() + HASH;
    (expanded + SSH_TEMP_SUFFIX <= SUN_PATH_USABLE).then_some(path)
}

/// What to run on the far side to get a daemon speaking the protocol.
///
/// `runner install` puts it in ~/.local/bin, but a non-login ssh command's PATH
/// is whatever the remote shell config gives it — often not that. Naming it by
/// tilde (expanded by the remote shell, not guessed here) finds it regardless
/// of PATH.
///
/// The name carries this build's channel, and no fallback: this asks a runner
/// for the daemon `runner install` put there, which is exactly one name. Falling
/// back to the bare `farcoolerd` would find the release daemon on any host
/// that has one and quietly join a preview client to it.
fn daemon_command() -> String {
    format!("~/.local/bin/{} --stdio", runner_install::daemon_name())
}

/// Open a protocol connection to a remote daemon.
///
/// `identity` names the key to authenticate with, or `None` to let ssh offer
/// whatever the agent and config volunteer — which is what every caller does
/// today and what a person at a shell expects. Device onboarding passes the
/// managed key, because a session that cannot say which key authenticated it
/// cannot be revoked by key either.
pub async fn connect(
    target: &str,
    identity: Option<&Path>,
) -> Result<RemoteLink, Box<dyn std::error::Error>> {
    let mut command = Command::new("ssh");
    command.args(ssh_args(target, identity));
    command.arg(daemon_command());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // stderr passes through, so the remote daemon's complaints and ssh's own
        // errors reach the user rather than vanishing.
        .stderr(Stdio::inherit())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|e| format!("cannot run ssh: {e}"))?;
    let stdin = child.stdin.take().ok_or("ssh gave no stdin")?;
    let stdout = child.stdout.take().ok_or("ssh gave no stdout")?;

    let result = Client::over(
        Box::new(stdout) as Reader,
        Box::new(stdin) as Writer,
        "farcooler-cli",
        env!("CARGO_PKG_VERSION"),
    )
    .await;

    let client = match result {
        Ok(client) => client,
        // Async, and given the `Child`: telling a connection failure apart
        // from a missing install needs ssh's own exit code, which only the
        // process can give us. See `explain` below.
        Err(e) => return Err(explain(target, e, &mut child).await.into()),
    };

    Ok(RemoteLink { client, child })
}

/// Turn a handshake failure into the thing the user has to fix.
async fn explain(target: &str, error: ClientError, child: &mut Child) -> String {
    match error {
        // The far side said nothing before the pipe closed, which happens in
        // two situations that read identically on the wire: `farcoolerd
        // --stdio` ran and never answered (worth blaming on the install), or
        // ssh never reached the target at all — refused, timed out, DNS
        // failure, rejected key — in which case the remote command was never
        // even attempted, and telling someone to install Far Cooler sends
        // them to fix the wrong thing entirely. ssh's own exit code is what
        // tells the two apart.
        ClientError::Closed | ClientError::NoHello if ssh_never_connected(child).await => format!(
            "Could not reach {target} over ssh.\n\
             Check the host name, your ssh config, and that the host is reachable."
        ),
        ClientError::Closed | ClientError::NoHello => not_installed(target),
        ClientError::VersionMismatch { daemon, client } => format!(
            "{target} runs protocol {daemon}; this client speaks {client}.\n\
             Update the older side:  {cli} runner install {target}",
            cli = runner_install::cli_name()
        ),
        other => format!("{target}: {other}"),
    }
}

/// Reached the host, got nothing back from the daemon.
///
/// Names this channel's binaries rather than Far Cooler in general. A preview
/// build that reported `farcoolerd` would send someone to check a binary it
/// never asked for — and one that is very likely present and healthy, since a
/// host running the release build has it — so the report would read as
/// nonsense. Naming the CLI in the fix has the same reason: a preview install
/// is done by `farcooler-preview`, and the bare `farcooler` may not be on that
/// person's host at all.
fn not_installed(target: &str) -> String {
    format!(
        "Connected to {target}, but `{daemon} --stdio` did not answer.\n\
         Is Far Cooler installed there?  {cli} runner install {target}",
        daemon = runner_install::daemon_name(),
        cli = runner_install::cli_name()
    )
}

/// Whether ssh itself never reached `target`, as opposed to reaching it and
/// the remote command not answering.
///
/// ssh's documented contract is the whole mechanism: it exits 255 for its
/// OWN failures (refused, timed out, no route, key rejected — the target was
/// never reached), and for every other outcome forwards the remote command's
/// exit status untouched. A shell failing to find `farcoolerd` on a reached
/// target exits nonzero too, but never with exactly 255 — that code is
/// reserved for ssh's own use, not something a login shell would pick by
/// coincidence — so checking for it specifically, rather than "any nonzero
/// exit", is what keeps a genuine missing-install report from being
/// misclassified as a connection failure.
///
/// By the time a handshake failure is observed the pipe has already closed,
/// which for ssh means the process has exited or is exiting this instant —
/// `try_wait` catches the already-exited case without blocking, and the
/// short bounded `wait` covers the small race where the exit has not been
/// reaped yet. If neither produces a status in time, this reports false: the
/// existing "is it installed" wording, not a new and possibly wrong claim.
async fn ssh_never_connected(child: &mut Child) -> bool {
    let status = match child.try_wait() {
        Ok(Some(status)) => Some(status),
        Ok(None) => {
            tokio::time::timeout(Duration::from_secs(2), child.wait()).await.ok().and_then(|r| r.ok())
        }
        Err(_) => None,
    };
    is_ssh_connection_failure(status)
}

/// Pure classification, kept separate from process-waiting so it can be
/// tested without spawning ssh: exit 255 is a connection failure, anything
/// else — including no status at all — is not.
fn is_ssh_connection_failure(status: Option<std::process::ExitStatus>) -> bool {
    status.and_then(|s| s.code()) == Some(255)
}

/// Run a command on the remote runner's own CLI, passing our streams straight
/// through.
///
/// Used for the live-byte commands. They are already a byte pipe, and ssh is a
/// byte pipe, so the honest implementation is to connect the two and get out of
/// the way. Returns the remote exit status.
pub async fn exec(
    target: &str,
    args: &[String],
    tty: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut command = Command::new("ssh");
    if tty {
        // A pty, so the remote program sees a terminal and Ctrl-C reaches it.
        command.arg("-t");
    }
    // The streaming path is a byte pipe for a person at a terminal, so it
    // uses ordinary ssh identity resolution like any other shell command.
    command.args(ssh_args(target, None));
    command.arg(cli_command(args));
    command.kill_on_drop(true);

    let status = command.status().await.map_err(|e| format!("cannot run ssh: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

/// What to run on the far side to reach the remote runner's own CLI.
///
/// Named by tilde, not bare: a non-login ssh exec's PATH often lacks
/// ~/.local/bin, where `runner install` puts the binary — same reason
/// `daemon_command` above does not exec a bare name either. And carrying this
/// build's channel for the same reason again: the CLI on that runner is the
/// one this CLI uploaded, and a channel that streamed through the release CLI
/// would be reading a different daemon's terminals.
fn cli_command(args: &[String]) -> String {
    format!("~/.local/bin/{} {}", runner_install::cli_name(), shell_join(args))
}

/// Quote arguments for the remote login shell.
///
/// Single quotes with the escape-and-reopen trick, so a task name containing a
/// space, a quote or a `$` reaches the remote CLI as one argument and is never
/// interpreted. ssh concatenates its command arguments and hands the result to
/// a shell, so anything not quoted here IS shell input.
fn shell_join(args: &[String]) -> String {
    args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

fn shell_quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@=+,".contains(c))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this file had: `runner install` uploads `farcoolerd-local`, and
    /// then every connection asked the runner for `farcoolerd`. On a host
    /// with nothing else installed that is a confusing "is Far Cooler
    /// installed there?" against a host where it plainly is. On a host
    /// that also runs the release build it is worse and silent — the local CLI
    /// gets a working session with the stable daemon, and the two channels
    /// that were supposed to never meet share a database.
    #[test]
    fn ssh_runs_this_channels_daemon_and_never_another() {
        let command = daemon_command();
        assert_eq!(
            command,
            format!("~/.local/bin/{} --stdio", farcooler_protocol::CHANNEL.daemon_binary_name())
        );
        if farcooler_protocol::CHANNEL != farcooler_protocol::Channel::Stable {
            assert!(
                !command.starts_with("~/.local/bin/farcoolerd "),
                "a {} build reached for the stable daemon: {command}",
                farcooler_protocol::CHANNEL.as_str()
            );
        }
    }

    /// Same rule for the live-byte path. It is a separate ssh invocation, so
    /// it was a separate hardcoded name and would have gone on working against
    /// the stable CLI after the control connection was fixed.
    #[test]
    fn a_streamed_command_runs_this_channels_cli() {
        let command = cli_command(&["terminal".into(), "stream".into()]);
        assert_eq!(
            command,
            format!(
                "~/.local/bin/{} terminal stream",
                farcooler_protocol::CHANNEL.cli_binary_name()
            )
        );
        if farcooler_protocol::CHANNEL != farcooler_protocol::Channel::Stable {
            assert!(!command.starts_with("~/.local/bin/farcooler "), "{command}");
        }
    }

    /// The wording sends someone somewhere. A preview build reporting that
    /// `farcoolerd` did not answer sends them to check a binary that is not
    /// the one it asked for, and `farcooler runner install` is a command they
    /// may not have — the same reason `crates/client/src/session.rs` names its
    /// own channel's daemon in the error it raises.
    #[test]
    fn a_missing_install_is_reported_against_the_binary_we_asked_for() {
        let message = not_installed("box");
        assert!(
            message.contains(farcooler_protocol::CHANNEL.daemon_binary_name()),
            "does not name the daemon it asked for: {message}"
        );
        assert!(
            message.contains(farcooler_protocol::CHANNEL.cli_binary_name()),
            "does not name the CLI that would fix it: {message}"
        );
    }

    #[test]
    fn plain_arguments_are_passed_through_unquoted() {
        assert_eq!(shell_join(&["workspace".into(), "list".into()]), "workspace list");
        assert_eq!(shell_quote("feat/auth-2"), "feat/auth-2");
        assert_eq!(shell_quote("--branch"), "--branch");
    }

    #[test]
    fn anything_a_shell_would_touch_is_quoted() {
        assert_eq!(shell_quote("add auth"), "'add auth'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(shell_quote("`whoami`"), "'`whoami`'");
    }

    #[test]
    fn a_quote_cannot_escape_its_own_quoting() {
        // The one case that breaks naive quoting, and the reason a task name is
        // not free to become a command.
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("'; rm -rf /; '"), r"''\''; rm -rf /; '\'''");
    }

    #[test]
    fn an_empty_argument_survives_as_an_empty_argument() {
        // Unquoted, it would disappear and shift every argument after it.
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn ssh_never_waits_for_input_it_will_not_get() {
        let args = ssh_args("box", None);
        assert!(args.windows(2).any(|w| w[0] == "-o" && w[1] == "BatchMode=yes"));
        assert_eq!(args.last().unwrap(), "box");
    }

    /// The options that turn a silent death into an event.
    ///
    /// Without these, a peer that goes away without closing the socket — a lid
    /// that closes, a VPN that drops, a server that reboots — leaves ssh parked
    /// on a half-open TCP connection indefinitely. The process never exits, so
    /// nothing above it ever learns, and the runner keeps looking connected
    /// while delivering nothing. There is no signal for a client to react to,
    /// which is why this cannot be fixed any higher up.
    ///
    /// Asserted by name because their absence is invisible until a connection
    /// dies, which is exactly when nobody is reading test output.
    ///
    /// The values are asserted exactly, not just their presence, because the
    /// timing is the behavior here, not an implementation detail: silently
    /// loosening `ServerAliveInterval` from 15 to 999 would mean fifty minutes
    /// to notice a dead host instead of forty-five seconds, and that change
    /// should have to touch this test on purpose.
    #[test]
    fn ssh_notices_a_peer_that_stopped_answering() {
        let args = ssh_args("box", None);
        let has_pair =
            |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(
            has_pair(["-o", "ServerAliveInterval=15"]),
            "no 15s liveness probe: {args:?}"
        );
        assert!(has_pair(["-o", "ServerAliveCountMax=3"]), "no probe limit of 3: {args:?}");
        assert!(
            has_pair(["-o", "ConnectTimeout=10"]),
            "a stale control socket could hang: {args:?}"
        );
    }

    /// A never-reachable host must be told apart from a reachable one
    /// missing the install — this is the review that caught it: a
    /// `.notInstalled` dot (grey) instead of `.unreachable` (red), plus
    /// `DaemonClient`'s slow five-minute retry cadence for what should back
    /// off fast and reconnect promptly, both keyed off `explain()`'s
    /// wording. ssh's exit code 255 is the whole distinction: it is ssh's
    /// own reserved code for a connection it never made, which is why
    /// `is_ssh_connection_failure` checks for it exactly rather than for
    /// "any nonzero code".
    #[cfg(unix)]
    #[test]
    fn ssh_reports_a_connection_it_never_made_as_255() {
        use std::os::unix::process::ExitStatusExt;
        let never_connected = std::process::ExitStatus::from_raw(255 << 8);
        assert!(is_ssh_connection_failure(Some(never_connected)));
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_remote_command_is_not_mistaken_for_a_connection_failure() {
        use std::os::unix::process::ExitStatusExt;
        // A login shell reporting "command not found" for a reached target —
        // 127 is the traditional code, but the point is any code other than
        // ssh's own 255. Misreading this as a connection failure would send
        // someone to check their network for a host that answered fine.
        let command_not_found = std::process::ExitStatus::from_raw(127 << 8);
        assert!(!is_ssh_connection_failure(Some(command_not_found)));
    }

    #[test]
    fn no_exit_status_yet_is_not_assumed_to_be_a_connection_failure() {
        // Where the bounded wait in `ssh_never_connected` timing out lands.
        // Silence about which failure this is beats confidently naming the
        // wrong one.
        assert!(!is_ssh_connection_failure(None));
    }

    /// `--` before the destination, or an address is an option.
    ///
    /// ssh reads a leading `-` as a flag wherever it appears, so a destination
    /// of `-oProxyCommand=...` runs a command on the CLIENT. Nothing types
    /// that today — every destination is typed by a human — but device
    /// onboarding has them arrive in a scanned manifest, and that is the
    /// transition that turns a latent hazard into a vulnerability.
    #[test]
    fn the_destination_cannot_be_read_as_an_option() {
        let args = ssh_args("-oProxyCommand=touch /tmp/pwned", None);
        let end = args.iter().position(|a| a == "--").expect("no -- before the destination");
        let target =
            args.iter().position(|a| a.starts_with("-oProxyCommand")).expect("no destination");
        assert!(end < target, "-- must come before the destination: {args:?}");
        assert_eq!(target, args.len() - 1, "the destination is last: {args:?}");
    }

    #[test]
    fn an_ordinary_destination_is_still_last() {
        let args = ssh_args("you@box", None);
        assert_eq!(args.last().map(String::as_str), Some("you@box"));
        assert_eq!(args[args.len() - 2], "--");
    }

    /// A named identity is the only identity offered.
    #[test]
    fn a_named_identity_is_the_only_one_offered() {
        let args = ssh_args("you@box", Some(Path::new("/tmp/farcooler-key")));
        let has_pair =
            |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has_pair(["-i", "/tmp/farcooler-key"]), "no -i: {args:?}");
        assert!(has_pair(["-o", "IdentitiesOnly=yes"]), "no IdentitiesOnly: {args:?}");
    }

    /// A named identity does not multiplex, so revocation is not outlived.
    ///
    /// Keying the control path on the key instead would not fix it: a path
    /// names a file, not the key inside it, and `ControlPersist` keeps a master
    /// answering after the key is gone. A runner cannot close that master —
    /// `ssh -O exit` reaches a socket on the client's own disk.
    #[test]
    fn a_named_identity_does_not_multiplex() {
        let args = ssh_args("you@box", Some(Path::new("/tmp/k")));
        let has_pair =
            |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert!(has_pair(["-o", "ControlMaster=no"]), "still multiplexing: {args:?}");
        assert!(
            !args.iter().any(|a| a.starts_with("ControlPersist")),
            "a persistent master outlives revocation: {args:?}"
        );
    }

    /// The unnamed path still multiplexes, and its socket carries the channel.
    ///
    /// Conditional on this machine having a home directory a socket fits
    /// under, because that is now what decides whether there is a socket at
    /// all — see `control_path`. Asserted as an equivalence rather than
    /// skipped, so the two halves cannot drift into disagreeing: a
    /// `ControlPath` argument appears exactly when `control_path` yields one.
    #[test]
    fn the_unnamed_path_multiplexes_per_channel() {
        let args = ssh_args("you@box", None);
        let found = args.iter().find(|a| a.starts_with("ControlPath="));

        match control_path() {
            Some(_) => {
                let path = found.expect("no ControlPath when one fits");
                assert!(
                    path.contains(farcooler_protocol::CHANNEL.as_str()),
                    "a channel's master is its own: {path}"
                );
            }
            None => assert!(found.is_none(), "named a socket that cannot be bound: {found:?}"),
        }
    }

    /// The socket path fits, whatever the destination is called.
    ///
    /// A unix socket path is bounded at 104 bytes on macOS and ssh adds a
    /// 17-character suffix of its own while the master comes up. Spelled
    /// `%r@%h:%p`, the path grew with the user and host names and a real
    /// destination went over:
    ///
    ///     unix_listener: path "…/farcooler-canary-\
    ///     eliang_paraform_com@parasky1.intern.eliang.work:22.Gtlfh3xo5RhbkoaO"
    ///     too long for Unix domain socket
    ///
    /// It presents as "could not reach the host" — including for the command
    /// that writes a device's key into `authorized_keys`, which is how a QR
    /// exchange completes without enrolling anything.
    ///
    /// `%C` is 40 characters for every destination there is, so the host can
    /// no longer push it over. This pins that for a long one.
    #[test]
    fn the_control_path_does_not_grow_with_the_destination() {
        // 103, not 104: the buffer is 104 bytes and one is the NUL. The path
        // in the report below measured exactly 104 and was refused.
        const SUN_PATH_USABLE: usize = 103;
        const SSH_TEMP_SUFFIX: usize = 17;
        const HASH: usize = 40;

        let home = "/Users/e-liang";
        let path = control_path_under(home).expect("an ordinary home should multiplex");
        assert!(path.contains("%C"), "the path must hash its destination: {path}");
        assert!(!path.contains("%h"), "the host must not be spelled out: {path}");

        let expanded = path.replace("%C", &"c".repeat(HASH)).len() + SSH_TEMP_SUFFIX;
        assert!(
            expanded <= SUN_PATH_USABLE,
            "control path needs {expanded} bytes, {SUN_PATH_USABLE} available: {path}"
        );

        // The exact path from the report, under the same home, against the
        // same bound — so the rule that was too loose stays caught.
        let broke = format!(
            "{home}/.ssh/farcooler-canary-\
             eliang_paraform_com@parasky1.intern.eliang.work:22"
        );
        assert!(
            broke.len() + SSH_TEMP_SUFFIX > SUN_PATH_USABLE,
            "the path that actually failed must not measure as fitting"
        );
    }

    /// A home directory too long for a socket gives up multiplexing, not the
    /// connection.
    ///
    /// The hash bounds the destination; nothing bounds the home directory. So
    /// the remaining case is answered by not naming a socket at all — one
    /// connection per command is slower and works, and this is the difference
    /// between a client that is slightly slower and a client that cannot
    /// enroll a device.
    #[test]
    fn an_unfittable_home_stops_multiplexing_rather_than_failing() {
        let absurd = format!("/Users/{}", "x".repeat(120));
        assert_eq!(control_path_under(&absurd), None, "named a socket that cannot be bound");
    }

    #[test]
    fn the_destination_is_still_last_with_an_identity() {
        let args = ssh_args("you@box", Some(Path::new("/tmp/k")));
        assert_eq!(args.last().map(String::as_str), Some("you@box"));
        assert_eq!(args[args.len() - 2], "--");
    }
}

//! Reaching another machine.
//!
//! There is no Far Cooler network protocol. `farcooler --host you@box ...` runs
//! `ssh you@box farcoolerd --stdio` and speaks the exact framing it speaks over
//! a local socket, because sshd has already authenticated both directions and
//! the caller is the same Unix user who owns the database there. Far Cooler adds
//! no certificate authority, no pairing flow, and no second set of credentials
//! to lose.
//!
//! Two consequences worth stating, because they shape everything here:
//!
//! - **No port is opened anywhere.** A host that is reachable by SSH is
//!   reachable by Far Cooler, and one that is not, is not. Tailscale or a
//!   bastion is the answer to reachability, not something Far Cooler reimplements.
//! - **Live terminal bytes do not go through this.** Streaming runs the remote
//!   `farcooler` CLI over its own ssh session, which is a byte pipe already.
//!   Wrapping a multi-hour stream inside a request/response conversation would
//!   be the wrong shape for both.

use std::process::Stdio;
use std::time::Duration;

use farcooler_transport::{Client, ClientError};
use tokio::process::{Child, Command};

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
/// unanswered probes end the session, so a dead machine is noticed in about
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
fn ssh_args(target: &str) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        // Multiplex over one connection so a burst of commands does not mean a
        // burst of TCP handshakes and key exchanges.
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        "ControlPath=~/.ssh/farcooler-%r@%h:%p".into(),
        "-o".into(),
        "ControlPersist=120".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        target.to_string(),
    ]
}

/// Open a protocol connection to a remote daemon.
pub async fn connect(target: &str) -> Result<RemoteLink, Box<dyn std::error::Error>> {
    let mut command = Command::new("ssh");
    command.args(ssh_args(target));
    // `host install` puts it in ~/.local/bin, but a non-login ssh command's
    // PATH is whatever the remote shell config gives it — often not that.
    // Naming it by tilde (expanded by the remote shell, not guessed here)
    // finds it regardless of PATH.
    command.arg("~/.local/bin/farcoolerd --stdio");
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
             Check the host name, your ssh config, and that the machine is reachable."
        ),
        ClientError::Closed | ClientError::NoHello => format!(
            "Connected to {target}, but `farcoolerd --stdio` did not answer.\n\
             Is Far Cooler installed there?  farcooler host install {target}"
        ),
        ClientError::VersionMismatch { daemon, client } => format!(
            "{target} runs protocol {daemon}; this client speaks {client}.\n\
             Update the older side:  farcooler host install {target}"
        ),
        other => format!("{target}: {other}"),
    }
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

/// Run a command on the remote host's own CLI, passing our streams straight
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
    command.args(ssh_args(target));
    // Named by tilde, not bare: a non-login ssh exec's PATH often lacks
    // ~/.local/bin, where `host install` puts the binary — same reason
    // `connect` above does not exec a bare `farcoolerd` either.
    command.arg(format!("~/.local/bin/farcooler {}", shell_join(args)));
    command.kill_on_drop(true);

    let status = command.status().await.map_err(|e| format!("cannot run ssh: {e}"))?;
    Ok(status.code().unwrap_or(1))
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
        let args = ssh_args("box");
        assert!(args.windows(2).any(|w| w[0] == "-o" && w[1] == "BatchMode=yes"));
        assert_eq!(args.last().unwrap(), "box");
    }

    /// The options that turn a silent death into an event.
    ///
    /// Without these, a peer that goes away without closing the socket — a lid
    /// that closes, a VPN that drops, a server that reboots — leaves ssh parked
    /// on a half-open TCP connection indefinitely. The process never exits, so
    /// nothing above it ever learns, and the machine keeps looking connected
    /// while delivering nothing. There is no signal for a client to react to,
    /// which is why this cannot be fixed any higher up.
    ///
    /// Asserted by name because their absence is invisible until a connection
    /// dies, which is exactly when nobody is reading test output.
    ///
    /// The values are asserted exactly, not just their presence, because the
    /// timing is the behavior here, not an implementation detail: silently
    /// loosening `ServerAliveInterval` from 15 to 999 would mean fifty minutes
    /// to notice a dead machine instead of forty-five seconds, and that change
    /// should have to touch this test on purpose.
    #[test]
    fn ssh_notices_a_peer_that_stopped_answering() {
        let args = ssh_args("box");
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

    /// A never-reachable machine must be told apart from a reachable one
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
        // someone to check their network for a machine that answered fine.
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
}

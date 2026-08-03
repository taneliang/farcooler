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
        target.to_string(),
    ]
}

/// Open a protocol connection to a remote daemon.
pub async fn connect(target: &str) -> Result<RemoteLink, Box<dyn std::error::Error>> {
    let mut command = Command::new("ssh");
    command.args(ssh_args(target));
    // `farcoolerd` is found on the remote user's PATH. `host install` puts it in
    // ~/.local/bin, and the login shell is what puts that on PATH, so this runs
    // through a shell rather than exec'ing an absolute path we would have to
    // guess.
    command.arg("farcoolerd --stdio");
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

    let client = Client::over(
        Box::new(stdout) as Reader,
        Box::new(stdin) as Writer,
        "farcooler-cli",
        env!("CARGO_PKG_VERSION"),
    )
    .await
    .map_err(|e| explain(target, e))?;

    Ok(RemoteLink { client, child })
}

/// Turn a handshake failure into the thing the user has to fix.
fn explain(target: &str, error: ClientError) -> String {
    match error {
        // The far side said nothing, which nearly always means the command did
        // not exist rather than that the protocol went wrong.
        ClientError::Closed | ClientError::NoHello => format!(
            "Connected to {target}, but `farcoolerd --stdio` did not answer.\n\
             Is FarCooler installed there?  farcooler host install {target}"
        ),
        ClientError::VersionMismatch { daemon, client } => format!(
            "{target} runs protocol {daemon}; this client speaks {client}.\n\
             Update the older side:  farcooler host install {target}"
        ),
        other => format!("{target}: {other}"),
    }
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
    command.arg(format!("farcooler {}", shell_join(args)));
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
}

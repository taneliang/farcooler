//! Getting a connection to a pane's fanout, starting one if there is none.

use farcooler_core::{DomainError, Result};

use super::Runtime;

impl Runtime {
    /// Get a connection to this pane's fanout, starting one if there is none.
    ///
    /// Connect first, ask questions later: a running fanout is the common case
    /// once anything is watching, and a connection succeeding is a better test
    /// that one is alive than any amount of asking tmux, which would happily
    /// report a pipe into a process that has since died.
    pub(crate) async fn attach_to_fanout(&self, pane_id: &str) -> Result<tokio::net::UnixStream> {
        // Which install is asking. The tmux socket is already named for it —
        // `farcooler-<install id>` — so this is the identity we already hold
        // rather than a second answer to the same question, which could
        // disagree with the first.
        //
        // It is load-bearing. Without it, two daemons on one host share a
        // fanout socket per pane NUMBER, and every tmux server numbers from
        // `%0`: the second daemon connects to the first one's fanout, never
        // starts a pipe of its own, and reads a stranger's pane. See
        // `fanout::socket_path`.
        let install = self.tmux.socket().to_string();
        if let Some(socket) = crate::fanout::subscribe(&install, pane_id).await {
            return Ok(socket);
        }

        let exe = fanout_binary().ok_or_else(|| {
            tracing::warn!("cannot find farcoolerd to pipe this pane into");
            DomainError::OperationFailed
        })?;
        self.start_fanout(pane_id, &install, &exe).await
    }

    /// Pipe the pane into a new fanout run from `exe`, and connect to it once
    /// it is listening. `attach_to_fanout`'s second half, given the binary so
    /// a test can hand it one that is slow to start.
    pub(crate) async fn start_fanout(&self, pane_id: &str, install: &str, exe: &std::path::Path) -> Result<tokio::net::UnixStream> {
        // See `fanout::pipe_command` for what is in it and why.
        let command = crate::fanout::pipe_command(exe, pane_id, install);
        self.tmux.pipe_pane_start(pane_id, &command).await?;

        // Retried rather than slept through: the fanout has a process to spawn
        // and a socket to bind before it can be connected to, and how long that
        // takes belongs to the machine, not to a number chosen here.
        //
        // Probed BEFORE the first sleep, which is the difference between "as
        // fast as the machine can manage" and "never faster than 10ms". The
        // socket is frequently already there: `pipe_pane_start` above has
        // already awaited tmux, and within the fanout's five-second idle grace
        // a pane that was open moments ago still has a live one to connect to —
        // the common case when someone switches away from a layout and back.
        // Sleeping first put a floor under every one of those.
        let deadline = std::time::Instant::now() + FANOUT_START;
        loop {
            if let Some(socket) = crate::fanout::subscribe(install, pane_id).await {
                return Ok(socket);
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tracing::warn!(pane = pane_id, "the pane fanout never came up");
        Err(DomainError::OperationFailed)
    }
}

/// How long a new fanout has to start listening before the pane it was
/// piped is given up on (ov-1051).
///
/// The fanout is a process tmux spawns, through `sh`, from `farcoolerd`, and
/// the first time macOS runs a binary it assesses it first: a fresh
/// `farcoolerd` took 0.87-0.94 s to exec once and 10 ms every time after, at
/// a load of 64. The first stream after an update starts a fresh one, and so
/// does the first test after a build. The budget was a second, so a cold start
/// on a busy machine ended a stream that would have worked, and at 62 s it
/// failed the stream_end tests in full runs. The cost of waiting longer falls
/// only on a fanout that is never coming, where 10 s is still an answer.
const FANOUT_START: std::time::Duration = std::time::Duration::from_secs(10);

/// The binary that serves a pane fanout: `farcoolerd`, never `farcooler`.
///
/// NOT `current_exe()`. `--fanout` is the daemon's flag, but this code also
/// runs inside the CLI — `farcooler terminal stream` opens a Runtime of its
/// own — so `current_exe()` there is the CLI, which pipes the pane into
/// `farcooler --fanout N`, gets "unexpected argument", and never binds the
/// socket. The stream then times out after `FANOUT_START` and exits, which a client
/// cannot tell apart from a pane that finished: keystrokes land, the pane
/// changes, and the screen simply never updates.
///
/// Same shape as `service::cli_binary`, and for the same reason: two binaries
/// come out of this workspace and only one of them answers any given flag.
///
/// By candidate name rather than the bare literal, because the name on disk
/// depends on the channel. A preview daemon that piped a pane into the release
/// `farcoolerd` standing beside it in `~/.local/bin` would hand this pane's
/// bytes to a daemon on the other side of the isolation — and the fanout
/// socket it then bound would be one this daemon never looks at.
///
/// Unit tests use the shared Cargo binary lookup: the depth below the
/// profile directory changed in Rust 1.100 (ov-1125).
pub fn fanout_binary() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let found = fanout_binary_beside(&exe);
    #[cfg(test)]
    let found = found.or_else(test_binary::daemon_binary);
    found
}

#[cfg(test)]
#[path = "../../../../test/daemon_binary.rs"]
mod test_binary;

/// The choice `fanout_binary` makes, given the executable it is made from, so
/// it can be tested without one.
fn fanout_binary_beside(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let candidates = farcooler_protocol::CHANNEL.daemon_binary_candidates();
    if exe.file_name().is_some_and(|n| candidates.iter().any(|c| n == *c)) {
        return Some(exe.to_path_buf());
    }
    let dir = exe.parent()?;
    candidates.iter().map(|name| dir.join(name)).find(|p| p.exists())
}

#[cfg(test)]
mod fanout_binary_tests {
    use super::fanout_binary_beside;
    use farcooler_protocol::CHANNEL;

    /// A `~/.local/bin` with two channels installed in it holds two daemons,
    /// and the pane has to go to this one. The other one's fanout listens on a
    /// socket this daemon never subscribes to, so the stream would time out
    /// after `FANOUT_START` and exit — which a client cannot tell apart from a pane
    /// that finished.
    #[test]
    fn the_pane_goes_to_this_channels_daemon() {
        let dir = tempfile::tempdir().unwrap();
        for name in CHANNEL.daemon_binary_candidates() {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        // Asked from the CLI, which is where this matters: the CLI opens a
        // Runtime of its own for `terminal stream` and must not pipe the pane
        // into itself.
        let cli = dir.path().join(CHANNEL.cli_binary_name());
        assert_eq!(
            fanout_binary_beside(&cli),
            Some(dir.path().join(CHANNEL.daemon_binary_name()))
        );
    }

    /// Cargo renames nothing, so a checkout still finds what it just built.
    #[test]
    fn a_cargo_target_directory_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("farcoolerd"), b"").unwrap();
        assert_eq!(
            fanout_binary_beside(&dir.path().join("farcooler")),
            Some(dir.path().join("farcoolerd"))
        );
    }

    /// The daemon asking about itself answers with itself, without touching
    /// the filesystem — `--fanout` is its own flag.
    #[test]
    fn the_daemon_is_its_own_fanout() {
        let exe = std::path::Path::new("/nowhere").join(CHANNEL.daemon_binary_name());
        assert_eq!(fanout_binary_beside(&exe), Some(exe));
    }
}

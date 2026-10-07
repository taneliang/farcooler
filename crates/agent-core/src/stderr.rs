//! Keeping an agent's stderr, so a handshake that dies can say why (ov-410, ov-414).
//!
//! On Oct 7 npm published a codex launcher without its platform binary, and all
//! CI could report was "closed without answering": the handshake had sent the
//! launcher's stderr to /dev/null, which is where the one line naming the cause
//! was printed. Every backend's handshake and startup read keeps stderr now,
//! through the same three functions, so no agent can fail more quietly than
//! another.

use std::sync::mpsc::Receiver;
use std::time::Duration;

/// How long a failed handshake waits for a dead agent's stderr to end.
pub const GRACE: Duration = Duration::from_secs(5);

/// The one line of a dead agent's stderr that says why it died.
///
/// Node's uncaught-error report opens with the source line and a caret and
/// closes with a stack and the node version, so neither the first nor the last
/// line is the reason: the line starting `Error:` is. A Rust agent that refuses
/// its arguments says `error: …` instead. Anything else falls back to the last
/// thing said. Capped, because this ends up in a one-line status.
pub fn reason_in(stderr: &str) -> Option<String> {
    let lines = || stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines()
        .find(|l| l.starts_with("Error:") || l.starts_with("error:"))
        .or_else(|| lines().next_back())?;
    const CAP: usize = 300;
    if line.chars().count() > CAP {
        Some(line.chars().take(CAP).chain(['…']).collect())
    } else {
        Some(line.to_string())
    }
}

/// The head of a stream, for a reader that has to read it all.
///
/// Read to the end rather than stopping at the cap, so a chatty child never
/// blocks on a full pipe; only the first 64 KiB is kept, which is where a
/// startup failure puts its reason.
#[derive(Default)]
pub struct Head(Vec<u8>);

impl Head {
    const KEEP: usize = 64 * 1024;

    pub fn push(&mut self, chunk: &[u8]) {
        let room = Self::KEEP.saturating_sub(self.0.len());
        self.0.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

/// Read a child's stderr to its end on a thread of its own, keeping the head.
pub fn drain(mut stderr: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Head::default();
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            kept.push(&chunk[..n]);
        }
        let _ = tx.send(kept.text());
    });
    rx
}

/// What a drained stderr says, waiting up to [`GRACE`] for it to end.
///
/// Bounded because a grandchild could still hold the pipe open. Blocking: an
/// async caller runs it on a blocking thread.
pub fn why(stderr: &Receiver<String>) -> Option<String> {
    stderr.recv_timeout(GRACE).ok().as_deref().and_then(reason_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_is_the_error_line_or_else_the_last_thing_said() {
        assert_eq!(
            reason_in("file:///x.js:1\n\nError: it broke\n    at f (x.js:1)\n\nNode.js v24\n")
                .as_deref(),
            Some("Error: it broke")
        );
        assert_eq!(
            reason_in("starting\nerror: unexpected argument 'app-server'\n").as_deref(),
            Some("error: unexpected argument 'app-server'")
        );
        assert_eq!(reason_in("first\nlast words\n\n").as_deref(), Some("last words"));
        assert_eq!(reason_in(" \n\n"), None);
        assert_eq!(reason_in(&"x".repeat(1000)).map(|r| r.chars().count()), Some(301));
    }

    #[test]
    fn a_drained_stream_is_read_past_the_cap_and_keeps_its_head() {
        let mut said = b"Error: first\n".to_vec();
        said.extend(std::iter::repeat_n(b'x', 200_000));
        let rx = drain(std::io::Cursor::new(said));
        assert_eq!(why(&rx).as_deref(), Some("Error: first"));
    }
}

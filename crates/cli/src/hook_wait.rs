//! `farcooler hook wait --mailbox <dir>`: claude's waiter (ov-563).
//!
//! Registered with `asyncRewake` on SessionStart and Stop. It reads the hook
//! payload for the session's id, then waits in that session's mailbox
//! (`farcooler_agent_hooks::mailbox`) and, when the daemon drops a message,
//! prints it on stderr and exits 42, which the hook command makes the 2 that wakes
//! the model with it.
//!
//! **It leaves when its mailbox does**: the daemon removes the mailbox of a
//! session its pane no longer has (`claude_mailbox::sweep`).
//!
//! **It ends with its session.** A waiter outlives a claude that was killed,
//! and would then hold the mailbox against the next session. So it records its
//! ancestors (the shell claude ran it through, claude, and above) and leaves
//! when any of them is gone. `claim_waiter` also clears a pid file left by a
//! process that no longer exists.
//!
//! Like every hook it is never in the way: whatever goes wrong, it exits 0 and
//! prints nothing.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use farcooler_agent_hooks::mailbox::{Mailbox, Waited, pid_alive};

/// What it exits with when it has a message. Not 2: the hook command maps 42
/// to the 2 claude reads as a wake (`claude_mailbox::wrapped`), so a `farcooler`
/// that fails or doesn't know this subcommand (clap's usage error is 2) can't
/// wake claude with its error text.
pub const WOKE: i32 = 42;

/// How often the mailbox and the session are looked at.
const POLL: Duration = Duration::from_millis(200);

/// How long claude has to hand over the payload.
const PAYLOAD_PATIENCE: Duration = Duration::from_secs(10);

/// The most ancestors followed up from this process.
const MOST_ANCESTORS: usize = 8;

/// What `farcooler hook` does besides report an event.
#[derive(clap::Subcommand)]
pub enum HookSub {
    /// Wait for a message for this claude session and hand it over by
    /// exiting 42 (the hook command makes that claude's `asyncRewake` 2). Registered on SessionStart and
    /// Stop (ov-563). Not for humans.
    Wait {
        /// The runner's mailbox directory; the session's own is under it.
        #[arg(long)]
        mailbox: PathBuf,
    },
}

/// Run the waiter; it never returns.
pub fn run(root: &Path) -> ! {
    let Some(session) = session_of(&read_payload()) else { std::process::exit(0) };
    let Some(mailbox) = Mailbox::of_session(root, &session) else { std::process::exit(0) };
    let ancestors = ancestors_of(std::process::id());
    if ancestors.is_empty() {
        std::process::exit(0);
    }
    match mailbox.wait(std::process::id(), &pid_alive, || mailbox.dir().is_dir() && ancestors.iter().all(|&pid| pid_alive(pid)), POLL) {
        Waited::Message(text) => {
            eprintln!("{}", text.trim_end());
            std::process::exit(WOKE)
        }
        Waited::Another | Waited::Gone => std::process::exit(0),
    }
}

/// The hook payload on stdin, or nothing when claude doesn't finish it in
/// time (a pipe it holds open must not hold the waiter).
fn read_payload() -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    rx.recv_timeout(PAYLOAD_PATIENCE).unwrap_or_default()
}

fn session_of(payload: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(payload).ok()?.get("session_id")?.as_str().map(str::to_string)
}

/// `pid`'s ancestors below init, nearest first, from `ps`.
fn ancestors_of(pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut at = pid;
    while chain.len() < MOST_ANCESTORS {
        let Ok(out) = Command::new("ps").args(["-o", "ppid=", "-p", &at.to_string()]).output() else { break };
        let Some(parent) = String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().ok().filter(|p| *p > 1) else { break };
        chain.push(parent);
        at = parent;
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_comes_from_the_payload() {
        assert_eq!(session_of(br#"{"session_id":"7ecba53c","hook_event_name":"Stop"}"#).as_deref(), Some("7ecba53c"));
        assert_eq!(session_of(b"{}"), None);
        assert_eq!(session_of(b"not json"), None);
        assert_eq!(session_of(b""), None, "a payload that never came");
    }

    #[test]
    fn a_process_knows_its_ancestors() {
        let chain = ancestors_of(std::process::id());
        assert!(!chain.is_empty(), "a test runs under cargo");
        assert!(chain.iter().all(|&pid| pid_alive(pid)));
        assert_eq!(ancestors_of(u32::MAX), Vec::<u32>::new());
    }
}

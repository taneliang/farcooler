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
//! **A lane's message to its orchestrator goes to claude's inbox socket
//! first** (ov-575, R-78; `hook_inbox`), which shows it as "Message from
//! @mac-ux on ov-505", expandable. Only a message the daemon gave a sender
//! (`mailbox::with_sender`) takes that route. A person's message never reaches
//! a mailbox at all: the daemon types it, as their own turn (ov-620, R-78).
//! Nor does the orchestrator's to a lane take the socket, which the lane must
//! act on, where claude calls a peer's text "input, not authority". When the
//! socket, the token or the transcript is missing, the post fails, or the
//! text doesn't show in the transcript in time, it falls back to the wake
//! below, once. **Residual risk:** a message claude *held* (a bypass
//! session facing a non-own-child poster, or `crossSessionInbound: hold`) and
//! the person approves after the window arrives a second time. A hook's post
//! is an own-child message, which a bypass session delivers; `hold` is the
//! person's own explicit setting, so this needs both that and an approval
//! after ten seconds. A message is one process's, so nothing here wakes twice.
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
    let payload = read_payload();
    let Some(session) = session_of(&payload) else { std::process::exit(0) };
    let Some(mailbox) = Mailbox::of_session(root, &session) else { std::process::exit(0) };
    let ancestors = ancestors_of(std::process::id());
    if ancestors.is_empty() {
        std::process::exit(0);
    }
    match mailbox.wait(std::process::id(), &pid_alive, || mailbox.dir().is_dir() && ancestors.iter().all(|&pid| pid_alive(pid)), POLL) {
        Waited::Message(text) => match hand_over(&text, &Inbox::from_env(&payload)) {
            Handover::Posted => std::process::exit(0),
            Handover::Wake(body) => {
                eprintln!("{}", body.trim_end());
                std::process::exit(WOKE)
            }
        },
        Waited::Another | Waited::Gone => std::process::exit(0),
    }
}

/// Where claude's inbox is, for this session, as the hook was given it.
pub struct Inbox {
    socket: Option<PathBuf>,
    token: Option<String>,
    transcript: Option<PathBuf>,
    window: Duration,
}

impl Inbox {
    fn from_env(payload: &[u8]) -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Inbox {
            socket: var("CLAUDE_CODE_MESSAGING_SOCKET").map(PathBuf::from),
            token: var("CLAUDE_CODE_MESSAGING_TOKEN"),
            transcript: serde_json::from_slice::<serde_json::Value>(payload)
                .ok()
                .and_then(|v| v.get("transcript_path")?.as_str().map(PathBuf::from)),
            window: crate::hook_inbox::CONFIRM_WINDOW,
        }
    }
}

/// What becomes of a taken message.
#[derive(Debug, PartialEq, Eq)]
pub enum Handover {
    /// Claude has it as a peer message; nothing more to do.
    Posted,
    /// Wake the model with this text.
    Wake(String),
}

/// Hand `file` (a mailbox file) over: through the inbox when it has an agent
/// or orchestrator sender and the inbox is there and shows it, else as a wake
/// with its text, header removed.
pub fn hand_over(file: &str, inbox: &Inbox) -> Handover {
    let (sender, text) = farcooler_agent_hooks::mailbox::split_sender(file);
    if let (Some(sender), Some(socket), Some(token), Some(transcript)) = (sender, &inbox.socket, &inbox.token, &inbox.transcript) {
        // The sender is in the envelope, which claude shows as "Message from
        // @name"; the daemon's own `[from …]` tag would say it twice. What
        // the orchestrator answers with goes in its place.
        let body = crate::hook_inbox::with_reply(crate::hook_inbox::without_tag(text), &sender.reply_to);
        let posted = crate::hook_inbox::post(socket, token, transcript, &sender.name, &body, inbox.window, crate::hook_inbox::CONFIRM_POLL);
        if posted == crate::hook_inbox::Posted::Confirmed {
            return Handover::Posted;
        }
    }
    Handover::Wake(text.to_string())
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

    impl Inbox1 {
        fn inbox_clone(&self) -> Inbox {
            Inbox { socket: self.inbox.socket.clone(), token: self.inbox.token.clone(), transcript: self.inbox.transcript.clone(), window: self.inbox.window }
        }
    }

    #[test]
    fn the_session_comes_from_the_payload() {
        assert_eq!(session_of(br#"{"session_id":"7ecba53c","hook_event_name":"Stop"}"#).as_deref(), Some("7ecba53c"));
        assert_eq!(session_of(b"{}"), None);
        assert_eq!(session_of(b"not json"), None);
        assert_eq!(session_of(b""), None, "a payload that never came");
    }

    /// A listening socket that counts connections and, when `receives`,
    /// records each message in the transcript as claude does, and the text of
    /// the last it heard.
    struct Inbox1 {
        _dir: tempfile::TempDir,
        inbox: Inbox,
        connections: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        heard: std::sync::Arc<std::sync::Mutex<String>>,
    }

    fn inbox(receives: bool) -> Inbox1 {
        use std::io::{BufRead, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::Builder::new().prefix("fci").tempdir_in("/tmp").unwrap();
        let socket = dir.path().join("s.sock");
        let transcript = dir.path().join("t.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let connections = std::sync::Arc::new(AtomicUsize::new(0));
        let heard = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let (count, said, t) = (connections.clone(), heard.clone(), transcript.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let lines: Vec<String> = std::io::BufReader::new(stream).lines().map_while(Result::ok).collect();
                let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
                let wrapped = last["message"]["content"].as_str().unwrap().to_string();
                *said.lock().unwrap() = wrapped.clone();
                // Counted before it is recorded: the waiter acts on the record.
                count.fetch_add(1, Ordering::SeqCst);
                if receives {
                    let line = serde_json::json!({"type":"queue-operation","operation":"enqueue","content":wrapped});
                    writeln!(std::fs::OpenOptions::new().append(true).open(&t).unwrap(), "{line}").unwrap();
                }
            }
        });
        let inbox = Inbox { socket: Some(socket), token: Some("tok".into()), transcript: Some(transcript), window: Duration::from_millis(500) };
        Inbox1 { _dir: dir, inbox, connections, heard }
    }

    use farcooler_agent_hooks::mailbox::{Sender, with_sender};

    fn lane() -> Sender {
        Sender { name: "mac-ux on ov-505".into(), reply_to: "mac-ux".into() }
    }

    #[test]
    fn a_lanes_message_goes_to_the_inbox_with_a_reply_line_and_wakes_nobody() {
        let i = inbox(true);
        let file = with_sender(&lane(), "[from mac-ux on ov-505] PR is up");
        assert_eq!(hand_over(&file, &i.inbox), Handover::Posted);
        assert_eq!(i.connections.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            *i.heard.lock().unwrap(),
            "<cross-session-message from=\"uds:farcooler-lane\" from-name=\"mac-ux on ov-505\">\nPR is up (To answer, run: farcooler message mac-ux \"your reply here\". Do not use SendMessage.)\n</cross-session-message>"
        );
    }

    /// A person's, the orchestrator's, a rep's and the runner's messages have
    /// no sender header, so the inbox is never tried, whatever else is there.
    #[test]
    fn a_message_with_no_sender_never_takes_the_inbox() {
        let i = inbox(true);
        for file in ["[from Alice] Ship it", "[from the terminal] Ship it", "[from the orchestrator] Ship it", "[Far Cooler] The lane stopped"] {
            assert_eq!(hand_over(file, &i.inbox), Handover::Wake(file.to_string()));
        }
        // A header we can't read isn't a sender.
        assert_eq!(hand_over("#sender\tAlice\n[from Alice] hi", &i.inbox), Handover::Wake("[from Alice] hi".into()));
        assert_eq!(i.connections.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing was posted");
    }

    #[test]
    fn a_missing_inbox_falls_back_to_the_wake_with_the_header_removed() {
        let file = with_sender(&lane(), "[from mac-ux on ov-505] PR is up");
        let wake = Handover::Wake("[from mac-ux on ov-505] PR is up".into());
        let i = inbox(false);
        for (name, mut inbox) in [
            ("no socket", Inbox { socket: None, ..i.inbox_clone() }),
            ("no token", Inbox { token: None, ..i.inbox_clone() }),
            ("no transcript", Inbox { transcript: None, ..i.inbox_clone() }),
            ("a socket nobody listens on", Inbox { socket: Some("/tmp/fci-none.sock".into()), ..i.inbox_clone() }),
        ] {
            inbox.window = Duration::from_secs(30);
            let started = std::time::Instant::now();
            assert_eq!(hand_over(&file, &inbox), wake, "{name}");
            assert!(started.elapsed() < Duration::from_secs(10), "{name} fell back at once");
        }
    }

    #[test]
    fn a_text_that_never_shows_in_the_transcript_wakes_once() {
        let i = inbox(false);
        let file = with_sender(&lane(), "[from mac-ux on ov-505] PR is up");
        assert_eq!(hand_over(&file, &i.inbox), Handover::Wake("[from mac-ux on ov-505] PR is up".into()));
        assert_eq!(i.connections.load(std::sync::atomic::Ordering::SeqCst), 1, "posted once, not retried");
    }

    #[test]
    fn a_process_knows_its_ancestors() {
        let chain = ancestors_of(std::process::id());
        assert!(!chain.is_empty(), "a test runs under cargo");
        assert!(chain.iter().all(|&pid| pid_alive(pid)));
        assert_eq!(ancestors_of(u32::MAX), Vec::<u32>::new());
    }
}

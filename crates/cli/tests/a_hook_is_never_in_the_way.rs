//! The guard the whole design rests on, asserted about the BINARY.
//!
//! The unit tests in `hook.rs` check a private helper that returns a `String`,
//! which is the wrong subject for this one property. A helper cannot be seen to
//! exit non-zero, cannot be seen to write to stderr, and — the reason this file
//! exists — cannot be seen to never return at all. The promise is about the
//! process an agent forks, so it is measured on the process an agent forks.
//!
//! Most cases here assert the same three things, because together they are the
//! whole of "byte-identical to having no hook installed": **exit 0, nothing on
//! stdout, nothing on stderr**, reached inside a hard wall-clock bound.
//!
//! The bound is not decoration. A hook that hangs is worse than one that fails,
//! because a failure is over and a hang is somebody's agent stopped mid-turn
//! with no way to tell why.

use std::path::Path;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// How long a test waits before it calls the hook wedged.
///
/// Deliberately far above the hook's own deadline and far below forever: this
/// is here to tell "bounded" from "never", not to measure the bound. Anything
/// under it passes, so it can be loosened for a slow CI box without weakening
/// what these tests prove.
const WEDGED: Duration = Duration::from_secs(3);

/// Big enough that the frame it becomes overruns a Unix socket's send buffer.
///
/// `net.local.stream.sendspace` is 8192 on macOS. A `PreToolUse` carrying a
/// Write's file content or an Edit's diff clears that routinely, so this is an
/// ordinary payload, not a pathological one.
const OVER_THE_SEND_BUFFER: usize = 64 * 1024;

/// What the agent does with the pipe once it has written the payload.
enum Pipe {
    /// Closed, which is what claude does.
    Closed,
    /// Written and held open. Nothing obliges an agent to close it, codex and
    /// cursor are unmeasured, and a hook that waits on this waits forever.
    HeldOpen,
}

/// A payload of `size` bytes of content: valid JSON, shaped like a real one.
fn a_payload_of(size: usize) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "session_id": "abc",
        "tool_name": "Write",
        "tool_input": { "content": "a".repeat(size) },
    }))
    .expect("a payload")
}

/// Run the real binary the way an agent runs it, under a wall-clock bound.
async fn run_the_hook(
    event: &str,
    gating: bool,
    socket: &Path,
    payload: Vec<u8>,
    pipe: Pipe,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_farcooler"));
    command
        .args(["hook", "--agent", "claude", "--event", event, "--socket"])
        .arg(socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // So a wedged hook is killed when the timeout below drops it, rather
        // than outliving the test run that caught it.
        .kill_on_drop(true);
    if gating {
        command.arg("--gating");
    }
    let mut child = command.spawn().expect("the hook binary runs");

    // Handed over on a task, exactly as a real agent hands it over: the writer
    // must not be what blocks if the hook stops reading.
    let mut stdin = child.stdin.take().expect("a pipe to write the payload on");
    let holding = tokio::spawn(async move {
        let _ = stdin.write_all(&payload).await;
        match pipe {
            Pipe::Closed => None,
            // Returned rather than dropped, so it stays open for as long as
            // this test is watching.
            Pipe::HeldOpen => Some(stdin),
        }
    });

    let started = Instant::now();
    let Ok(finished) = tokio::time::timeout(WEDGED, child.wait_with_output()).await else {
        panic!(
            "the hook was still running after {WEDGED:?} on `{event}`. An agent that \
             forked it is stopped mid-turn, and nothing it can do will free it."
        );
    };
    assert!(
        started.elapsed() < WEDGED,
        "the hook took {:?}, which is the agent sitting still for no reason",
        started.elapsed()
    );
    drop(holding);
    finished.expect("the hook exited")
}

/// Exit 0, nothing on stdout, nothing on stderr. The whole of the promise.
fn it_was_never_in_the_way(out: &Output, what: &str) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "a hook exits 0 on every path; {what} exited {:?}",
        out.status.code()
    );
    assert!(
        out.stdout.is_empty(),
        "a hook with nothing to say prints nothing; {what} printed {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        out.stderr.is_empty(),
        "a hook never explains itself to the agent's terminal; {what} said {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// No daemon at all. If only one test in this repository survives, it is this one.
#[tokio::test]
async fn a_hook_with_no_daemon_behind_it_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("nobody-is-listening.sock");
    let out =
        run_the_hook("MessageDisplay", false, &socket, a_payload_of(64), Pipe::Closed).await;
    it_was_never_in_the_way(&out, "a hook with no socket");
}

/// A socket that is bound but whose backlog nobody drains.
///
/// Not a hypothetical peer: it is a daemon paused on a write of its own,
/// stopped under a debugger, or mid-restart with its socket still bound. And
/// `connect()` SUCCEEDS against it, because AF_UNIX completes into the listen
/// backlog with nothing having accepted — so every guard the hook has is
/// already behind that point when the send buffer fills and the write begins
/// waiting for a writability that never comes.
///
/// It matters most here, on the NON-gating path, because that is
/// `MessageDisplay`: the event that fires on every message, where the hook has
/// nothing to wait for and no reason to be slow.
#[tokio::test]
async fn a_hook_whose_daemon_never_reads_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let out = run_the_hook(
        "MessageDisplay",
        false,
        &socket,
        a_payload_of(OVER_THE_SEND_BUFFER),
        Pipe::Closed,
    )
    .await;
    it_was_never_in_the_way(&out, "a hook whose daemon never reads");
}

#[tokio::test]
async fn a_gating_hook_whose_daemon_never_reads_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let out = run_the_hook(
        "PermissionRequest",
        true,
        &socket,
        a_payload_of(OVER_THE_SEND_BUFFER),
        Pipe::HeldOpen,
    )
    .await;
    it_was_never_in_the_way(&out, "a gating hook whose daemon never reads");
}

/// An agent that writes the payload and never closes the pipe.
#[tokio::test]
async fn a_hook_whose_agent_holds_the_pipe_open_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("nobody-is-listening.sock");
    let out =
        run_the_hook("MessageDisplay", false, &socket, a_payload_of(64), Pipe::HeldOpen).await;
    it_was_never_in_the_way(&out, "a hook whose agent held the pipe open");
}

#[tokio::test]
async fn a_hook_handed_something_that_is_not_json_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let _listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let out =
        run_the_hook("Stop", false, &socket, b"this is not json".to_vec(), Pipe::Closed).await;
    it_was_never_in_the_way(&out, "a hook handed something that is not json");
}

/// An agent name nothing ships. Not an error a hook may report either.
#[tokio::test]
async fn a_hook_told_an_agent_nobody_ships_is_invisible() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .args(["hook", "--agent", "nothing-ships-this", "--event", "Stop", "--socket"])
        .arg(&socket)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("the hook binary runs");
    it_was_never_in_the_way(&out, "a hook told an agent nobody ships");
}

/// The other half: when there IS an answer, it reaches the agent.
///
/// The unit tests end at a `String`. Between that string and the agent lie a
/// write, a flush and a process exit — and stdout is line-buffered while a
/// verdict is one line with no newline on the end, so "the string is right" and
/// "the agent can read it" are two different claims. Only this one is about the
/// binary, and it is the case where being wrong is silent: an agent that reads
/// nothing defers, which looks exactly like a daemon with no opinion.
#[tokio::test]
async fn a_gating_hook_hands_the_verdict_to_the_agent_on_stdout() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut reader = tokio::io::BufReader::new(&mut stream);
        let mut line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
        // Written as bytes rather than built with `encode_line`, so this pins
        // what actually crosses the socket rather than agreeing with our own
        // encoder about it.
        let verdict = br#"{"decision":{"behavior":"deny","message":"Denied from a test"}}"#;
        tokio::io::AsyncWriteExt::write_all(&mut stream, verdict).await.expect("write");
        tokio::io::AsyncWriteExt::write_all(&mut stream, b"\n").await.expect("newline");
    });

    let out = run_the_hook(
        "PermissionRequest",
        true,
        &socket,
        a_payload_of(64),
        Pipe::Closed,
    )
    .await;

    assert_eq!(out.status.code(), Some(0), "a hook exits 0 even when it has something to say");
    assert!(
        out.stderr.is_empty(),
        "and still says nothing to the terminal: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(printed.trim()).unwrap_or_else(|e| panic!("stdout was {printed:?}: {e}"));
    assert_eq!(parsed["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert_eq!(
        parsed["hookSpecificOutput"]["decision"]["message"],
        "Denied from a test",
        "our own words reach the pane verbatim, through the flush and the exit"
    );
}

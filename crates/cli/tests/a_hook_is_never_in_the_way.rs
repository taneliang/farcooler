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
///
/// This is the assertion for a test about TIMELINESS. See `NEVER_FINISHED`,
/// which is the bound for the one test here that is not.
const WEDGED: Duration = Duration::from_secs(3);

/// The same wait, for a test that is about INTEGRITY rather than timeliness.
///
/// Deliberately far beyond anything ordinary parallel test load can reach,
/// because for the test that uses it the clock is not the assertion: it asks
/// whether half a megabyte of verdict survives the exit intact, and the answer
/// is no different at 200 ms than at four seconds. Bounding that tightly buys
/// nothing and costs the thing a guard is for — a red that means "the machine
/// was busy" trains the next reader to disbelieve a red that means "the flush
/// race is back".
///
/// So the two numbers are not a tidy-up waiting to happen. They are two
/// different assertions that happen to be spelled the same way.
const NEVER_FINISHED: Duration = Duration::from_secs(60);

/// The hook's OWN deadline, for a test that is about integrity.
///
/// `NEVER_FINISHED` loosens only the test's clock. The hook carries a clock of
/// its own, `hook::HOOK_DEADLINE`, 400 ms, and when it runs out the hook prints
/// nothing and exits 0 — by design, because the agent then asks at the
/// keyboard. So under that deadline a test that expects a verdict on stdout is
/// also asserting that the fake daemon, the socket and two runtimes all got
/// scheduled inside 400 ms, and a full workspace run is entitled to make that
/// false. It did, on 2026-09-25: exit 0, stdout empty, a red that read as a lost
/// verdict and was the deadline doing its job.
///
/// Passed as the hidden `--deadline-ms`, which no installed hook carries. Under
/// `NEVER_FINISHED`, so a hook that ignores it still ends the test rather than
/// wedging it. What the real deadline does to a late answer is asserted on its
/// own, below, with the real deadline.
const SHAPE_NOT_SPEED: Duration = Duration::from_secs(30);

/// What a hook that really waited out `hook::HOOK_DEADLINE` (400 ms) must
/// have taken at least.
///
/// Below the deadline rather than at it, so the floor proves "it waited for
/// the verdict" without also asserting the timer's precision. A hook that
/// gave up at once, on a connect that failed or a frame it never sent, exits
/// in a few milliseconds and falls far short of this.
const WAITED_OUT_THE_DEADLINE: Duration = Duration::from_millis(300);

/// Long past `hook::HOOK_DEADLINE` and well inside `SHAPE_NOT_SPEED`.
///
/// Measured from the daemon's accept, which is after the hook started its
/// clock, so a hook under the real deadline has always given up by the time
/// this answer is written, however slow the machine is.
const LATE: Duration = Duration::from_secs(1);

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

/// Run the real binary the way an agent runs it, under `WEDGED`.
async fn run_the_hook(
    event: &str,
    gating: bool,
    socket: &Path,
    payload: Vec<u8>,
    pipe: Pipe,
) -> Output {
    run_the_hook_within(WEDGED, None, event, gating, socket, payload, pipe).await
}

/// The same, for a test whose bound is not the thing it is asserting.
///
/// `deadline` is the hook's own, passed as `--deadline-ms`; `None` runs it
/// exactly as an installed hook runs, under `hook::HOOK_DEADLINE`.
async fn run_the_hook_within(
    bound: Duration,
    deadline: Option<Duration>,
    event: &str,
    gating: bool,
    socket: &Path,
    payload: Vec<u8>,
    pipe: Pipe,
) -> Output {
    run_the_hook_timed(bound, deadline, event, gating, socket, payload, pipe).await.0
}

/// The same again, also returning how long the process ran.
///
/// Measured from just after the spawn to the exit. The hook starts its own
/// clock later than that, once it is running, so a hook that waited out its
/// deadline always shows at least the whole deadline here.
async fn run_the_hook_timed(
    bound: Duration,
    deadline: Option<Duration>,
    event: &str,
    gating: bool,
    socket: &Path,
    payload: Vec<u8>,
    pipe: Pipe,
) -> (Output, Duration) {
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
    if let Some(deadline) = deadline {
        command.arg("--deadline-ms").arg(deadline.as_millis().to_string());
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
    let Ok(finished) = tokio::time::timeout(bound, child.wait_with_output()).await else {
        panic!(
            "the hook was still running after {bound:?} on `{event}`. An agent that \
             forked it is stopped mid-turn, and nothing it can do will free it."
        );
    };
    let took = started.elapsed();
    assert!(took < bound, "the hook took {took:?}, which is the agent sitting still for no reason");
    drop(holding);
    (finished.expect("the hook exited"), took)
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
/// write, a flush and a process exit, and only a test of the binary crosses
/// those. Being wrong here is silent — an agent that reads nothing defers,
/// which looks exactly like a daemon with no opinion.
///
/// What this test does NOT do is guard the flush, and the file should say so
/// rather than let its name imply otherwise: with the flush deleted this case
/// still passes about 197 times in 200, because at the size of a real verdict
/// the bytes usually win their race with the exit. The test below is the one
/// that guards it.
///
/// Under `SHAPE_NOT_SPEED` and `NEVER_FINISHED` rather than the real deadline
/// and `WEDGED`, because this is about what arrives, not when: see
/// `SHAPE_NOT_SPEED` for the run it went red in and why that red was false.
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

    let out = run_the_hook_within(
        NEVER_FINISHED,
        Some(SHAPE_NOT_SPEED),
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

/// The verdict is not lost to the exit that follows it.
///
/// tokio's stdout hands the real write to the blocking pool and returns before
/// any of it has left the process; `std::process::exit` does not wait for that
/// pool, and std's own cleanup flush `try_lock`s and skips a pool thread
/// holding the lock. So between writing the verdict and exiting there is a
/// race, and the hook's `flush` is what settles it. At the size a deny message
/// really is, the bytes win that race about 199 times in 200 — which is exactly
/// the shape of a check that cannot fail, and therefore does not check.
///
/// So the daemon here hands back a verdict far larger than a real one, for no
/// reason except that it makes the window certain. Measured on this machine:
/// with the hook's `flush` deleted, 40 of 40 runs lose the verdict at this size
/// and 3 of 200 lose it at a realistic one; with the flush, 0 of 100 and 0 of
/// 200. Nothing about the program is bent to suit the test — the only thing
/// made unusual is the length of a string the daemon chose to send.
///
/// It waits under `NEVER_FINISHED` rather than `WEDGED`, and the difference is
/// deliberate. The tests about a socket nobody reads assert TIMELINESS —
/// taking too long is itself the failure, so a tight bound is the assertion.
/// This one asserts INTEGRITY, and does not care whether the verdict takes
/// 200 ms or four seconds to arrive whole. Bounding it tightly only lets a
/// loaded machine redden it for a reason unrelated to what it tests — and a
/// reader who sees that red will reasonably conclude the flush race is back.
/// It flaked exactly that way once under a full workspace run before the
/// bounds were split.
///
/// Splitting the test's bound was only half of that. The hook's own 400 ms
/// deadline still covered the half megabyte's trip across the socket, and a
/// hook that runs out of it prints nothing — which this test reported as "the
/// exit outran the write". So it also runs under `SHAPE_NOT_SPEED`.
#[tokio::test]
async fn a_big_verdict_is_not_lost_to_the_exit_that_follows_it() {
    /// Comfortably past the point where the blocking pool cannot finish the
    /// write before the exit. Not a plausible deny message, and not pretending
    /// to be one.
    const A_VERDICT_TOO_BIG_TO_RACE: usize = 512 * 1024;

    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let message = "D".repeat(A_VERDICT_TOO_BIG_TO_RACE);
    let sent = message.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut reader = tokio::io::BufReader::new(&mut stream);
        let mut line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
        let verdict = format!(
            "{}\n",
            serde_json::json!({ "decision": { "behavior": "deny", "message": sent } })
        );
        tokio::io::AsyncWriteExt::write_all(&mut stream, verdict.as_bytes())
            .await
            .expect("write");
    });

    // `NEVER_FINISHED`, not `WEDGED`. Half a megabyte through a socket, a pipe
    // and a JSON parse is not fast, and how fast it is proves nothing here.
    let out = run_the_hook_within(
        NEVER_FINISHED,
        Some(SHAPE_NOT_SPEED),
        "PermissionRequest",
        true,
        &socket,
        a_payload_of(64),
        Pipe::Closed,
    )
    .await;

    assert_eq!(out.status.code(), Some(0), "a hook exits 0 even with a lot to say");
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(
        !printed.is_empty(),
        "the verdict never reached the agent: the exit outran the write"
    );
    let parsed: serde_json::Value = serde_json::from_str(printed.trim()).unwrap_or_else(|e| {
        panic!("stdout was {} bytes and did not parse: {e}", out.stdout.len())
    });
    assert_eq!(
        parsed["hookSpecificOutput"]["decision"]["message"]
            .as_str()
            .map(str::len),
        Some(A_VERDICT_TOO_BIG_TO_RACE),
        "the verdict arrived truncated, which is the same race half-lost"
    );
    assert_eq!(parsed["hookSpecificOutput"]["decision"]["message"], message);
}

/// A fake daemon that reads the frame, waits `after`, and answers with a deny.
fn a_daemon_that_denies(socket: &Path, after: Duration) {
    let listener = tokio::net::UnixListener::bind(socket).expect("bind");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut reader = tokio::io::BufReader::new(&mut stream);
        let mut line = String::new();
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
        tokio::time::sleep(after).await;
        let verdict = b"{\"decision\":{\"behavior\":\"deny\",\"message\":\"Denied late\"}}\n";
        // The hook may be long gone; a write into its closed socket is fine.
        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, verdict).await;
    });
}

/// What the agent gets when the verdict misses the real deadline: nothing.
///
/// This is the case the 2026-09-25 red was, made certain instead of left to
/// the machine's load. The design means it (spec, "Permissions, which is the
/// feature": "On timeout the hook returns nothing and the TUI asks as it
/// always would"), and it is why a late deny is safe: claude reads no output
/// from a `PermissionRequest` hook as no decision, and asks the person at the
/// keyboard. It never reads it as an allow. If the hook ever printed an allow,
/// or anything at all, on this path, a daemon's slowness would be deciding.
#[tokio::test]
async fn a_verdict_that_misses_the_deadline_leaves_the_agent_to_ask() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    a_daemon_that_denies(&socket, LATE);
    let (out, took) = run_the_hook_timed(
        WEDGED,
        None,
        "PermissionRequest",
        true,
        &socket,
        a_payload_of(64),
        Pipe::Closed,
    )
    .await;
    it_was_never_in_the_way(&out, "a gating hook whose verdict came after its deadline");
    // Nothing on stdout is also what a hook that never reached the daemon
    // prints. This is what says it got there and waited.
    assert!(
        took >= WAITED_OUT_THE_DEADLINE,
        "the hook gave up after {took:?}, before its deadline could have fired: \
         something other than the deadline ended it"
    );
}

/// The other side of the same daemon: `--deadline-ms` is honored.
///
/// Without this, `SHAPE_NOT_SPEED` could be silently ignored and the two
/// integrity tests above would be back on the 400 ms deadline without anything
/// saying so, green on an idle machine and red on a busy one again.
#[tokio::test]
async fn a_wider_deadline_waits_for_the_same_late_verdict() {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    a_daemon_that_denies(&socket, LATE);
    let out = run_the_hook_within(
        NEVER_FINISHED,
        Some(SHAPE_NOT_SPEED),
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
    assert_eq!(parsed["hookSpecificOutput"]["decision"]["message"], "Denied late");
}

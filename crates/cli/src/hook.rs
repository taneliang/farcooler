//! `farcooler hook <event>` — the one process a live agent session runs.
//!
//! **It is never in the way.** Every failure path here exits 0 and prints
//! nothing: no socket, no daemon, a refused connection, a malformed payload, a
//! deadline. That is not an error-handling style, it is the property that lets
//! the terminal keep its promise — an agent whose Far Cooler is broken must
//! behave exactly as an agent with no Far Cooler at all.
//!
//! Failing is only half of it. **Not returning is the worse failure**, because a
//! failure is over and a hang is somebody's agent stopped mid-turn with nothing
//! it can do about it. So nothing in here is merely unlikely to block: the whole
//! errand runs under one deadline, and every way of getting stuck — a parent
//! that never closes the pipe, a connect, a write into a socket nobody drains,
//! an answer that never comes — ends the same way, at the same moment, with
//! nothing printed.
//!
//! It is a subcommand of the binary that already ships rather than a second
//! executable, so there is nothing extra to build, sign, notarize or install.

use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_agent_hooks::wire::{Decision, HookLine, HookVerdict, decode_line, encode_line};
use farcooler_agent_hooks::Agent;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// The whole of the hook's patience, spent once.
///
/// A name and not a literal at the call site, because it is the knob that
/// trades "the phone got a chance to answer" against "the agent sat still for
/// no reason". 400 ms is where the spec starts it: long enough for a live
/// daemon to say "nobody is watching", short enough that a dead one is
/// imperceptible to the person at the keyboard.
///
/// The spec allows a longer bound — on the order of a minute — but only once a
/// client is attached and has ACCEPTED the ask, and nothing here can be told
/// that: `HookVerdict` is the whole of the daemon's reply and it arrives once,
/// at the end. Widening this unconditionally would mean a daemon that wedges
/// mid-answer wedges the agent for a minute, which is the one thing this file
/// exists to prevent. So the bound stays short until there is a signal to widen
/// it on.
const HOOK_DEADLINE: Duration = Duration::from_millis(400);

pub async fn run(agent: Agent, event: String, socket: PathBuf, gating: bool) {
    // One bound over everything, rather than one per way of getting stuck. A
    // list of blocking calls each with its own guard is only right while
    // somebody keeps adding to the list; a bound around the lot is a property,
    // and it is the property this file exists for.
    let out = tokio::time::timeout(HOOK_DEADLINE, errand(agent, event, socket, gating))
        .await
        .unwrap_or_default();
    if !out.is_empty() {
        let mut stdout = tokio::io::stdout();
        if stdout.write_all(out.as_bytes()).await.is_ok() {
            // Explicit, and honestly: no test can currently make it matter.
            // std's stdout is line-buffered and a verdict is one line with NO
            // trailing newline, so it sits in the buffer until something
            // flushes — but today both routes out of this process do, because
            // `process::exit` runs the same runtime cleanup a normal return
            // from `main` does. Measured, not assumed: removing this line
            // breaks nothing. It stays because the next person to reach for a
            // harder exit — `libc::_exit`, an abort path, a panic hook — gets
            // no warning that the verdict is what they dropped, and an agent
            // that reads nothing defers, which looks exactly like a daemon
            // having had no opinion.
            let _ = stdout.flush().await;
        }
    }
    // The deadline above bounds the WORK. This bounds the PROCESS, and they are
    // not the same thing. `tokio::io::stdin()` reads on the blocking pool, and
    // dropping that read when the deadline fires does not cancel it; the
    // runtime's own shutdown then waits for it to finish. So a parent that
    // writes the payload and holds the pipe open keeps this process alive
    // forever, having already sailed past every guard above — measured, not
    // feared. claude closes the pipe when it has written; codex and cursor are
    // unmeasured, and holding a pipe open is the first thing either of them
    // could do to us.
    //
    // Exiting is also simply what this program means: the errand is done, there
    // is nothing here to tear down, and the one thing that must survive the
    // exit was flushed above.
    std::process::exit(0);
}

/// Take the payload from the agent, and do the errand with it.
///
/// The stdin read is inside the deadline rather than before it because it is a
/// blocking call like any other. claude closes the pipe when it has written the
/// payload; codex and cursor are unmeasured, and a parent that writes and then
/// holds the pipe open would otherwise leave this process alive forever, before
/// it had reached a single one of the guards below.
async fn errand(agent: Agent, event: String, socket: PathBuf, gating: bool) -> String {
    let mut payload = Vec::new();
    if tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::stdin(), &mut payload).await.is_err() {
        return String::new();
    }
    run_with_input(agent, event, socket, gating, payload).await
}

/// The whole of the hook, minus stdin and stdout, so it can be tested.
async fn run_with_input(
    agent: Agent,
    event: String,
    socket: PathBuf,
    gating: bool,
    payload: Vec<u8>,
) -> String {
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&payload) else {
        return String::new();
    };
    let line = HookLine { agent, event, payload };
    let Ok(encoded) = encode_line(&line) else {
        return String::new();
    };
    // The same bound again, over the conversation alone, so this function holds
    // the property on its own terms and a test can say so. `run`'s bound starts
    // first and so still dominates: the process is over inside one
    // `HOOK_DEADLINE`, whichever route it took to get there.
    tokio::time::timeout(HOOK_DEADLINE, converse(&socket, &encoded, gating))
        .await
        .unwrap_or_default()
}

/// Connect, hand over the frame, and — for a gating event — read the answer.
///
/// Every step of this can block against a peer that is merely PRESENT rather
/// than working. `connect` to an AF_UNIX socket succeeds as soon as it lands in
/// the listen backlog, with nothing having accepted it, so a daemon paused on a
/// write of its own, stopped under a debugger, or mid-restart with its socket
/// still bound is indistinguishable from a healthy one until the send buffer
/// fills — 8 KB of it on macOS, which a `PreToolUse` carrying a Write's content
/// clears routinely. The write then waits for a writability that never arrives.
///
/// Hence the deadline around the caller's call to this, and not around the read
/// alone. Being cut off mid-write leaves a partial line on the socket, which is
/// the right outcome: the daemon reads whole lines, and half a frame with no
/// newline is discarded at EOF rather than acted on.
async fn converse(socket: &Path, encoded: &str, gating: bool) -> String {
    let Ok(mut stream) = UnixStream::connect(socket).await else {
        return String::new();
    };
    if stream.write_all(encoded.as_bytes()).await.is_err() {
        return String::new();
    }

    if !gating {
        return String::new();
    }

    let mut reply = String::new();
    let mut reader = BufReader::new(&mut stream);
    if !matches!(reader.read_line(&mut reply).await, Ok(n) if n > 0) {
        return String::new();
    }

    let Ok(verdict) = decode_line::<HookVerdict>(reply.trim()) else {
        return String::new();
    };
    let Some(decision) = verdict.decision else {
        return String::new();
    };
    claude_shaped_output(&decision).unwrap_or_default()
}

/// The decision, in the shape the agent expects to read on stdout.
///
/// Claude's shape, measured against 2.1.263. Codex and cursor are handed the
/// same envelope until each is measured; a shape an agent does not understand
/// is ignored by it, which is the same as deferring.
fn claude_shaped_output(decision: &Decision) -> Option<String> {
    let inner = match decision {
        Decision::Allow => serde_json::json!({ "behavior": "allow" }),
        Decision::Deny { message } => {
            serde_json::json!({ "behavior": "deny", "message": message })
        }
    };
    serde_json::to_string(&serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": inner,
        }
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard the whole design rests on.
    ///
    /// If this ever goes red, a broken Far Cooler can wedge somebody's agent,
    /// and the promise that the terminal is independent of us is void.
    #[tokio::test]
    async fn a_hook_with_no_daemon_behind_it_says_nothing_and_succeeds() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("nobody-is-listening.sock");

        let out = run_with_input(
            Agent::Claude,
            "MessageDisplay".to_string(),
            socket,
            false,
            b"{\"session_id\":\"abc\"}".to_vec(),
        )
        .await;

        assert_eq!(out, "", "a hook with nowhere to send must print nothing at all");
    }

    #[tokio::test]
    async fn a_gating_hook_prints_the_decision_the_daemon_returned() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut line = String::new();
            tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
            let verdict = HookVerdict {
                decision: Some(Decision::Deny { message: "Denied from a test".to_string() }),
            };
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                encode_line(&verdict).expect("encode").as_bytes(),
            )
            .await
            .expect("write");
        });

        let out = run_with_input(
            Agent::Claude,
            "PermissionRequest".to_string(),
            socket,
            true,
            b"{\"session_id\":\"abc\",\"tool_name\":\"Write\"}".to_vec(),
        )
        .await;

        let parsed: serde_json::Value = serde_json::from_str(&out).expect("hook printed json");
        assert_eq!(parsed["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(
            parsed["hookSpecificOutput"]["decision"]["message"],
            "Denied from a test",
            "our own words reach the pane verbatim"
        );
    }

    /// The one answer that can approve something on a person's behalf.
    ///
    /// Every other outcome in this file degrades to deferring, which is safe
    /// because deferring is what an agent with no hook installed already does.
    /// This branch is the exception: a wrong shape here is the difference
    /// between the TUI asking and the TUI being told yes.
    #[tokio::test]
    async fn a_gating_hook_prints_the_allow_the_daemon_returned() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut line = String::new();
            tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
            let verdict = HookVerdict { decision: Some(Decision::Allow) };
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                encode_line(&verdict).expect("encode").as_bytes(),
            )
            .await
            .expect("write");
        });

        let out = run_with_input(
            Agent::Claude,
            "PermissionRequest".to_string(),
            socket,
            true,
            b"{\"session_id\":\"abc\",\"tool_name\":\"Write\"}".to_vec(),
        )
        .await;

        let parsed: serde_json::Value = serde_json::from_str(&out).expect("hook printed json");
        assert_eq!(
            parsed["hookSpecificOutput"]["decision"]["behavior"],
            "allow",
            "an allow must arrive as an allow, under the name the agent reads"
        );
        assert_eq!(
            parsed["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest",
            "claude's envelope, measured against 2.1.263 and asserted for claude only: \
             codex and cursor are sent this same shape unmeasured, and cursor does not \
             even spell its gate this way"
        );
        assert_eq!(
            parsed["hookSpecificOutput"]["decision"]["message"],
            serde_json::Value::Null,
            "an allow carries no words to put in the pane"
        );
    }

    /// A daemon that hangs must not hang an agent.
    #[tokio::test]
    async fn a_gating_hook_whose_daemon_never_answers_defers_to_the_human() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            std::future::pending::<()>().await;
        });

        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_with_input(
                Agent::Claude,
                "PermissionRequest".to_string(),
                socket,
                true,
                b"{}".to_vec(),
            ),
        )
        .await
        .expect("the hook must not outlive its own deadline");

        assert_eq!(out, "", "no answer in time means the TUI asks, as it always would");
    }
}

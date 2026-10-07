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
//! it can do about it. So nothing in here is merely unlikely to block. The first
//! contact (the payload, the connect, the write, the daemon's first word) runs
//! under one deadline, and every way of getting stuck in it — a parent that
//! never closes the pipe, a connect, a write into a socket nobody drains, an
//! answer that never comes — ends the same way, at the same moment, with
//! nothing printed. Only a gating hook whose daemon answered inside that
//! deadline with a hold waits longer, for one more line, and never for more
//! than `LONGEST_HOLD` and its `HOLD_GRACE`; see `HOOK_DEADLINE`.
//!
//! It is a subcommand of the binary that already ships rather than a second
//! executable, so there is nothing extra to build, sign, notarize or install.

use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_agent_hooks::wire::{
    Decision, HOLD_GRACE, HookLine, HookVerdict, LONGEST_HOLD, decode_line, encode_line,
};
use farcooler_agent_hooks::Agent;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::Instant;

/// The first contact's patience: connect, hand over the frame, and — for a
/// gating hook — hear the daemon's first word.
///
/// A name and not a literal at the call site, because it is the knob that
/// trades "the phone got a chance to answer" against "the agent sat still for
/// no reason". 400 ms is where the spec starts it: long enough for a live
/// daemon to say "nobody is watching", short enough that a dead one is
/// imperceptible to the person at the keyboard.
///
/// It is the whole of the wait for every hook but one kind. A gating hook can
/// wait longer, up to `LONGEST_HOLD` (plus `HOLD_GRACE`), and only on the
/// daemon's word: a first line of `{"hold_ms":…}` says the ask is in front of a
/// phone, and the hook then reads one more line for the verdict. That word has to arrive inside
/// this deadline, so the long wait is granted by a daemon that has just shown
/// it is alive and answering, never assumed. A daemon that wedges before its
/// first word costs the agent 400 ms, as it always did; one that wedges mid-hold
/// costs at most the hold. Widening this deadline itself, unconditionally,
/// would mean a daemon that wedges wedges the agent for a minute, which is the
/// one thing this file exists to prevent.
///
/// Missing it, or running out of a hold, loses a decision to the safe side,
/// not the wrong one. The hook prints nothing, and an agent that reads nothing
/// from a `PermissionRequest` hook asks at the keyboard exactly as it would
/// with no hook installed (spec, "Permissions, which is the feature": "On
/// timeout the hook returns nothing and the TUI asks as it always would"). A
/// deny that arrives late becomes a question, never a yes.
pub const HOOK_DEADLINE: Duration = Duration::from_millis(400);

/// The most `--deadline-ms` may ask for: the spec's longest wait, "on the
/// order of a minute". Without a cap, a value near `u64::MAX` would overflow
/// tokio's deadline into its far-future sleep, which is no deadline at all.
pub const LONGEST_DEADLINE_MS: u64 = 60_000;

/// `--deadline-ms`, as the deadline it asks for, capped at
/// `LONGEST_DEADLINE_MS`.
///
/// Clamped rather than refused: refusing is clap exiting 2 with words on
/// stderr, which a hook never does. So no value, however absurd, removes the
/// deadline.
pub fn deadline_from_ms(ms: u64) -> Duration {
    Duration::from_millis(ms.min(LONGEST_DEADLINE_MS))
}

/// A hold's `hold_ms`, as the wait it asks for, capped at `LONGEST_HOLD`.
///
/// Capped for the reason `deadline_from_ms` is: the hold is the daemon's word,
/// and no word from it, however absurd, may remove the hook's bound.
pub fn hold_from_ms(ms: u64) -> Duration {
    Duration::from_millis(ms).min(LONGEST_HOLD)
}

/// The most a hook may take, first contact and hold together.
///
/// Only a gating hook can be held, so only a gating hook gets the hold's room.
fn ceiling(gating: bool, deadline: Duration) -> Duration {
    if gating { deadline + LONGEST_HOLD + HOLD_GRACE } else { deadline }
}

/// `deadline` is `HOOK_DEADLINE` for every hook an agent runs. It is a
/// parameter only so the binary's own tests can take the machine's speed out
/// of a test that is about something else; see `--deadline-ms` in `main.rs`.
pub async fn run(agent: Agent, event: String, socket: PathBuf, gating: bool, deadline: Duration) {
    // One bound over everything, rather than one per way of getting stuck. A
    // list of blocking calls each with its own guard is only right while
    // somebody keeps adding to the list; a bound around the lot is a property,
    // and it is the property this file exists for.
    let bound = ceiling(gating, deadline);
    let out = tokio::time::timeout(bound, errand(agent, event, socket, gating, deadline))
        .await
        .unwrap_or_default();
    if !out.is_empty() {
        let mut stdout = tokio::io::stdout();
        if stdout.write_all(out.as_bytes()).await.is_ok() {
            // Required, for a reason one layer below where it looks. This
            // write does not reach `std::io::Stdout` here at all: tokio's
            // stdout is a `Blocking<std::io::Stdout>`, and its `poll_write`
            // copies the bytes into its own buffer, hands the real write to the
            // blocking pool, and returns `Ready` before anything has left this
            // process (tokio-1.53.1, `src/io/blocking.rs:109-133`).
            // `poll_flush` is what waits for that write to land. The `exit`
            // below does not wait for the blocking pool, and the stdout flush
            // in std's own cleanup `try_lock`s and skips if the pool thread is
            // holding the lock — two independent ways to lose the verdict. And
            // losing it is silent: an agent that reads nothing defers, which is
            // indistinguishable from a daemon that had no opinion.
            //
            // Measured, because measuring too little is how this line was once
            // called unnecessary. Delete it and a short verdict is lost about
            // 3 times in 200 — so a suite that runs the case once stays green
            // and proves nothing. `a_big_verdict_is_not_lost_to_the_exit_that_
            // follows_it` widens the window until the race is certain: 40/40
            // lost without this line, 0/100 with it.
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
///
/// And it is inside the FIRST-CONTACT deadline, not merely inside `run`'s
/// ceiling, because a gating hook's ceiling has a hold's minute of room in it.
/// That room is the daemon's to grant; an agent holding the pipe open must not
/// be able to take it. So the payload, the connect, the write and the first
/// line all share one instant, and only a hold reaches past it.
async fn errand(
    agent: Agent,
    event: String,
    socket: PathBuf,
    gating: bool,
    deadline: Duration,
) -> String {
    let by = Instant::now() + deadline;
    let mut payload = Vec::new();
    let mut stdin = tokio::io::stdin();
    let read = tokio::io::AsyncReadExt::read_to_end(&mut stdin, &mut payload);
    if !matches!(tokio::time::timeout_at(by, read).await, Ok(Ok(_))) {
        return String::new();
    }
    with_input(agent, event, socket, gating, payload, by).await
}

/// The whole of the hook, minus stdin and stdout, so it can be tested.
#[cfg(test)]
async fn run_with_input(
    agent: Agent,
    event: String,
    socket: PathBuf,
    gating: bool,
    payload: Vec<u8>,
    deadline: Duration,
) -> String {
    with_input(agent, event, socket, gating, payload, Instant::now() + deadline).await
}

/// The hook from its payload on, with first contact due `by`.
async fn with_input(
    agent: Agent,
    event: String,
    socket: PathBuf,
    gating: bool,
    payload: Vec<u8>,
    by: Instant,
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
    // first and so still dominates: the process is over inside one ceiling,
    // whichever route it took to get there. `converse` bounds its two parts
    // more tightly still; this is the backstop to its arithmetic.
    let hold = if gating { LONGEST_HOLD + HOLD_GRACE } else { Duration::ZERO };
    tokio::time::timeout_at(by + hold, converse(&socket, &encoded, gating, by))
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
/// Hence `by` around the whole first contact, and not around the read
/// alone. Being cut off mid-write leaves a partial line on the socket, which is
/// the right outcome: the daemon reads whole lines, and half a frame with no
/// newline is discarded at EOF rather than acted on.
///
/// The first contact is all a non-gating hook does, and all a gating one does
/// unless the daemon's first word is a hold. A hold buys one more line, read
/// under the hold's own bound, and that line is the verdict. EOF or an error in
/// the hold is the daemon gone, and ends the wait at once.
async fn converse(socket: &Path, encoded: &str, gating: bool, by: Instant) -> String {
    let first = tokio::time::timeout_at(by, first_contact(socket, encoded, gating)).await;
    let Ok(Some((mut reader, verdict))) = first else {
        return String::new();
    };
    let verdict = match verdict.hold_ms {
        None => verdict,
        Some(ms) => {
            // The grace, because the daemon's clock for this hold started
            // after ours did (`HOLD_GRACE`).
            let rest =
                tokio::time::timeout(hold_from_ms(ms) + HOLD_GRACE, read_verdict(&mut reader)).await;
            let Ok(Some(verdict)) = rest else {
                return String::new();
            };
            verdict
        }
    };
    let Some(decision) = verdict.decision else {
        return String::new();
    };
    claude_shaped_output(&decision).unwrap_or_default()
}

/// Connect, write the frame, and read the daemon's first line.
///
/// `None` for a non-gating hook, which reads nothing, and for every way the
/// conversation can fail. The reader is handed back rather than rebuilt,
/// because it may already hold the line after this one.
async fn first_contact(
    socket: &Path,
    encoded: &str,
    gating: bool,
) -> Option<(BufReader<UnixStream>, HookVerdict)> {
    let mut stream = UnixStream::connect(socket).await.ok()?;
    stream.write_all(encoded.as_bytes()).await.ok()?;
    if !gating {
        return None;
    }
    let mut reader = BufReader::new(stream);
    let verdict = read_verdict(&mut reader).await?;
    Some((reader, verdict))
}

/// One line from the daemon, as a verdict. `None` at EOF, on an error, or on
/// a line that is not one.
async fn read_verdict(reader: &mut BufReader<UnixStream>) -> Option<HookVerdict> {
    let mut line = String::new();
    if !matches!(reader.read_line(&mut line).await, Ok(n) if n > 0) {
        return None;
    }
    decode_line::<HookVerdict>(line.trim()).ok()
}

/// The decision, in the shape the agent expects to read on stdout.
///
/// Claude's shape, measured against 2.1.263. Codex and cursor are handed the
/// same envelope until each is measured; a shape an agent does not understand
/// is ignored by it, which is the same as deferring.
fn claude_shaped_output(decision: &Decision) -> Option<String> {
    let inner = match decision {
        Decision::Allow { updated_input: None } => serde_json::json!({ "behavior": "allow" }),
        // How a question is answered and a plan approved (ov-370): claude
        // takes the tool's input as given, and a plain allow leaves either
        // dialog up.
        Decision::Allow { updated_input: Some(input) } => {
            serde_json::json!({ "behavior": "allow", "updatedInput": input })
        }
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
    use farcooler_agent_hooks::wire::Reply;

    /// The deadline for a test about what the hook PRINTS rather than when.
    ///
    /// Under `HOOK_DEADLINE` these tests assert, without saying so, that a
    /// fake daemon in the same runtime answers inside 400 ms, and a loaded
    /// machine is entitled to make that false. Their binary-level twin in
    /// `tests/a_hook_is_never_in_the_way.rs` went red exactly that way in a
    /// full workspace run on 2026-09-25: the hook deferred as designed, and the
    /// test called the empty answer a lost verdict. The tests here that are
    /// about the bound keep the real one.
    const SHAPE_NOT_SPEED: Duration = Duration::from_secs(30);

    /// A deadline asked for is a deadline given, up to a minute and no more.
    #[test]
    fn a_deadline_is_capped_at_a_minute() {
        assert_eq!(deadline_from_ms(250), Duration::from_millis(250), "under the cap: as asked");
        assert_eq!(deadline_from_ms(1_000_000), Duration::from_secs(60), "over it: the cap");
        assert_eq!(
            deadline_from_ms(u64::MAX),
            Duration::from_secs(60),
            "the value that used to overflow into no deadline at all"
        );
    }

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
            HOOK_DEADLINE,
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
            let verdict =
                Reply::verdict(Some(Decision::Deny { message: "Denied from a test".to_string() }));
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
            SHAPE_NOT_SPEED,
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
            let verdict = Reply::verdict(Some(Decision::allow()));
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
            SHAPE_NOT_SPEED,
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
        assert!(
            parsed["hookSpecificOutput"]["decision"].get("updatedInput").is_none(),
            "a plain allow runs the tool as claude asked: {out}"
        );
    }

    /// An answer to a question, or a plan's approval: claude reads the
    /// answers from `updatedInput`, under that name (claude 2.1.290, ov-370).
    #[test]
    fn an_allow_with_an_input_prints_it_as_updated_input() {
        let input = serde_json::json!({ "questions": [{ "question": "Which?" }], "answers": { "Which?": "Blue" } });
        let out = claude_shaped_output(&Decision::Allow { updated_input: Some(input.clone()) }).expect("printed");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("json");
        assert_eq!(parsed["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(parsed["hookSpecificOutput"]["decision"]["updatedInput"], input, "{out}");
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
                HOOK_DEADLINE,
            ),
        )
        .await
        .expect("the hook must not outlive its own deadline");

        assert_eq!(out, "", "no answer in time means the TUI asks, as it always would");
    }

    /// What a fake daemon does once it has read the hook's frame.
    enum Say {
        Wait(Duration),
        Write(&'static [u8]),
        /// Close the connection, as a daemon that dies does.
        HangUp,
    }

    /// A fake daemon that reads one frame and then follows `script`.
    ///
    /// A script that does not end in `HangUp` keeps the connection open for
    /// as long as the test runs, so a hook that reads past what it was sent
    /// waits rather than meeting an EOF that would let it off.
    fn a_daemon_that(socket: &Path, script: Vec<Say>) {
        let listener = tokio::net::UnixListener::bind(socket).expect("bind");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut line = String::new();
            tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await.expect("read");
            for step in script {
                match step {
                    Say::Wait(d) => tokio::time::sleep(d).await,
                    Say::Write(bytes) => {
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, bytes).await;
                    }
                    Say::HangUp => return,
                }
            }
            std::future::pending::<()>().await;
        });
    }

    const A_HOLD: &[u8] = b"{\"hold_ms\":60000}\n";
    const A_DENY: &[u8] = b"{\"decision\":{\"behavior\":\"deny\",\"message\":\"Denied from a test\"}}\n";

    /// The first-contact deadline for a test that proves the wait widened.
    ///
    /// Not `HOOK_DEADLINE`: the fake daemon's hold must land inside it, and a
    /// loaded machine is entitled to take longer than 400 ms to schedule it
    /// (see `SHAPE_NOT_SPEED`). What the test needs is a verdict that comes
    /// after this deadline and is still printed, and `PAST_FIRST_CONTACT`
    /// is that.
    const FIRST_CONTACT: Duration = Duration::from_secs(1);
    const PAST_FIRST_CONTACT: Duration = Duration::from_millis(1_500);

    async fn ask(socket: PathBuf, gating: bool, deadline: Duration) -> (String, Duration) {
        let started = std::time::Instant::now();
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            run_with_input(
                Agent::Claude,
                "PermissionRequest".to_string(),
                socket,
                gating,
                b"{\"session_id\":\"abc\",\"tool_name\":\"Write\"}".to_vec(),
                deadline,
            ),
        )
        .await
        .expect("the hook must not outlive its own bounds");
        (out, started.elapsed())
    }

    /// The point of a hold: a verdict that comes after the first-contact
    /// deadline still reaches the agent, because the daemon asked for the wait.
    #[tokio::test]
    async fn a_held_hook_prints_the_decision_that_follows_the_hold() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![
            Say::Write(A_HOLD),
            Say::Wait(PAST_FIRST_CONTACT),
            Say::Write(A_DENY),
        ]);
        let (out, _) = ask(socket, true, FIRST_CONTACT).await;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("hook printed {out:?}: {e}"));
        assert_eq!(parsed["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(parsed["hookSpecificOutput"]["decision"]["message"], "Denied from a test");
    }

    /// A hold is bounded by what it says, not by `LONGEST_HOLD` alone.
    #[tokio::test]
    async fn a_held_hook_whose_daemon_goes_quiet_prints_nothing_when_the_hold_ends() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![Say::Write(b"{\"hold_ms\":300}\n")]);
        let (out, took) = ask(socket, true, SHAPE_NOT_SPEED).await;
        assert_eq!(out, "", "a hold that runs out leaves the agent to ask at the keyboard");
        assert!(took < Duration::from_secs(4), "a 300 ms hold (plus its 2 s grace) took {took:?}");
    }

    /// A daemon that goes away mid-hold frees the hook at once, not at the
    /// end of the hold: EOF is an answer.
    #[tokio::test]
    async fn a_held_hook_whose_daemon_hangs_up_prints_nothing() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![Say::Write(A_HOLD), Say::HangUp]);
        let (out, took) = ask(socket, true, SHAPE_NOT_SPEED).await;
        assert_eq!(out, "");
        assert!(took < Duration::from_secs(2), "a hung-up hold took {took:?}");
    }

    /// The first contact stays bounded: a hold that comes late is no hold.
    #[tokio::test]
    async fn a_hold_that_arrives_after_the_first_contact_deadline_is_ignored() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![
            Say::Wait(Duration::from_millis(600)),
            Say::Write(A_HOLD),
            Say::Write(A_DENY),
        ]);
        let (out, took) = ask(socket, true, HOOK_DEADLINE).await;
        assert_eq!(out, "", "a verdict after a late hold must not be printed");
        assert!(took < Duration::from_secs(1), "the hook waited {took:?} past its deadline");
    }

    /// `MessageDisplay` and the rest never read, so a hold cannot slow them.
    ///
    /// The daemon follows its hold with a deny at once, so a hook that read
    /// the hold and waited on it prints the deny: that is the assertion, and
    /// it needs no clock. The time bound is only far below the 60 s hold, so a
    /// loaded machine cannot trip it and a hook that sat out the hold still
    /// does.
    #[tokio::test]
    async fn a_non_gating_hook_never_waits_for_a_hold() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![Say::Write(A_HOLD), Say::Write(A_DENY)]);
        let (out, took) = ask(socket, false, SHAPE_NOT_SPEED).await;
        assert_eq!(out, "", "a non-gating hook read past its frame");
        assert!(took < Duration::from_secs(5), "a non-gating hook took {took:?}");
    }

    /// The daemon's clock for a hold starts after the hook's, so its verdict
    /// can land a little after the hold the hook was told. The hook reads on
    /// for `HOLD_GRACE` past it, so a verdict the daemon wrote at the end of
    /// its hold still reaches the agent instead of an unread socket.
    #[tokio::test]
    async fn a_verdict_written_as_the_hold_ends_still_reaches_the_agent() {
        let dir = tempfile::tempdir().expect("a directory");
        let socket = dir.path().join("h.sock");
        a_daemon_that(&socket, vec![
            Say::Write(b"{\"hold_ms\":300}\n"),
            Say::Wait(Duration::from_millis(600)),
            Say::Write(A_DENY),
        ]);
        let (out, _) = ask(socket, true, SHAPE_NOT_SPEED).await;
        assert!(out.contains("Denied from a test"), "the late verdict was lost: {out:?}");
    }

    #[test]
    fn a_hold_is_capped_at_the_longest_hold() {
        assert_eq!(hold_from_ms(300), Duration::from_millis(300), "under the cap: as asked");
        assert_eq!(hold_from_ms(u64::MAX), Duration::from_secs(60));
    }
}

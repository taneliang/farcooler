//! Proving an ACP adapter works.
//!
//! Moved here from `farcooler-core` when chat mode grew more than one backend.
//! `core::activity::handshake` could not dispatch to a backend crate without
//! inverting the dependency graph — core sits below every backend — so the
//! handshake moved to the backend that performs it and
//! `farcooler_agent::dispatch::handshake` chooses between them.
//!
//! The property that mattered survives the move: the Test button in the
//! runner-settings editor and the test that checks every built-in adapter are
//! still ONE implementation rather than two that agree today.

use farcooler_agent_core::backend::Launch;

/// How long to wait for `initialize` to answer before treating the adapter as
/// wedged, when the caller has no reason to pick a different bound.
///
/// Matches `crates/cli/src/agent_host.rs`'s `AgentSession::start` timeout, which
/// reasons about the identical state under the identical name
/// (`Status::AdapterSilent`): "an adapter that starts but never answers
/// `initialize` is a real state and it looks like nothing at all: the process is
/// alive, the pane is `running`, and the screen stays blank forever." 90s is
/// generous enough that a cold `npx` fetching a package on first use is not
/// killed mid-download.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// What an adapter said when asked to identify itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// The agent and version it reported, when it answered.
    pub reported: String,
}

/// Spawn an adapter, send an ACP `initialize`, and read until it answers.
///
/// This is what makes an adapter form worth having: without it, a wrong launch
/// command is discovered by opening a pane, pressing the chat toggle and getting
/// a blank screen — a failure that lands nowhere near the form that caused it.
///
/// Takes a `Launch` rather than an `AdapterSpec` because the program has to be
/// resolved before it gets here, and resolution needs `farcooler_core::programs`
/// which this crate deliberately does not depend on.
///
/// Three properties worth keeping, each for an observed reason:
///
/// - **A silent adapter fails rather than hanging.** `BufReader::lines().next()`
///   has no timeout of its own, and a process that starts and writes nothing is
///   a real failure mode (`Status::AdapterSilent`). The read happens on its own
///   thread so the bound can be enforced from outside it; killing the child
///   closes the pipe that thread is blocked on and turns the block into an EOF,
///   so it exits on its own.
/// - **Lines that are not the answer are skipped.** Adapters log before they
///   answer, and treating the first line as the response fails on the ones that
///   are chattiest about starting up.
/// - **It runs in a temp directory.** An `initialize` is not scoped to a project
///   and should not be able to touch one.
///
/// What it does NOT prove, and callers must not imply otherwise: that the
/// adapter will be RECOGNIZED. `commands`, `identity`, `blocked` and `working`
/// are matched against agent output, and nothing here exercises them.
pub fn handshake(
    launch: &Launch,
    timeout: std::time::Duration,
) -> std::result::Result<Handshake, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let shown = launch.program.display();

    let mut child = Command::new(&launch.program)
        .args(&launch.args)
        .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": 1, "clientCapabilities": {} }
    });
    let mut stdin = child.stdin.take().expect("piped");
    let sent = writeln!(stdin, "{request}").and_then(|()| stdin.flush());
    if let Err(e) = sent {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("could not talk to `{shown}`: {e}"));
    }

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        let result = loop {
            match lines.next() {
                Some(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    if value.get("id") == Some(&serde_json::json!(1)) {
                        break Ok(value);
                    }
                }
                Some(Err(e)) => break Err(e.to_string()),
                None => break Err("the adapter closed without answering".to_string()),
            }
        };
        // If `recv_timeout` already gave up, the receiver is gone and this send
        // fails. There is nothing left to report to, and the thread's only
        // remaining job is to exit — which it now does.
        let _ = tx.send(result);
    });

    let answer = rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Err("the adapter started and then went silent".to_string())
    });
    let _ = child.kill();
    let _ = child.wait();

    let value = answer?;
    // An `error` member is a well-formed refusal, not a success. Reported as the
    // adapter's own words rather than as "handshake failed", because the message
    // is the only clue about which parameter it disliked.
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("the adapter refused to initialize");
        return Err(message.to_string());
    }
    let result = value
        .get("result")
        .ok_or_else(|| "the adapter answered without a result".to_string())?;

    Ok(Handshake { reported: describe(result) })
}

/// A one-line "who are you" from an `initialize` result.
///
/// ACP puts the agent's name and version under `agentInfo`, but not every
/// adapter fills it in, and one that answered correctly should not be reported
/// as anonymous. So: the name and version when they are there, the protocol
/// version when they are not, and a bare acknowledgement when neither is.
fn describe(result: &serde_json::Value) -> String {
    let info = result.get("agentInfo");
    let name = info.and_then(|i| i.get("name")).and_then(|n| n.as_str());
    let version = info.and_then(|i| i.get("version")).and_then(|v| v.as_str());
    match (name, version) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        (Some(n), None) => n.to_string(),
        _ => match result.get("protocolVersion") {
            Some(p) => format!("answered, ACP protocol {p}"),
            None => "answered".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An adapter that answers `initialize` the way a real one does.
    ///
    /// `/bin/sh` by absolute path rather than by name, because resolution is
    /// the caller's job now — see `Launch`.
    fn answering(body: &str) -> Launch {
        Launch {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), format!("read line; printf '%s\\n' '{body}'")],
            env: Default::default(),
        }
    }

    #[test]
    fn a_handshake_reports_the_agent_it_was_told_about() {
        let launch = answering(
            r#"{"jsonrpc":"2.0","id":1,"result":{"agentInfo":{"name":"My Agent","version":"1.2.3"}}}"#,
        );
        let shake = handshake(&launch, std::time::Duration::from_secs(10)).expect("answered");
        assert_eq!(shake.reported, "My Agent 1.2.3");
    }

    #[test]
    fn an_adapter_that_answers_without_naming_itself_still_succeeds() {
        // Answering correctly is the thing being proven. An adapter that omits
        // agentInfo has still proven its launch command works, and reporting
        // that as a failure would send a user editing a form that was right.
        let launch = answering(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let shake = handshake(&launch, std::time::Duration::from_secs(10)).expect("answered");
        assert_eq!(shake.reported, "answered, ACP protocol 1");
    }

    #[test]
    fn chatter_before_the_answer_is_skipped_rather_than_mistaken_for_it() {
        // Adapters log on startup. Treating the first line as the response
        // fails on exactly the ones that are noisiest about starting.
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "read line; echo 'starting up'; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"agentInfo\":{\"name\":\"Noisy\"}}}'".into(),
            ],
            env: Default::default(),
        };
        assert_eq!(
            handshake(&launch, std::time::Duration::from_secs(10)).expect("answered").reported,
            "Noisy"
        );
    }

    #[test]
    fn a_refusal_is_reported_in_the_adapters_own_words() {
        // The message is the only clue about which parameter it disliked.
        // "handshake failed" would throw away the useful half.
        let launch = answering(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"unsupported protocolVersion"}}"#,
        );
        let failure = handshake(&launch, std::time::Duration::from_secs(10)).expect_err("refused");
        assert_eq!(failure, "unsupported protocolVersion");
    }

    #[test]
    fn an_adapter_that_starts_and_says_nothing_fails_rather_than_hanging() {
        // The observed failure this bound exists for: the process is alive, the
        // pane is running, and the screen stays blank forever.
        let launch = Launch {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            env: Default::default(),
        };
        let failure =
            handshake(&launch, std::time::Duration::from_millis(500)).expect_err("must not hang");
        assert!(failure.contains("silent"), "{failure}");
    }
}

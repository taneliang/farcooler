//! The built-in adapters, checked against the world they depend on.
//!
//! Both tests here exist because of one concrete near-miss: every current
//! search result and third-party document names `@zed-industries/codex-acp`
//! for codex, npm reports it deprecated in favour of
//! `@agentclientprotocol/codex-acp`, and it stalled at 0.16.0 against the live
//! 1.1.9. No amount of unit testing can catch that — only asking the outside
//! world can.

use farcooler_core::activity::Registry;

/// The npm package an adapter runs, if it runs one.
fn npm_package(spec: &farcooler_core::activity::AdapterSpec) -> Option<&str> {
    if spec.program != "npx" {
        return None;
    }
    spec.args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
}

#[test]
fn no_built_in_adapter_is_deprecated_on_npm() {
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        let Some(package) = npm_package(spec) else { continue };

        let out = std::process::Command::new("npm")
            .args(["view", package, "deprecated"])
            .output()
            .expect("npm must be installed to verify the adapters");
        assert!(
            out.status.success(),
            "{} names a package npm cannot resolve: {package}",
            rules.preset
        );
        let notice = String::from_utf8_lossy(&out.stdout);
        assert!(
            notice.trim().is_empty(),
            "{} uses a DEPRECATED package {package}: {}",
            rules.preset,
            notice.trim()
        );
    }
}

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long to wait for `initialize` to answer before treating the adapter as
/// wedged, when the caller has no reason to pick a different bound.
///
/// Matches `crates/cli/src/agent_host.rs`'s `AgentSession::start` timeout,
/// which reasons about the identical state under the identical name
/// (`Status::AdapterSilent`): "an adapter that starts but never answers
/// `initialize` is a real state and it looks like nothing at all: the
/// process is alive, the pane is `running`, and the screen stays blank
/// forever." 90s there is generous enough that a cold `npx` fetching a
/// package on first use is not killed mid-download; this test hits the same
/// npx-fetch cost against the same real packages, so the same bound applies
/// here for the same reason.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(90);

/// Spawn an adapter, send `initialize`, and read until it answers or `timeout`
/// elapses.
///
/// A silent adapter is a real, observed failure mode (see
/// `Status::AdapterSilent` in `agent_host.rs`) and `BufReader::lines().next()`
/// has no timeout of its own: without one, a process that starts and then
/// writes nothing hangs this call forever, and since `cargo test --workspace`
/// runs this in CI on every push, that reads as infrastructure flakiness
/// rather than the broken adapter it actually is. The read happens on its own
/// thread so the timeout can be enforced from outside it; the thread exits on
/// its own once the child is killed below, because killing it closes the pipe
/// the thread is blocked reading from and turns the block into an EOF.
fn initialize(
    spec: &farcooler_core::activity::AdapterSpec,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", spec.program))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": 1, "clientCapabilities": {} }
    });
    let mut stdin = child.stdin.take().expect("piped");
    writeln!(stdin, "{request}").map_err(|e| e.to_string())?;
    stdin.flush().map_err(|e| e.to_string())?;

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(stdout).lines();
        let result = loop {
            match lines.next() {
                // Adapters may log before answering; skip anything that is
                // not the response to id 1.
                Some(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    if value.get("id") == Some(&serde_json::json!(1)) {
                        break Ok(value);
                    }
                }
                Some(Err(e)) => break Err(e.to_string()),
                None => break Err("the adapter closed without answering initialize".to_string()),
            }
        };
        // If `recv_timeout` below already gave up, the receiver is gone and
        // this send fails; there is nothing left to report to, and that is
        // fine — the thread's only remaining job is to exit, which it now does.
        let _ = tx.send(result);
    });

    let answer = rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(format!(
            "no response to `initialize` within {timeout:?} — the adapter started and went silent"
        ))
    });
    let _ = child.kill();
    let _ = child.wait();
    answer
}

#[test]
fn every_built_in_adapter_completes_an_acp_handshake() {
    // A cold `npx` fetches a package on first use, so this is slow the first
    // time and fast afterwards. A missing program is a FAILURE, not a skip: on
    // a machine without the agent installed, silently passing would mean the
    // one test that can catch a broken adapter never runs where it matters.
    let mut failures = Vec::new();
    for rules in Registry::built_in().all() {
        let Some(spec) = &rules.adapter else { continue };
        match initialize(spec, HANDSHAKE_TIMEOUT) {
            Ok(v) if v.get("result").is_some() => {}
            Ok(v) => failures.push(format!("{}: {v}", rules.preset)),
            Err(e) => failures.push(format!("{}: {e}", rules.preset)),
        }
    }
    assert!(
        failures.is_empty(),
        "adapters that could not handshake:\n{}",
        failures.join("\n")
    );
}

#[test]
fn a_silent_adapter_fails_fast_instead_of_hanging_the_suite() {
    // The concrete failure this guards: an adapter that starts and then
    // writes nothing back. `sh -c "sleep 1000"` is exactly that from
    // `initialize`'s point of view — a live process, a stdout that never
    // produces a line. Proven with `sh`/`sleep` rather than a real adapter so
    // this test needs nothing from the network and stays fast: a short
    // explicit bound here, well under `HANDSHAKE_TIMEOUT`, so the suite does
    // not pay the full production timeout just to prove the mechanism works.
    let spec = farcooler_core::activity::AdapterSpec {
        program: "sh".to_string(),
        args: vec!["-c".to_string(), "sleep 1000".to_string()],
        env: Default::default(),
    };
    let result = initialize(&spec, Duration::from_millis(500));
    assert!(
        result.is_err(),
        "a silent adapter must fail the handshake, not hang the test runner"
    );
}

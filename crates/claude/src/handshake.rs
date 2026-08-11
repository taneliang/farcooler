//! Starting the `claude` CLI in stream-json mode and proving it answers.
//!
//! Shapes here come from `vendor/claude-sdk.d.ts` — `SDKControlRequest`,
//! `ControlResponse`, `SDKControlInitializeRequest` — and were then confirmed
//! against a live claude 2.1.226, which is how the two facts below were
//! learned rather than assumed.

use farcooler_agent_core::backend::{BackendError, Launch};

/// The CLI version this crate's understanding of the protocol was confirmed
/// against.
///
/// Not the same number as `vendor/PINNED`'s SDK version: the SDK is versioned
/// `0.3.N` and bundles CLI `2.1.N`, so 0.3.226 and 2.1.226 are the same
/// release. The CLI's number is used here because the CLI is what answers.
pub const PINNED_CLAUDE_VERSION: &str = "2.1.226";

/// The flags that put the CLI on the wire, plus whatever the user added.
///
/// These belong to the backend rather than to config — `args` under
/// `backend = "native"` means EXTRA arguments, appended after these. A user
/// pinning a model must not be able to unset `--output-format` and leave a
/// process nothing can talk to.
///
/// `--verbose` is not optional decoration: the CLI requires it alongside
/// stream-json output.
///
/// `--include-partial-messages` buys the `stream_event` frames that let an
/// answer appear a word at a time instead of landing whole. Without it 2.1.226
/// sends the finished `assistant` message and nothing before it, so a pane sat
/// on Working for the length of the answer and then printed all of it at once,
/// while codex — which streams by default — did not. The SDK spells the same
/// switch `includePartialMessages`.
///
/// `--permission-prompt-tool stdio` is what makes the CLI ASK. Measured against
/// 2.1.226: without it, a tool that needs approval is auto-denied and the agent
/// replies "this session is non-interactive so the prompt can't be answered" —
/// no `can_use_tool` control request is ever sent, so Far Cooler's Allow/Deny
/// row could not appear and the whole permission path was dead code. With it,
/// the CLI sends `can_use_tool` and blocks on the answer, which is the surface
/// `answer_permission` was written for.
pub fn launch_args(extra: &[String]) -> Vec<String> {
    let mut args = vec![
        "--print".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--permission-prompt-tool".to_string(),
        "stdio".to_string(),
    ];
    args.extend_from_slice(extra);
    args
}

/// Variables that must not reach a spawned `claude`.
///
/// The CLI refuses to launch nested inside another Claude Code session and
/// then neither answers nor exits — the documented cause of
/// `Status::AdapterSilent`, and a failure that looks like nothing at all: the
/// process is alive, the pane is running, the screen stays blank.
pub const NESTING_VARS: &[&str] =
    &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"];

/// One `control_request` frame.
pub fn control_request(request_id: &str, subtype: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": subtype },
    })
}

/// The `request_id` a `control_response` is answering, if that is what it is.
///
/// The id is nested under `response`, not at the top level — the frame is
/// `{"type":"control_response","response":{"subtype":"success","request_id":"1",…}}`.
/// Reading it from the top level finds nothing and every response looks
/// unmatched.
pub fn control_response_id(frame: &serde_json::Value) -> Option<String> {
    if frame.get("type").and_then(|t| t.as_str()) != Some("control_response") {
        return None;
    }
    frame
        .get("response")
        .and_then(|r| r.get("request_id"))
        .and_then(|i| i.as_str())
        .map(str::to_string)
}

/// Whether an installed claude is one this build was confirmed against.
///
/// Major and minor only, as for codex: a patch release that did nothing to the
/// wire must not cost a user chat mode.
pub fn check_version(found: &str, expected: &str) -> Result<(), BackendError> {
    let series = |v: &str| {
        let mut parts = v.split('.');
        let major = parts.next().unwrap_or_default().to_string();
        let minor = parts.next().unwrap_or_default().to_string();
        (major, minor)
    };
    if series(found) == series(expected) {
        return Ok(());
    }
    Err(BackendError::Incompatible { found: found.to_string(), expected: expected.to_string() })
}

/// Start `claude` in stream-json mode, initialize, and ask what it is.
///
/// Two requests rather than one, because the `initialize` response carries no
/// version — confirmed against 2.1.226, whose result holds `commands`,
/// `models`, `account` and a dozen other keys but nothing identifying the
/// binary. `get_binary_version` answers `{"buildTime":…,"version":"2.1.226"}`
/// in the same session, which is cheaper than a second process.
pub fn handshake(launch: &Launch, timeout: std::time::Duration) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let shown = launch.program.display();
    let mut command = Command::new(&launch.program);
    command
        .args(launch_args(&launch.args))
        .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for var in NESTING_VARS {
        command.env_remove(var);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;

    let mut stdin = child.stdin.take().expect("piped");
    let sent = writeln!(stdin, "{}", control_request("1", "initialize"))
        .and_then(|()| writeln!(stdin, "{}", control_request("2", "get_binary_version")))
        .and_then(|()| stdin.flush());
    if let Err(e) = sent {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("could not talk to `{shown}`: {e}"));
    }

    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut initialized = false;
        let mut lines = BufReader::new(stdout).lines();
        let result = loop {
            match lines.next() {
                Some(Ok(line)) => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                        continue;
                    };
                    // Skipping is required, not defensive: 2.1.226 emitted six
                    // `system` frames — the user's own SessionStart hooks —
                    // before the first control_response arrived.
                    let Some(id) = control_response_id(&value) else { continue };
                    let response = value.get("response").cloned().unwrap_or_default();
                    if response.get("subtype").and_then(|s| s.as_str()) == Some("error") {
                        let message = response
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("claude refused to initialize");
                        break Err(message.to_string());
                    }
                    match id.as_str() {
                        "1" => initialized = true,
                        "2" => {
                            let version = response
                                .get("response")
                                .and_then(|r| r.get("version"))
                                .and_then(|v| v.as_str())
                                .map(str::to_string);
                            break match version {
                                Some(v) if initialized => Ok(v),
                                // Answering the version without having answered
                                // initialize would mean the control channel
                                // works but the session does not, which is not
                                // a handshake worth reporting as a success.
                                Some(_) => Err("claude answered its version but not initialize"
                                    .to_string()),
                                None => Err("claude answered without a version".to_string()),
                            };
                        }
                        _ => continue,
                    }
                }
                Some(Err(e)) => break Err(e.to_string()),
                None => break Err("claude closed without answering".to_string()),
            }
        };
        let _ = tx.send(result);
    });

    let answer = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err("claude started and then went silent".to_string()));
    let _ = child.kill();
    let _ = child.wait();

    let version = answer?;
    check_version(&version, PINNED_CLAUDE_VERSION).map_err(|e| e.to_string())?;
    Ok(format!("claude stream-json {version}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_flags_are_the_ones_the_sdk_uses() {
        // stream-json in BOTH directions, and --verbose, which the CLI requires
        // alongside stream-json output rather than merely tolerating.
        let args = launch_args(&[]);
        assert!(args.contains(&"--print".to_string()), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--input-format", "stream-json"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--output-format", "stream-json"]), "{args:?}");
        assert!(args.contains(&"--verbose".to_string()), "{args:?}");
    }

    #[test]
    fn partial_messages_are_asked_for_so_an_answer_streams() {
        // Without this, 2.1.226 sends the finished `assistant` message and
        // nothing before it: the pane sits on Working for the whole answer and
        // then prints it in one go, while codex streams.
        assert!(launch_args(&[]).contains(&"--include-partial-messages".to_string()));
    }

    #[test]
    fn the_cli_is_told_to_ask_rather_than_auto_deny() {
        // Measured against 2.1.226: without `--permission-prompt-tool stdio` a
        // tool needing approval is auto-denied and the agent says the session
        // is non-interactive. No `can_use_tool` request is sent at all, so the
        // Allow/Deny row never appeared and every permission answer in this
        // crate was unreachable.
        let args = launch_args(&[]);
        assert!(args.windows(2).any(|w| w == ["--permission-prompt-tool", "stdio"]), "{args:?}");
    }

    #[test]
    fn extra_args_are_appended_after_the_protocol_flags() {
        // The config field means "extra", not "instead of". A user pinning a
        // model must not be able to unset --output-format and break the wire.
        let args = launch_args(&["--model".into(), "opus".into()]);
        assert_eq!(&args[args.len() - 2..], ["--model".to_string(), "opus".to_string()]);
        assert!(args.windows(2).any(|w| w == ["--output-format", "stream-json"]));
    }

    #[test]
    fn a_control_response_is_recognized_by_its_nested_request_id() {
        // The id is under `response`, not at the top level. Reading it from the
        // top finds nothing and every response looks unmatched — the exact
        // shape 2.1.226 sends.
        let frame = serde_json::json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "1" }
        });
        assert_eq!(control_response_id(&frame).as_deref(), Some("1"));
    }

    #[test]
    fn a_system_frame_is_not_mistaken_for_a_response() {
        // 2.1.226 emitted six of these — the user's own SessionStart hooks —
        // before the first control_response.
        let hook = serde_json::json!({ "type": "system", "subtype": "hook_started" });
        assert_eq!(control_response_id(&hook), None);
        assert_eq!(control_response_id(&serde_json::json!({ "type": "assistant" })), None);
    }

    #[test]
    fn the_nesting_variables_are_scrubbed_because_the_cli_hangs_on_them() {
        // Not a tidiness measure: nested inside another Claude Code session the
        // CLI neither answers nor exits, which is the documented cause of
        // Status::AdapterSilent.
        assert!(NESTING_VARS.contains(&"CLAUDECODE"));
    }

    #[test]
    fn a_version_outside_the_pin_is_incompatible_rather_than_a_guess() {
        assert!(matches!(
            check_version("2.3.0", "2.1.226"),
            Err(BackendError::Incompatible { .. })
        ));
        assert!(check_version("2.1.226", "2.1.226").is_ok());
    }

    #[test]
    fn a_patch_release_of_the_same_series_is_still_compatible() {
        // The CLI ships patch releases constantly. Refusing them would mean
        // chat mode broke roughly daily for a wire that had not moved.
        assert!(check_version("2.1.300", "2.1.226").is_ok());
        assert!(check_version("2.2.0", "2.1.226").is_err());
    }
}

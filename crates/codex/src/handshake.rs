//! Starting `codex app-server` and proving it answers.
//!
//! Every shape here was read off codex-cli 0.146.0 rather than taken from
//! documentation, because two of them contradict what the documentation
//! implies. See `initialize_request` and `version_of`.

use farcooler_agent_core::backend::{BackendError, Launch};

/// The version this crate's understanding of the protocol was generated
/// against.
///
/// Read out of `vendor/PINNED` at compile time rather than written here as a
/// literal. It was a literal for one afternoon and immediately went stale
/// against the file it was supposed to mirror — two records of one fact, which
/// is the drift `scripts/regen-backend-types.sh` exists to prevent.
pub const PINNED_CODEX_VERSION: &str = pinned_codex_version();

const fn pinned_codex_version() -> &'static str {
    let pinned = include_str!("../../../vendor/PINNED");
    // `const fn` cannot iterate lines, so the file's shape is the contract:
    // `codex-cli <version>` first, one field per line. `regen-backend-types.sh`
    // writes it and this reads it; if either changes, both change.
    let bytes = pinned.as_bytes();
    let prefix = b"codex-cli ";
    let mut start = 0;
    while start < prefix.len() {
        assert!(bytes[start] == prefix[start], "vendor/PINNED must start with `codex-cli `");
        start += 1;
    }
    let mut end = start;
    while end < bytes.len() && bytes[end] != b'\n' {
        end += 1;
    }
    // SAFETY-equivalent: the slice is inside a `&str` and split on ASCII
    // boundaries, so it is still valid UTF-8.
    match std::str::from_utf8(bytes.split_at(end).0.split_at(start).1) {
        Ok(version) => version,
        Err(_) => panic!("vendor/PINNED is not valid UTF-8"),
    }
}

/// The subcommand that turns the codex binary into an app-server.
///
/// Owned by this backend rather than by config: it is how the backend talks,
/// not a preference. A user's `args` are appended AFTER it — see the doc on
/// `AdapterSpec::args`.
pub fn launch_args(extra: &[String]) -> Vec<String> {
    let mut args = vec!["app-server".to_string()];
    args.extend_from_slice(extra);
    args
}

/// The `initialize` frame, as codex actually wants it.
///
/// **No `jsonrpc` member.** The app-server is JSON-RPC 2.0 in every other
/// respect but does not put that field on the wire, and neither does it send
/// one back — verified against 0.146.0, whose reply is exactly
/// `{"id":1,"result":{...}}`. Sending one was not tested to be harmless, so
/// this sends what the server itself sends.
pub fn initialize_request(id: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "method": "initialize",
        "params": initialize_params(),
    })
}

/// The `initialize` params on their own, for a caller that frames its own
/// requests. Shared so the handshake and a real session cannot introduce
/// themselves differently.
pub fn initialize_params() -> serde_json::Value {
    serde_json::json!({
        "clientInfo": {
            "name": "farcooler",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

/// The codex version out of an `initialize` result's `userAgent`.
///
/// The real string is
/// `farcooler/0.146.0 (Mac OS 26.5.2; arm64) kitty/0.42.0 (farcooler; 0.1.0)` —
/// note that the leading product name is the `clientInfo.name` WE supplied,
/// echoed back, so this cannot key on the word "codex". The version is the
/// token after the first `/`.
pub fn version_of(user_agent: &str) -> Option<String> {
    let after = user_agent.split_once('/')?.1;
    let version = after.split_whitespace().next()?;
    if version.is_empty() { None } else { Some(version.to_string()) }
}

/// Whether an installed codex is one this build can talk to.
///
/// **Major only**, and that is a correction rather than laxness. This compared
/// major AND minor for exactly one afternoon, during which codex went from
/// 0.146.0 to 0.147.0 and chat mode refused to start on a protocol that had not
/// visibly changed. A guard that fires on an ordinary release is not protecting
/// anyone; it is an outage on a schedule somebody else controls.
///
/// What actually protects the transcript is that the normalizer is lenient by
/// design: a frame this build does not know becomes a visible `Gap` rather than
/// a broken session, and `tests/turn_fixture.rs` asserts a real turn produces
/// none. So a minor bump degrades honestly instead of failing shut, and a major
/// bump — the one that means "this is a different protocol" — still refuses.
pub fn check_version(found: &str, expected: &str) -> Result<(), BackendError> {
    let major = |v: &str| v.split('.').next().unwrap_or_default().to_string();
    if major(found) == major(expected) {
        return Ok(());
    }
    Err(BackendError::Incompatible { found: found.to_string(), expected: expected.to_string() })
}

/// Start `codex app-server`, send `initialize`, and read until it answers.
///
/// Blocking, like the ACP handshake it sits beside, and for the same reason:
/// the caller runs it on a blocking pool because the bound is generous enough
/// that holding a runtime worker would be wrong.
pub fn handshake(launch: &Launch, timeout: std::time::Duration) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let shown = launch.program.display();
    let mut child = Command::new(&launch.program)
        .args(launch_args(&launch.args))
        .envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;

    let mut stdin = child.stdin.take().expect("piped");
    let request = initialize_request(1);
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
                    // Skipping non-matching lines is required, not defensive:
                    // 0.146.0 emits `remoteControl/status/changed` unprompted,
                    // and a reader that took the first line would read that.
                    if value.get("id") == Some(&serde_json::json!(1)) {
                        break Ok(value);
                    }
                }
                Some(Err(e)) => break Err(e.to_string()),
                None => break Err("codex app-server closed without answering".to_string()),
            }
        };
        let _ = tx.send(result);
    });

    let answer = rx
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err("codex app-server started and then went silent".to_string()));
    let _ = child.kill();
    let _ = child.wait();

    let value = answer?;
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("codex app-server refused to initialize");
        return Err(message.to_string());
    }
    let result = value
        .get("result")
        .ok_or_else(|| "codex app-server answered without a result".to_string())?;
    let user_agent = result
        .get("userAgent")
        .and_then(|u| u.as_str())
        .ok_or_else(|| "codex app-server answered without a userAgent".to_string())?;

    let version = version_of(user_agent)
        .ok_or_else(|| format!("could not read a version out of `{user_agent}`"))?;
    check_version(&version, PINNED_CODEX_VERSION).map_err(|e| e.to_string())?;

    Ok(format!("codex app-server {version}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initialize_request_omits_the_jsonrpc_member() {
        // Verified against codex-cli 0.146.0, whose own reply carries no
        // `jsonrpc` either. Sending one was never tested to be harmless.
        let frame = initialize_request(1);
        assert!(frame.get("jsonrpc").is_none(), "{frame}");
        assert_eq!(frame["method"], "initialize");
        assert_eq!(frame["params"]["clientInfo"]["name"], "farcooler");
        assert!(frame["params"]["clientInfo"]["version"].is_string());
    }

    #[test]
    fn app_server_is_the_backends_flag_and_user_args_come_after_it() {
        // The config field means "extra", not "instead of". A user pinning a
        // model must not be able to drop the subcommand that makes this a
        // protocol rather than an interactive session.
        assert_eq!(launch_args(&[]), vec!["app-server".to_string()]);
        assert_eq!(
            launch_args(&["-c".to_string(), "model=o3".to_string()]),
            vec!["app-server".to_string(), "-c".to_string(), "model=o3".to_string()]
        );
    }

    #[test]
    fn the_version_comes_after_the_first_slash_not_from_the_word_codex() {
        // The exact string 0.146.0 returned. The leading product name is the
        // clientInfo.name WE sent, echoed back — so anything keying on "codex"
        // would read the wrong token, or none.
        let real = "farcooler/0.146.0 (Mac OS 26.5.2; arm64) kitty/0.42.0 (farcooler; 0.1.0)";
        assert_eq!(version_of(real).as_deref(), Some("0.146.0"));
        assert_eq!(version_of("no-slash-here").as_deref(), None);
    }

    #[test]
    fn a_different_major_is_incompatible_rather_than_a_guess() {
        assert!(matches!(
            check_version("1.0.0", "0.147.0"),
            Err(BackendError::Incompatible { .. })
        ));
        assert!(check_version("0.147.0", "0.147.0").is_ok());
    }

    #[test]
    fn an_ordinary_release_does_not_take_chat_mode_down() {
        // The regression this exists for, observed rather than imagined: this
        // check compared minor versions too, codex shipped 0.146.0 -> 0.147.0
        // inside a day, and a working agent was refused on a protocol that had
        // not visibly changed. A guard that fires on a routine release is an
        // outage on somebody else's release schedule.
        assert!(check_version("0.147.0", "0.146.0").is_ok(), "a minor bump must not fail shut");
        assert!(check_version("0.146.7", "0.146.0").is_ok());
        assert!(check_version("0.200.0", "0.146.0").is_ok());
    }

    #[test]
    fn the_pin_is_read_from_the_file_that_records_it() {
        // One record of one fact. A literal here went stale against
        // vendor/PINNED within an afternoon.
        assert_eq!(PINNED_CODEX_VERSION, include_str!("../../../vendor/PINNED")
            .lines()
            .next()
            .unwrap()
            .trim_start_matches("codex-cli ")
            .trim());
    }

    #[test]
    fn an_incompatible_version_names_both_sides() {
        let e = check_version("2.0.0", "0.146.0").expect_err("incompatible");
        let text = e.to_string();
        assert!(text.contains("2.0.0"), "names what is installed: {text}");
        assert!(text.contains("0.146.0"), "and what this build expects: {text}");
    }
}

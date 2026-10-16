//! The real `farcooler rep-gate`, run as Claude Code runs it, admits a
//! connector's tool only when the file `--connectors` names lists it
//! (ov-586), and refuses every MCP tool without that file.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// The gate's exit code for a call to `tool`, given `connectors` or none.
fn gate(checkout: &Path, tool: &str, connectors: Option<&Path>) -> i32 {
    let cli = env!("CARGO_BIN_EXE_farcooler");
    let mut cmd = Command::new(cli);
    cmd.args(["rep-gate", "--checkout"]).arg(checkout).args(["--cli", cli, "--no-git"]);
    if let Some(file) = connectors {
        cmd.arg("--connectors").arg(file);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let call = serde_json::json!({ "tool_name": tool, "tool_input": { "channel": "general" } });
    child.stdin.take().unwrap().write_all(call.to_string().as_bytes()).unwrap();
    child.wait().unwrap().code().unwrap()
}

#[test]
fn only_a_listed_connector_passes_the_real_gate() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("rep-1-connectors.json");
    std::fs::write(&file, r#"{"connectors":["Slack"]}"#).unwrap();
    assert_eq!(gate(dir.path(), "mcp__claude_ai_Slack__send_message", Some(&file)), 0);
    assert_eq!(gate(dir.path(), "mcp__claude_ai_Linear__create_issue", Some(&file)), 2);
    assert_eq!(gate(dir.path(), "mcp__claude_ai_Slack__send_message", None), 2, "no file, no connector");
    std::fs::write(&file, r#"{"connectors":[]}"#).unwrap();
    assert_eq!(gate(dir.path(), "mcp__claude_ai_Slack__send_message", Some(&file)), 2, "turned off, refused at once");
}

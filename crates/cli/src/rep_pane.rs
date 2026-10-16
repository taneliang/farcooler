//! This CLI in a rep's pane (ov-527): the gate hook a rep's Claude Code runs
//! before every tool call, and the routing a rep's pane is refused.
//!
//! The gate's decision is the runner's (`farcooler_daemon::rep_gate`); this
//! is only its door. The `--runner` refusal is defense in depth: the gate
//! already refuses the flag, and the runner's allow-list doesn't depend on
//! either.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use farcooler_daemon::rep_gate::{Gate, Verdict, decide};

/// The most a hook's input may be. A tool call's input is far smaller; past
/// this it's refused rather than read.
const LONGEST_INPUT: u64 = 4 * 1024 * 1024;

/// Claude Code's blocking exit for a `PreToolUse` hook: the call is refused
/// before any permission rule is read.
pub(crate) const BLOCK: i32 = 2;

/// `farcooler rep-gate`: read one `PreToolUse` input on stdin and answer it.
/// An allowed call prints Claude Code's `allow` and exits 0. Anything else,
/// including input that can't be read, prints the reason on stderr and
/// exits `BLOCK`.
pub(crate) fn gate(checkout: &Path, cli: &str, git: Option<&Path>, no_git: bool, connectors: Option<&Path>) -> i32 {
    // A git named but not usable is a gate that can't do its job: refused, not
    // left to find one through the pane's `PATH`.
    if git.is_some_and(|git| !farcooler_daemon::git::pin_git(git.as_os_str())) {
        eprintln!("The rep gate couldn't use the runner's git.");
        return BLOCK;
    }
    let mut text = String::new();
    if std::io::stdin().take(LONGEST_INPUT).read_to_string(&mut text).is_err() {
        eprintln!("The rep gate couldn't read the tool call.");
        return BLOCK;
    }
    let tracked = |within: &Path| {
        if no_git { None } else { farcooler_daemon::repo_grep::tracked_blocking(checkout, within) }
    };
    let connectors = connectors.map(read_connectors).unwrap_or_default();
    let (code, out, err) =
        answer(&text, checkout, cli, &|p| std::fs::canonicalize(p).ok(), &|p| p.is_file(), &tracked, &connectors);
    if !out.is_empty() {
        println!("{out}");
    }
    if !err.is_empty() {
        eprintln!("{err}");
    }
    code
}

/// The gate's answer to `input`: the exit code, stdout and stderr.
pub(crate) fn answer(
    input: &str,
    checkout: &Path,
    cli: &str,
    resolve: &dyn Fn(&Path) -> Option<PathBuf>,
    is_file: &dyn Fn(&Path) -> bool,
    tracked: &dyn Fn(&Path) -> Option<bool>,
    connectors: &[String],
) -> (i32, String, String) {
    let Ok(call) = serde_json::from_str::<serde_json::Value>(input) else {
        return (BLOCK, String::new(), "The rep gate couldn't read the tool call.".into());
    };
    let Some(tool) = call.get("tool_name").and_then(serde_json::Value::as_str) else {
        return (BLOCK, String::new(), "The rep gate was given no tool.".into());
    };
    let empty = serde_json::Value::Null;
    let input = call.get("tool_input").unwrap_or(&empty);
    match decide(&Gate { checkout, cli, is_file, tracked, connectors }, tool, input, resolve) {
        Verdict::Allow => {
            let allow = serde_json::json!({
                "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "allow" }
            });
            (0, allow.to_string(), String::new())
        }
        Verdict::Deny(why) => (BLOCK, String::new(), why.to_string()),
    }
}

/// The connectors the runner's file names (`reps::write_connectors`), or
/// none when it can't be read, is too long or isn't what the runner writes:
/// a gate that can't tell which connectors are on refuses them all.
pub(crate) fn read_connectors(path: &Path) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    let mut text = String::new();
    if file.take(farcooler_daemon::reps::CONNECTORS_FILE_MAX).read_to_string(&mut text).is_err() {
        return Vec::new();
    }
    serde_json::from_str::<farcooler_daemon::reps::ConnectorsFile>(&text).map(|f| f.connectors).unwrap_or_default()
}

/// Refuse `--runner` in a rep's pane (`FARCOOLER_REP` set): a rep works only
/// on the runner it's on.
pub(crate) fn refuse_routing(runner: Option<&str>, rep: Option<&str>) -> Result<(), String> {
    match (runner, rep.filter(|r| !r.trim().is_empty())) {
        (Some(_), Some(_)) => Err("a rep works only on the runner it's on: leave out --runner".into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(p: &Path) -> Option<PathBuf> {
        ["/w/repo", "/w/repo/a.md", "/etc/passwd"].contains(&p.to_str()?).then(|| p.to_path_buf())
    }

    /// An allowed call answers `allow` and exits 0; a refused one, and input
    /// that can't be read, exit with Claude Code's blocking code and say why.
    #[test]
    fn the_gate_allows_with_json_and_refuses_with_a_blocking_exit() {
        let ask = |tool: &str, input: serde_json::Value| {
            serde_json::json!({ "tool_name": tool, "tool_input": input, "hook_event_name": "PreToolUse" }).to_string()
        };
        let (code, out, _) = answer(&ask("Bash", serde_json::json!({ "command": "/x/farcooler plan" })), Path::new("/w/repo"), "/x/farcooler", &resolve, &|_| true, &|_| Some(true), &[]);
        assert_eq!(code, 0);
        assert!(out.contains(r#""permissionDecision":"allow""#), "{out}");
        let (code, _, _) = answer(&ask("Read", serde_json::json!({ "file_path": "/w/repo/a.md" })), Path::new("/w/repo"), "/x/farcooler", &resolve, &|_| true, &|_| Some(true), &[]);
        assert_eq!(code, 0);
        for refused in [
            ask("Bash", serde_json::json!({ "command": "/x/farcooler plan; true" })),
            ask("Read", serde_json::json!({ "file_path": "/etc/passwd" })),
            ask("Write", serde_json::json!({ "file_path": "/w/repo/a.md" })),
            "not json".to_string(),
            "{}".to_string(),
        ] {
            let (code, out, err) = answer(&refused, Path::new("/w/repo"), "/x/farcooler", &resolve, &|_| true, &|_| Some(true), &[]);
            assert_eq!((code, out.as_str()), (BLOCK, ""), "{refused}");
            assert!(!err.is_empty(), "a refusal says why: {refused}");
        }
    }

    /// A runner that answers the reads `task show` and `task list` make, and
    /// records them.
    struct Reads(Vec<farcooler_protocol::v1::Request>);

    impl crate::tasks::DispatchLink for Reads {
        fn capabilities(&self) -> Vec<String> {
            ["workstreams", "tasks"].map(String::from).to_vec()
        }
        async fn call(
            &mut self,
            req: farcooler_protocol::v1::Request,
        ) -> Result<farcooler_protocol::v1::Result, farcooler_transport::ClientError> {
            use farcooler_protocol::v1::{self as pb, result::Value};
            let id = crate::id_bytes(uuid::Uuid::from_u128(7));
            let task = pb::Task { id: id.clone(), repository_id: id.clone(), workspace_id: id.clone(), key: "ov-1".into(), ..Default::default() };
            let value = match req.method.as_str() {
                "repository.list" => Value::RepositoryList(pb::RepositoryList { items: vec![pb::Repository { id: id.clone(), ..Default::default() }] }),
                "workspace.list" => Value::WorkspaceList(pb::WorkspaceList {
                    items: vec![pb::Workspace { id: id.clone(), repository_id: id.clone(), is_main: true, ..Default::default() }],
                }),
                "task.get_by_key" | "task.list" => Value::TaskList(pb::TaskList { items: vec![task], reads: None }),
                other => panic!("{other} isn't a read this fake answers"),
            };
            self.0.push(req);
            Ok(pb::Result { value: Some(value) })
        }
        async fn pause(&mut self, _wait: std::time::Duration) {}
    }

    /// Every call `task show` and `task list` make is one a rep's pane may
    /// make (`people::rep_may`): finding the card and listing the board, and
    /// `task.get`, which `task show` reads the card with.
    #[tokio::test]
    async fn task_show_and_list_call_only_what_a_rep_may() {
        let mut link = Reads(Vec::new());
        crate::tasks::find_task(&mut link, None, "ov-1").await.expect("found");
        let own = uuid::Uuid::from_u128(7).to_string();
        let board = crate::tasks::board_for(&mut link, None, None, Some(own)).await.expect("a board");
        crate::tasks::listing(&mut link, &board, None, None, false).await.expect("a listing");
        let methods: Vec<&str> = link.0.iter().map(|r| r.method.as_str()).chain(["task.get"]).collect();
        assert!(methods.contains(&"task.list") && methods.contains(&"task.get_by_key"), "{methods:?}");
        for method in methods {
            let req = farcooler_protocol::v1::Request { method: method.into(), ..Default::default() };
            assert!(farcooler_daemon::people::rep_may(&req), "{method} isn't a rep's to call");
        }
    }

    /// `--runner` is refused in a rep's pane, and only there.
    #[test]
    fn a_reps_pane_cannot_route_to_another_runner() {
        assert!(refuse_routing(Some("bob@box"), Some("alice")).is_err());
        assert!(refuse_routing(None, Some("alice")).is_ok());
        assert!(refuse_routing(Some("bob@box"), None).is_ok());
        assert!(refuse_routing(Some("bob@box"), Some("")).is_ok());
    }

    /// The gate reads the runner's connectors file on every call (ov-586):
    /// a connector named there passes, and a file that's missing, garbled,
    /// oversized or shaped otherwise refuses every MCP tool.
    #[test]
    fn the_gate_reads_its_members_connectors_and_fails_closed() {
        let dir = farcooler_core::scratch::dir("rep-connectors");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rep-1-connectors.json");
        let call = serde_json::json!({ "tool_name": "mcp__claude_ai_Slack__send_message", "tool_input": {} }).to_string();
        let ask = |connectors: &[String]| answer(&call, Path::new("/w/repo"), "/x/farcooler", &resolve, &|_| true, &|_| Some(true), connectors).0;

        std::fs::write(&file, r#"{"connectors":["Slack"]}"#).unwrap();
        assert_eq!(read_connectors(&file), vec!["Slack".to_string()]);
        assert_eq!(ask(&read_connectors(&file)), 0);
        let big = format!(r#"{{"connectors":["Slack"],"pad":"{}"}}"#, "x".repeat(70_000));
        for broken in ["", "not json", r#"{"connectors":"Slack"}"#, r#"{"connectors":["Slack"],"more":1}"#, r#"["Slack"]"#, big.as_str()] {
            std::fs::write(&file, broken).unwrap();
            assert!(read_connectors(&file).is_empty(), "{broken:.40}");
            assert_eq!(ask(&read_connectors(&file)), BLOCK, "{broken:.40}");
        }
        assert!(read_connectors(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

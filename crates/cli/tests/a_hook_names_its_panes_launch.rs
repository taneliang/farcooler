//! A pane's launch id (`FARCOOLER_LAUNCH`, ov-886) rides on every frame its hooks send, so the daemon
//! can tell a late hook from a restarted pane's killed process apart from its replacement's.
use std::process::Stdio;

use farcooler_agent_hooks::wire::{HookLine, decode_line};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Runs the real binary with `launch` in its environment (if any) and returns the frame it sent.
async fn frame(launch: Option<&str>) -> HookLine {
    let dir = tempfile::tempdir().expect("a directory");
    let socket = dir.path().join("h.sock");
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"));
    command
        .args(["hook", "--agent", "codex", "--event", "Stop", "--socket"])
        .arg(&socket)
        .args(["--deadline-ms", "5000"])
        .env_remove("FARCOOLER_LAUNCH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(launch) = launch {
        command.env("FARCOOLER_LAUNCH", launch);
    }
    let mut child = command.spawn().expect("the hook runs");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(br#"{"session_id":"abc","cwd":"/tmp"}"#).await.unwrap();
    drop(stdin);
    let (accepted, _) = tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
        .await
        .expect("the hook connected")
        .expect("accept");
    let mut reader = tokio::io::BufReader::new(accepted);
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("a line");
    let _ = reader.get_mut().write_all(b"{}\n").await;
    let _ = child.wait_with_output().await;
    decode_line::<HookLine>(line.trim()).expect("a frame")
}

#[tokio::test]
async fn a_hook_names_the_launch_its_pane_carries() {
    let launch = "019a6b2e-3c4d-7e5f-8a9b-0c1d2e3f4a5b";
    let line = frame(Some(launch)).await;
    assert_eq!(line.launch.as_deref(), Some(launch));
    assert_eq!(line.payload["session_id"], "abc", "the agent's payload is passed through untouched");
}

#[tokio::test]
async fn a_hook_outside_a_launched_pane_names_none() {
    assert_eq!(frame(None).await.launch, None);
}

/// Only a uuid: the daemon parses nothing else, and a value that isn't one says nothing about which
/// launch this is.
#[tokio::test]
async fn a_launch_that_is_not_a_uuid_is_dropped() {
    assert_eq!(frame(Some("not-a-launch")).await.launch, None);
    assert_eq!(frame(Some("")).await.launch, None);
}

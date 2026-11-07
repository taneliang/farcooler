//! A native workflow build's hooks file names one pinned socket, but each build listens on its own.
//! The build's pane carries `FARCOOLER_NATIVE_HOOK_SOCKET`, and the hook reports there instead.
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// Runs the real binary against `named` with `private` in the environment (if any); returns the
/// line the hook sent to whichever of the two sockets heard it.
async fn heard(private: bool) -> (Option<String>, Option<String>) {
    let dir = tempfile::tempdir().expect("a directory");
    let named = dir.path().join("h.sock");
    let other = dir.path().join("n-1.sock");
    let a = tokio::net::UnixListener::bind(&named).expect("bind");
    let b = tokio::net::UnixListener::bind(&other).expect("bind");
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"));
    command.args(["hook", "--agent", "claude", "--event", "UserPromptSubmit", "--socket"]).arg(&named).args(["--deadline-ms", "5000"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if private { command.env("FARCOOLER_NATIVE_HOOK_SOCKET", &other); }
    let mut child = command.spawn().expect("the hook runs");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(br#"{"session_id":"abc","prompt":"hi"}"#).await.unwrap();
    drop(stdin);
    async fn read(listener: &tokio::net::UnixListener) -> Option<String> {
        let accepted = tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept()).await.ok()?.ok()?.0;
        let mut reader = tokio::io::BufReader::new(accepted);
        let mut line = String::new();
        reader.read_line(&mut line).await.ok()?;
        let _ = reader.get_mut().write_all(b"{}\n").await;
        Some(line)
    }
    let (first, second) = if private {
        let second = read(&b).await;
        (None, second)
    } else {
        (read(&a).await, None)
    };
    let _ = child.wait_with_output().await;
    (first, second)
}

#[tokio::test]
async fn a_hook_reports_to_the_private_socket_its_pane_names() {
    let (named, private) = heard(true).await;
    assert!(named.is_none());
    assert!(private.is_some_and(|l| l.contains("UserPromptSubmit")), "the private socket heard nothing");
}

#[tokio::test]
async fn a_hook_without_that_variable_reports_where_its_file_says() {
    let (named, private) = heard(false).await;
    assert!(named.is_some_and(|l| l.contains("UserPromptSubmit")), "the named socket heard nothing");
    assert!(private.is_none());
}

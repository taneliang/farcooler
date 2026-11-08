//! A phone watching an engine agent can look and cannot touch (ov-976).
use super::*;
use tokio::io::AsyncReadExt;

#[test]
fn a_watch_names_a_workspace_and_an_item_and_nothing_a_viewer_could_type() {
    let workspace = Uuid::from_u128(7);
    let ask = watch_request(workspace, "ov-9");
    assert_eq!(ask.method, "workflow.watch");
    assert_eq!(ask.target_resource_id.as_deref(), Some(workspace.as_bytes().as_slice()));
    let Some(request::Payload::WorkflowCommand(p)) = ask.payload else { panic!("no payload") };
    assert_eq!((p.schema, p.json.as_str()), (1, r#"{"item":"ov-9"}"#));
}

/// The body of `watch_stream`, comments dropped.
fn body() -> String {
    let source = include_str!("link.rs").split("#[cfg(test)]").next().unwrap();
    let start = source.find("pub async fn watch_stream").unwrap();
    let end = source[start..].find("/// Receive fleet news").unwrap();
    source[start..start + end].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_watch_sends_one_request_and_no_input() {
    let code = body();
    // One call on the channel, ever; the rest of it only reads frames.
    assert_eq!(code.matches("client.call(").count(), 1, "the watch makes exactly one request");
    for verb in ["terminal.write", "terminal.resize", "terminal.tell", "terminal.paste", "terminal.interrupt", "send_keys", "TerminalWrite", "TerminalResize", "client.send", "client.notify"] {
        assert!(!code.contains(verb), "the watch mentions {verb}");
    }
    // The only thing written is to the reader the app holds.
    assert_eq!(code.matches("write_all").count(), 2);
    assert!(code.matches("write_all").count() == code.matches("sink.write_all").count());
}

fn reader(lost: bool) -> (WatchReader, tokio::io::DuplexStream) {
    let (mine, theirs) = tokio::io::duplex(64);
    let task = farcooler_transport::rt::spawn(async {});
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(lost));
    (WatchReader { inner: AttachReader { inner: mine, task }, lost: flag }, theirs)
}

#[tokio::test]
async fn a_reaped_pane_ends_the_watch_cleanly() {
    let (mut r, far) = reader(false);
    drop(far);
    let mut buf = [0u8; 8];
    assert_eq!(r.read(&mut buf).await.unwrap(), 0);
}

#[tokio::test]
async fn a_dropped_link_is_an_error_not_a_finish() {
    let (mut r, far) = reader(true);
    drop(far);
    let mut buf = [0u8; 8];
    let err = r.read(&mut buf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);
}

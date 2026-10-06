//! The fence over a socket (ov-360): claude's `PreToolUse` is answered "no
//! decision" once its call is marked in flight, and not while the session's
//! fence is held; the call's end clears the mark.

use super::*;

fn ingress_for_test() -> HookIngress {
    let store = Arc::new(Store::open_in_memory().expect("store"));
    let inventory: Arc<dyn RuntimeInventory> = Arc::new(farcooler_core::inventory::FakeInventory::default());
    HookIngress::new(store, inventory, Arc::default())
}

/// A hook connection: write `event` for `session`, and read the answer, if
/// one comes within `patience`.
fn hook(socket: &Path, event: &str, session: &str, patience: std::time::Duration) -> Option<String> {
    let line = HookLine {
        agent: Agent::Claude,
        event: event.to_string(),
        payload: serde_json::json!({ "session_id": session, "tool_name": "Bash" }),
    };
    let frame = encode_line(&line).expect("encode");
    let mut stream = std::os::unix::net::UnixStream::connect(socket).expect("connect");
    stream.set_read_timeout(Some(patience)).expect("timeout");
    std::io::Write::write_all(&mut stream, frame.as_bytes()).expect("write");
    let mut reply = String::new();
    match std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut reply) {
        Ok(n) if n > 0 => Some(reply),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pre_tool_use_is_marked_and_answered_after_the_fence() {
    let ingress = ingress_for_test();
    let sock = tempfile::tempdir().expect("dir");
    {
        let ingress = ingress.clone();
        let sock = sock.path().to_path_buf();
        tokio::spawn(async move { ingress.listen(&sock, |_, _| {}).await });
    }
    let socket = HookIngress::socket_path(sock.path());
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let asks = ingress.asks().clone();
    let patience = std::time::Duration::from_secs(2);

    // Heard, answered at once with no decision, and marked.
    let first = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || hook(&socket, "PreToolUse", "s1", patience)
    });
    let reply = first.await.unwrap().expect("an answer");
    assert!(!reply.contains("hold_ms") && !reply.contains("allow"), "{reply:?}");
    assert!(asks.tool_in_flight("s1"));
    let socket2 = socket.clone();
    tokio::task::spawn_blocking(move || hook(&socket2, "PostToolUse", "s1", std::time::Duration::from_millis(200)))
        .await
        .unwrap();
    for _ in 0..200 {
        if !asks.tool_in_flight("s1") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!asks.tool_in_flight("s1"), "the call's end cleared the mark");

    // With the fence held, the answer waits for it to be let go.
    let fence = asks.fence("s1").expect("a fence");
    let held = fence.lock().await;
    let started = std::time::Instant::now();
    let waiting = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || (hook(&socket, "PreToolUse", "s1", patience), std::time::Instant::now())
    });
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    assert!(asks.tool_in_flight("s1"), "marked before the wait");
    let let_go = std::time::Instant::now();
    drop(held);
    let (reply, answered) = waiting.await.unwrap();
    assert!(reply.is_some(), "answered once let go");
    assert!(answered >= let_go, "answered {:?} in, before the fence was let go", answered - started);
}

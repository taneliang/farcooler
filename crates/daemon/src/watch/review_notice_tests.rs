//! The worktree review count riding the relay notices (ov-181). Its own file
//! to keep `watch.rs` inside its size budget.

use super::*;

/// A service with a Main workspace and one claude pane in its checkout.
async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let checkout = rows.iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.store.create_terminal_for_test(checkout.id, main.id);
    (dir, svc, main.id, pane)
}

/// The next notice the watcher sends, waiting out any debounce.
async fn next(taps: &mut tokio::sync::mpsc::UnboundedReceiver<Tapped>) -> Option<Tapped> {
    tokio::time::timeout(std::time::Duration::from_secs(30), taps.recv()).await.ok().flatten()
}

/// The worktrees to review ride the count notice, and a diff that moved
/// with no needs-you item moving is news by itself (ov-181). The count is
/// the app's: a worktree that changed since it was reviewed AND has a diff.
#[tokio::test]
async fn a_moved_review_count_sends_a_count_notice_carrying_it() {
    let (_dir, svc, _, pane) = a_runner().await;
    let checkout = svc.store.get_terminal(pane).unwrap().worktree_id;
    let ws = svc.store.get_worktree(checkout).unwrap();
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();

    assert_eq!(crate::review_ops::waiting(&svc).await, Some(0), "no diff, nothing to review");
    svc.review_cache.set_counts_for_tests(
        ws.id,
        std::path::Path::new(&ws.worktree_path),
        crate::review::Counts::Known(1, 4, 0),
    );
    assert_eq!(crate::review_ops::waiting(&svc).await, Some(1), "a diff nobody has reviewed");

    tokio::time::pause();
    watcher.schedule_count_notice();
    let sent = next(&mut taps).await.expect("a moved review count is news");
    assert_eq!((sent.kind, sent.reviews), (Some("count"), Some(1)));

    // The same pair again is not.
    watcher.schedule_count_notice();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(30), taps.recv()).await.is_err(),
        "a repeat of what the relay holds is not sent"
    );
}

/// What the relay actually receives, not what the tap saw: the count
/// notice's body carries `reviews` under the key the relay reads (ov-181).
#[tokio::test]
async fn the_count_notice_on_the_wire_carries_the_review_count() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, svc, _, pane) = a_runner().await;
    let checkout = svc.store.get_terminal(pane).unwrap().worktree_id;
    let ws = svc.store.get_worktree(checkout).unwrap();
    svc.review_cache.set_counts_for_tests(
        ws.id,
        std::path::Path::new(&ws.worktree_path),
        crate::review::Counts::Known(1, 4, 0),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    crate::push::Pairing { relay: format!("http://{}", listener.local_addr().unwrap()), token: "t".into() }
        .save_in(svc.root_dir())
        .expect("pair");
    let relay = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen).to_string();
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                if seen.len() >= end + 4 + length {
                    socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
                    return serde_json::from_slice::<serde_json::Value>(&seen[end + 4..end + 4 + length]).unwrap();
                }
            }
            assert!(n != 0, "the relay's socket closed before a whole request");
        }
    });
    let watcher = Watcher::new(svc.clone());
    watcher.schedule_count_notice();
    let body = tokio::time::timeout(std::time::Duration::from_secs(30), relay).await.expect("a notice").unwrap();
    assert_eq!(body["kind"], "count", "{body}");
    assert_eq!(body["reviews"], serde_json::json!(1), "{body}");
    assert!(body.get("needsYou").is_some(), "{body}");
}

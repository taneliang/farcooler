use super::*;
use std::sync::Arc;

use crate::service::Service;
use crate::watch::Tapped;

#[test]
fn the_notice_counts_and_names_nothing_else() {
    assert_eq!(body(1), "A pull request wants your review");
    assert_eq!(body(3), "3 pull requests want your review");
}

/// One id on every runner, under `t:` so no app reads it as a pane, and short
/// enough for any collapse id.
#[test]
fn the_notice_id_is_the_same_on_every_runner_and_is_never_a_pane() {
    assert_eq!(NOTICE_ID, "t:pr-review");
    assert!(NOTICE_ID.starts_with("t:") && NOTICE_ID.len() <= 64);
}

async fn a_runner() -> (crate::test_support::ScratchDir, Arc<Service>) {
    let (dir, svc, _repo) = crate::test_support::fixture().await;
    (dir, svc)
}

/// The read's own clock, so the cached requests are not yet stale.
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn a_request() -> crate::pr_review::ReviewRequest {
    crate::pr_review::ReviewRequest {
        url: "https://github.com/acme/secret-billing/pull/213".into(),
        number: 213,
        repository: "acme/secret-billing".into(),
        title: "Rotate the signing key".into(),
        author: "mallory".into(),
        created_at: 1,
    }
}

/// A relay on a local port that answers 200 to every request and hands each
/// request's body on.
async fn a_relay() -> (String, tokio::sync::mpsc::UnboundedReceiver<(String, String)>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    seen.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&seen).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if seen.len() >= end + 4 + length {
                            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await;
                            let line = text.lines().next().unwrap_or_default().to_string();
                            let _ = tx.send((line, String::from_utf8_lossy(&seen[end + 4..end + 4 + length]).to_string()));
                            return;
                        }
                    }
                    if n == 0 {
                        return;
                    }
                }
            });
        }
    });
    (url, rx)
}

/// The announcement as it leaves for the relay, whole: the body the daemon
/// POSTs to `/v1/notify`, read off a socket and not rebuilt in the test. It is
/// a task notice of class review with a fixed title and a count, and nothing
/// of the pull request is in it.
#[tokio::test]
async fn the_body_posted_to_the_relay_names_no_repository_title_login_or_number() {
    let (_dir, svc) = a_runner().await;
    let (relay, mut posted) = a_relay().await;
    crate::push::Pairing { relay, token: "t".into() }.save_in(svc.root_dir()).expect("paired");
    let watcher = Watcher::new(svc.clone());
    svc.pr_requests.put(now_ms(), vec![a_request()]);

    watcher.announce_pr_review(1).await;

    let wire = loop {
        let (line, body) = tokio::time::timeout(std::time::Duration::from_secs(30), posted.recv()).await.unwrap().unwrap();
        assert!(line.starts_with("POST /v1/notify "), "{line}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        if json["kind"] == "task" {
            break (body, json);
        }
    };
    let (text, json) = wire;
    assert_eq!(json["event"], "review");
    assert_eq!(json["level"], "active");
    assert_eq!(json["title"], TITLE);
    assert_eq!(json["subtitle"], "A pull request wants your review");
    assert_eq!(json["noticeId"], "t:pr-review");
    for key in ["task", "workspace", "terminal", "options"] {
        assert!(json.get(key).is_none(), "a review request carries no `{key}`: {text}");
    }
    for secret in ["acme", "secret-billing", "Rotate", "mallory", "github.com", "pull/213", "signing"] {
        assert!(!text.contains(secret), "{secret} reached the relay: {text}");
    }
    // The relay is told how many, as a number inside `needsYou`, and a hash of
    // the URL would let it test a guess at one: nothing but the count (ov-590).
    assert_eq!((&json["needsYou"], &json["prReviews"]), (&serde_json::json!(1), &serde_json::json!(1)), "{text}");
    use sha2::Digest;
    let digest = sha2::Sha256::digest(a_request().url.as_bytes());
    let hash: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    assert!(!text.contains(&hash), "a hash of the URL reached the relay: {text}");
}

/// A runner that has a request to tell about sends a task notice of class
/// review, and a tap sees the same fields.
#[tokio::test]
async fn an_announced_request_is_a_review_class_task_notice() {
    let (_dir, svc) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    svc.pr_requests.put(now_ms(), vec![a_request()]);
    watcher.announce_pr_review(1).await;
    let sent: Tapped = tokio::time::timeout(std::time::Duration::from_secs(30), taps.recv()).await.unwrap().unwrap();
    assert_eq!((sent.kind, sent.event, sent.level), (Some("task"), Some("review"), Some("active")));
    assert_eq!(sent.notice_id.as_deref(), Some("t:pr-review"));
    assert_eq!((sent.task, sent.terminal, sent.workspace), (None, None, None));
}

#[tokio::test]
async fn an_unpaired_runner_with_nobody_listening_sends_nothing() {
    let (_dir, svc) = a_runner().await;
    let watcher = Watcher::new(svc);
    // No pairing and no tap: `audience` is none, so nothing is gathered or sent.
    watcher.announce_pr_review(2).await;
}

/// A runner with nothing to review says so with a zero, beside its count, so
/// the relay does not carry a stale maximum for it (ov-590).
#[tokio::test]
async fn a_runner_with_no_request_sends_zero_pull_requests_beside_its_count() {
    let (_dir, svc) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    let (relay, mut posted) = a_relay().await;
    crate::push::Pairing { relay, token: "t".into() }.save_in(svc.root_dir()).expect("paired");

    watcher.announce_pr_review(0).await;

    let (_, body) = tokio::time::timeout(std::time::Duration::from_secs(30), posted.recv()).await.unwrap().unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!((&json["needsYou"], &json["prReviews"]), (&serde_json::json!(0), &serde_json::json!(0)), "{body}");
}

/// A pull request swapped for another item leaves the total where it was and
/// moves what the relay maximizes over, so the same total is not "told".
#[tokio::test]
async fn a_moved_pull_request_count_is_news_though_the_total_is_the_same() {
    let (_dir, svc) = a_runner().await;
    let watcher = Watcher::new(svc.clone());
    svc.pr_requests.put(now_ms(), vec![a_request()]);
    let (count, prs) = watcher.needs_you_counted().await.expect("a count");
    assert_eq!(prs, 1);
    watcher.told(count, None, prs);
    assert!(watcher.already_told(count, None, prs));

    svc.pr_requests.put(now_ms(), Vec::new());
    // The total below would be unchanged by an ask taking the request's place.
    let (_, prs) = watcher.needs_you_counted().await.expect("a count");
    assert_eq!(prs, 0);
    assert!(!watcher.already_told(count, None, prs), "the request left and the relay was not told");
}

/// A notice's two numbers come from one gather: a later gather that saw the
/// request go (another notice building at once) does not rewrite the pull
/// requests inside a total that still counted it.
#[tokio::test]
async fn a_notice_carries_the_pull_requests_of_the_gather_its_count_came_from() {
    let (_dir, svc) = a_runner().await;
    let (relay, mut posted) = a_relay().await;
    crate::push::Pairing { relay, token: "t".into() }.save_in(svc.root_dir()).expect("paired");
    let watcher = Watcher::new(svc.clone());
    svc.pr_requests.put(now_ms(), vec![a_request()]);
    let (count, prs) = watcher.needs_you_counted().await.expect("a count");
    assert_eq!((count, prs), (1, 1));

    // Another gather runs before this notice is delivered, and sees none.
    svc.pr_requests.put(now_ms(), Vec::new());
    assert_eq!(watcher.needs_you_counted().await, Some((0, 0)));

    let pairing = crate::push::Pairing::load_in(svc.root_dir());
    let outgoing = crate::push::Outgoing {
        kind: Some("count"),
        needs_you: Some(count),
        pr_reviews: Some(prs),
        ..Default::default()
    };
    assert!(watcher.deliver_push(pairing, outgoing).await);
    let (_, body) = tokio::time::timeout(std::time::Duration::from_secs(30), posted.recv()).await.unwrap().unwrap();
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!((&json["needsYou"], &json["prReviews"]), (&serde_json::json!(1), &serde_json::json!(1)), "{body}");
}

//! A pane is read the way the engine's builds are: through a real tmux, with claude's own captured screens.
use std::sync::atomic::{AtomicBool, Ordering};

use uuid::Uuid;

use super::*;

const TRUST: &str = include_str!("../../core/captures/claude-trust-gate.txt");
const PERMISSION: &str = include_str!("../../core/captures/claude-blocked.txt");
const IDLE: &str = include_str!("../../core/captures/claude-idle-fresh.txt");

#[test]
fn a_trust_or_permission_screen_has_a_question_and_an_idle_one_has_none() {
    assert!(asking("claude", TRUST).is_some_and(|q| q.contains("Trust this folder")), "{:?}", asking("claude", TRUST));
    assert!(asking("claude", PERMISSION).is_some());
    assert_eq!(asking("claude", IDLE), None);
}

struct Pane {
    tmux: TmuxServer,
    id: String,
    dir: std::path::PathBuf,
}
impl Drop for Pane {
    fn drop(&mut self) {
        farcooler_tmux::reap_server(self.tmux.socket());
    }
}
impl Pane {
    /// A pane that prints `screen` as it is, then waits.
    async fn showing(screen: &str) -> Self {
        assert!(farcooler_core::programs::find("tmux").is_some(), "these tests need tmux");
        let dir = farcooler_core::scratch::dir("engine-stuck");
        let file = dir.join("screen.txt");
        std::fs::write(&file, screen).unwrap();
        let tmux = TmuxServer::in_dir(&format!("test-{}", Uuid::now_v7().simple()), Uuid::now_v7(), &dir);
        let window = tmux
            .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "t", "/tmp", &format!("sh -c \"cat '{}'; sleep 600\"", file.display()))
            .await
            .expect("a window");
        Self { tmux, id: window.pane_id, dir }
    }
}

/// Run the watch for `ms`, setting `heard` after `hear_at` when given.
async fn run(pane: &Pane, item: &str, heard: &AtomicBool, ms: u64) -> Vec<Stuck> {
    let mut seen = Vec::new();
    let mut watching = std::pin::pin!(watch("ws-test", item, "claude", &pane.tmux, &pane.id, heard));
    let end = tokio::time::sleep(Duration::from_millis(ms));
    tokio::pin!(end);
    let mut look = tokio::time::interval(Duration::from_millis(40));
    loop {
        tokio::select! {
            never = &mut watching => match never {},
            _ = &mut end => break,
            _ = look.tick() => seen.extend(list().into_iter().filter(|s| s.item == item)),
        }
    }
    seen
}

#[tokio::test]
async fn a_build_on_the_trust_question_is_listed_with_it_before_any_hook() {
    let pane = Pane::showing(TRUST).await;
    let heard = AtomicBool::new(false);
    // Well inside `START_WITHIN`: the question alone puts it in the list.
    let seen = run(&pane, "ov-t1", &heard, 200).await;
    assert!(seen.iter().any(|s| s.question.contains("Trust this folder")), "{seen:?}");
    assert!(list().iter().all(|s| s.item != "ov-t1"), "ending the watch unlists it");
    let _ = &pane.dir;
}

#[tokio::test]
async fn a_build_nobody_hears_from_is_listed_after_the_window_and_one_that_spoke_is_not() {
    let pane = Pane::showing(IDLE).await;
    let silent = AtomicBool::new(false);
    let early = run(&pane, "ov-t2", &silent, 150).await;
    assert!(early.is_empty(), "quiet inside the window: {early:?}");
    let late = run(&pane, "ov-t2", &silent, 700).await;
    assert!(late.iter().any(|s| s.question.contains("Hasn’t started")), "{late:?}");

    let spoke = AtomicBool::new(true);
    spoke.store(true, Ordering::Relaxed);
    assert!(run(&pane, "ov-t3", &spoke, 700).await.is_empty());
}

#[tokio::test]
async fn a_build_that_becomes_stuck_tells_clients_to_reread_needs_you() {
    use farcooler_protocol::v1::event::Payload;
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    let watcher = crate::watch::Watcher::new(svc);
    let mut events = watcher.subscribe();
    let pane = Pane::showing(TRUST).await;
    let heard = AtomicBool::new(false);
    run(&pane, "ov-t4", &heard, 1000).await;
    let said = tokio::time::timeout(Duration::from_secs(3), async {
        while let Ok(event) = events.recv().await {
            if matches!(event.payload, Some(Payload::NeedsYouChanged(_))) { return true; }
        }
        false
    }).await.unwrap_or(false);
    assert!(said, "no NeedsYouChanged was sent when the build became stuck");
}

//! An idle daemon's tick reads nothing from the host (ov-388). Its own file to
//! keep `watch.rs` inside its size budget.

use super::*;

/// The sampling loop on a daemon with no pane never reads the process table or
/// the sockets: that pair of walks, once a second, is what kept the load in
/// the hundreds. With a pane, the next tick reads the table once.
#[tokio::test]
async fn an_idle_daemon_samples_without_walking_the_host() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let watcher = Watcher::new(svc.clone());
    for _ in 0..4 {
        watcher.sample().await;
    }
    assert_eq!(watcher.host_walk.reads(), (0, 0), "no pane, no walk");

    let rows = svc.store.list_worktrees_for_repository(repo).unwrap();
    let main = rows.iter().find(|w| w.is_main_checkout).unwrap();
    svc.create_terminal(main.id, "shell", "shell").await.expect("a shell pane");
    watcher.sample().await;
    assert!(watcher.host_walk.reads().0 >= 1, "a live pane is read");
}

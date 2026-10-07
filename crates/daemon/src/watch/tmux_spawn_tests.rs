//! How many `tmux` processes a tick costs (ov-390). Its own file to keep
//! `watch.rs` inside its size budget.
//!
//! The daemon samples once a second, and before this a tick spawned a
//! `list-panes` three times and a `capture-pane` per shell pane. Counted at the
//! one place that spawns (`TmuxServer::call_counts`), so these tests fail on
//! the cost itself, whatever shape the code takes.

use std::collections::BTreeMap;

use super::screen_cache::ScreenCache;
use super::*;

/// What a sample spawned: the verbs' counts since `before`.
fn spawned_since(svc: &Service, before: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mut now = svc.tmux.call_counts();
    for (verb, was) in before {
        if let Some(n) = now.get_mut(verb) {
            *n -= was;
        }
    }
    now.retain(|_, n| *n > 0);
    now
}

fn tmux_here(test: &str) -> bool {
    if farcooler_core::programs::find("tmux").is_some() {
        return true;
    }
    assert!(std::env::var_os("CI").is_none(), "CI installs tmux, and {test} needs it");
    eprintln!("SKIP {test}: tmux is not installed here");
    false
}

/// A daemon with no terminal has no tmux server, and a tick that reads nothing
/// from tmux spawns nothing: at most the one read that finds that out.
#[tokio::test]
async fn an_idle_daemon_with_no_server_spawns_at_most_one_tmux_call() {
    if !tmux_here("an_idle_daemon_with_no_server_spawns_at_most_one_tmux_call") {
        return;
    }
    let (_dir, svc, _repo) = crate::test_support::fixture().await;
    let watcher = Watcher::new(svc.clone());
    let before = svc.tmux.call_counts();
    for _ in 0..8 {
        watcher.sample().await;
    }
    let spawned = spawned_since(&svc, &before);
    assert!(spawned.values().sum::<u64>() <= 1, "eight ticks with no server: {spawned:?}");
}

/// Ten idle shell panes cost one `list-panes` a tick and no screen reads, where
/// they cost three and ten.
///
/// "Idle" is waited for rather than assumed: a shell that has just started is
/// still drawing its prompt, and a pane that is drawing is READ every tick, as
/// it must be. Once a tick reads nothing, the ticks after it are counted.
#[tokio::test]
async fn ten_idle_panes_cost_one_list_panes_a_tick() {
    if !tmux_here("ten_idle_panes_cost_one_list_panes_a_tick") {
        return;
    }
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    for n in 0..10 {
        svc.create_terminal(main.id, &format!("pane {n}"), "shell").await.expect("a shell pane");
    }
    assert_eq!(svc.inventory_snapshot().panes.len(), 10, "ten live panes");

    let mut quiet = false;
    for _ in 0..15 {
        let before = svc.tmux.call_counts();
        watcher.sample().await;
        if !spawned_since(&svc, &before).contains_key("capture-pane") {
            quiet = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    }
    assert!(quiet, "ten shells never settled");

    let before = svc.tmux.call_counts();
    for _ in 0..6 {
        watcher.sample().await;
    }
    let spawned = spawned_since(&svc, &before);
    assert_eq!(spawned.get("list-panes"), Some(&6), "one read a tick, not two or three: {spawned:?}");
    assert!(spawned.values().sum::<u64>() <= 6, "ten idle panes, six ticks: {spawned:?}");
}

/// A screen that moved is read again, at once, and the cache never hands back
/// the one from before it moved.
#[tokio::test]
async fn a_pane_that_wrote_is_read_again() {
    if !tmux_here("a_pane_that_wrote_is_read_again") {
        return;
    }
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    let term = svc.create_terminal(main.id, "shell", "shell").await.expect("a shell pane");
    let cache = ScreenCache::default();
    let runtime = svc.runtime();

    // Let the prompt land and its second pass, so the next read can be held.
    tokio::time::sleep(std::time::Duration::from_millis(2300)).await;
    let snapshot = svc.inventory.refresh().await;
    let before = svc.tmux.call_counts();
    cache.read(&runtime, &snapshot, term.id).await.expect("a screen");
    cache.read(&runtime, &snapshot, term.id).await.expect("a screen");
    assert_eq!(spawned_since(&svc, &before).get("capture-pane"), Some(&1), "the second read is held");

    runtime.send_input(term.id, "echo FC_MOVED_390\n").await.expect("typed");
    let mut seen = false;
    for _ in 0..40 {
        let snapshot = svc.inventory.refresh().await;
        let (screen, _, _) = cache.read(&runtime, &snapshot, term.id).await.expect("a screen");
        if screen.contains("FC_MOVED_390") {
            seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(seen, "the screen moved and the cache never read it");
}

fn pane(activity: u64) -> farcooler_core::inventory::TaggedPane {
    use farcooler_core::inventory::{ScreenStamp, TaggedPane};
    TaggedPane {
        daemon_id: Uuid::nil(),
        worktree_id: Uuid::nil(),
        terminal_id: Uuid::nil(),
        schema_version: 1,
        pane_id: "%3".into(),
        window_id: "@1".into(),
        columns: 80,
        rows: 24,
        left: 0,
        top: 0,
        window_active: true,
        pane_active: true,
        zoomed: false,
        tty: String::new(),
        dead: false,
        dead_status: None,
        dead_signal: None,
        command: "fish".into(),
        title: String::new(),
        stamp: ScreenStamp { activity, history: 5, cursor: (2, 3), pid: 77 },
    }
}

/// Each reason a held screen stops standing, one at a time.
#[test]
fn a_held_screen_stands_only_while_nothing_it_can_see_moved() {
    use super::screen_cache::Taken;
    let now = 1_000_000;
    let held = |taken_at: u64| Taken::for_test(&pane(now - 5), taken_at);
    let limit = std::time::Duration::from_secs(20);

    assert!(held(now).stands_for(&pane(now - 5), limit), "nothing moved");
    assert!(!held(now).stands_for(&pane(now + 1), limit), "the window was active again");
    assert!(!held(now - 5).stands_for(&pane(now - 5), limit), "taken in the very second of the last output");
    assert!(held(now - 4).stands_for(&pane(now - 5), limit), "taken the second after it");

    let mut grew = pane(now - 5);
    grew.stamp.history += 1;
    assert!(!held(now).stands_for(&grew, limit), "scrollback grew");
    let mut moved = pane(now - 5);
    moved.stamp.cursor = (0, 3);
    assert!(!held(now).stands_for(&moved, limit), "the cursor moved");
    let mut respawned = pane(now - 5);
    respawned.stamp.pid = 78;
    assert!(!held(now).stands_for(&respawned, limit), "a new program");
    let mut resized = pane(now - 5);
    resized.columns = 100;
    assert!(!held(now).stands_for(&resized, limit), "a resize redraws with no output");
    assert!(!held(now).stands_for(&pane(0), limit), "an unknown activity proves nothing");
    let mut first = pane(now - 5);
    first.pane_id = "%0".into();
    assert!(!held(now).stands_for(&first, std::time::Duration::ZERO), "past its age");
    assert!(held(now).stands_for(&pane(now - 5), std::time::Duration::ZERO), "%3's backstop is spread three seconds later");
}

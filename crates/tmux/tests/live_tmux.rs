//! Integration tests against a REAL private tmux server.
//!
//! These prove the thing the unit tests cannot: that tags survive, that identity
//! is provable from a live pane, and that killing one terminal never touches
//! another. Each test uses its own install id so runs cannot collide, and tears
//! its server down afterwards.

use overnight_core::inventory::RuntimeInventory;
use overnight_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

fn unique_server() -> TmuxServer {
    // A fresh socket per test: never the user's default server.
    let install = format!("test-{}", Uuid::now_v7().simple());
    TmuxServer::new(&install, Uuid::now_v7())
}

/// A fresh, isolated server, or `None` if there is no live tmux to test
/// against.
///
/// These are integration tests against a real tmux binary, which is not a
/// given everywhere this suite runs. A missing tmux has to make the test a
/// skip, never a failure, or CI images without tmux installed would be red
/// for a reason that has nothing to do with the code under test.
async fn live_server() -> Option<TmuxServer> {
    let has_tmux = tokio::process::Command::new("tmux").arg("-V").output().await.is_ok();
    if !has_tmux {
        return None;
    }
    Some(unique_server())
}

#[tokio::test]
async fn creates_a_tagged_window_and_proves_its_identity() {
    let srv = unique_server();
    let workspace = Uuid::now_v7();
    let terminal = Uuid::now_v7();

    let win = srv
        .create_terminal_window(workspace, terminal, "shell", "/tmp", "sleep 30")
        .await
        .expect("create window");

    assert!(win.window_id.starts_with('@'), "stable window id");
    assert!(win.pane_id.starts_with('%'), "stable pane id");

    let panes = srv.list_tagged_panes().await.expect("list panes");
    let mine: Vec<_> = panes.iter().filter(|p| p.terminal_id == terminal).collect();

    assert_eq!(mine.len(), 1, "exactly one pane proves this terminal");
    assert_eq!(mine[0].daemon_id, srv.daemon_id());
    assert_eq!(mine[0].workspace_id, workspace);

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn killing_one_terminal_leaves_the_others_running() {
    let srv = unique_server();
    let ws = Uuid::now_v7();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();

    srv.create_terminal_window(ws, a, "a", "/tmp", "sleep 30").await.unwrap();
    srv.create_terminal_window(ws, b, "b", "/tmp", "sleep 30").await.unwrap();

    assert_eq!(srv.list_tagged_panes().await.unwrap().len(), 2);

    assert!(srv.kill_terminal_window(a).await.unwrap(), "killed a");

    let left = srv.list_tagged_panes().await.unwrap();
    assert_eq!(left.len(), 1, "only one terminal removed");
    assert_eq!(left[0].terminal_id, b, "the survivor is b");

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn killing_an_unknown_terminal_is_a_no_op() {
    let srv = unique_server();
    srv.create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "x", "/tmp", "sleep 30")
        .await
        .unwrap();

    // A terminal id we never created must not match anything.
    assert!(!srv.kill_terminal_window(Uuid::now_v7()).await.unwrap());
    assert_eq!(srv.list_tagged_panes().await.unwrap().len(), 1);

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn an_exited_command_is_observed_as_dead_not_silently_gone() {
    let srv = unique_server();
    let ws = Uuid::now_v7();
    let t = Uuid::now_v7();

    // Exits immediately with a distinctive code.
    srv.create_terminal_window(ws, t, "quick", "/tmp", "sh -c 'exit 42'").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let panes = srv.list_tagged_panes().await.unwrap();
    let p = panes
        .iter()
        .find(|p| p.terminal_id == t)
        .expect("remain-on-exit retains the pane so the exit is observable");

    assert!(p.dead, "the pane reports itself dead");
    assert!(!p.proves_life(), "a dead pane must never prove life");
    assert_eq!(p.dead_status, Some(42), "the exact exit code is observable");

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn a_killed_window_stops_proving_identity_entirely() {
    let srv = unique_server();
    let ws = Uuid::now_v7();
    let long = Uuid::now_v7();
    let doomed = Uuid::now_v7();

    srv.create_terminal_window(ws, long, "long", "/tmp", "sleep 30").await.unwrap();
    srv.create_terminal_window(ws, doomed, "doomed", "/tmp", "sleep 30").await.unwrap();

    // Killing the window removes the pane outright, unlike a command exiting.
    assert!(srv.kill_terminal_window(doomed).await.unwrap());

    let panes = srv.list_tagged_panes().await.unwrap();
    assert!(
        !panes.iter().any(|p| p.terminal_id == doomed),
        "a killed window leaves no pane at all, so identity is unprovable"
    );
    assert!(panes.iter().any(|p| p.terminal_id == long), "the neighbour is untouched");

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn live_inventory_reflects_reality_after_refresh() {
    let srv = unique_server();
    let inv = LiveInventory::new(srv.clone());

    // Before any refresh nothing is proved alive.
    assert!(!inv.snapshot().inventory_healthy);

    let t = Uuid::now_v7();
    srv.create_terminal_window(Uuid::now_v7(), t, "shell", "/tmp", "sleep 30").await.unwrap();

    let snap = inv.refresh().await;
    assert!(snap.inventory_healthy);
    assert_eq!(inv.snapshot().claimants(t).len(), 1);

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn resize_and_capture_target_the_exact_window() {
    let srv = unique_server();
    let t = Uuid::now_v7();
    let win = srv
        .create_terminal_window(Uuid::now_v7(), t, "sized", "/tmp", "sleep 30")
        .await
        .unwrap();

    srv.resize_window(&win.window_id, 100, 30).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let panes = srv.list_tagged_panes().await.unwrap();
    let p = panes.iter().find(|p| p.terminal_id == t).unwrap();
    assert_eq!((p.columns, p.rows), (100, 30));

    // capture-pane must return something without erroring
    srv.capture_pane(&win.pane_id, 100).await.unwrap();

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn respawning_a_pane_keeps_its_id_its_tag_and_its_place() {
    // The toggle's whole correctness argument. If the pane id changed, the
    // terminal would become unidentifiable and derive as `lost`; if the
    // rectangle changed, a four-tile layout would reflow every time someone
    // opened a chat.
    let Some(server) = live_server().await else { return };
    let workspace = Uuid::now_v7();
    let terminal = Uuid::now_v7();
    let window = server
        .create_terminal_window(workspace, terminal, "respawn", "/tmp", "/bin/sh -c 'sleep 300'")
        .await
        .expect("window");

    let before = server.list_tagged_panes().await.expect("panes");
    let pane = before.iter().find(|p| p.terminal_id == terminal).expect("tagged pane");
    let pane_id = pane.pane_id.clone();

    server
        .respawn_pane(&pane_id, "/tmp", "/bin/sh -c 'sleep 300'")
        .await
        .expect("respawn succeeds");

    let after = server.list_tagged_panes().await.expect("panes");
    let same = after.iter().find(|p| p.terminal_id == terminal).expect("still tagged");
    assert_eq!(same.pane_id, pane_id, "pane identity must survive a respawn");

    let _ = server.kill_terminal_window(terminal).await;
    let _ = window;
}

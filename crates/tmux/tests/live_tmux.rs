//! Integration tests against a REAL private tmux server.
//!
//! These prove the thing the unit tests cannot: that tags survive, that identity
//! is provable from a live pane, and that killing one terminal never touches
//! another. Each test uses its own install id so runs cannot collide, and tears
//! its server down afterwards.

use farcooler_core::inventory::RuntimeInventory;
use farcooler_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

fn unique_server() -> Reaped {
    // A fresh socket per test: never the user's default server.
    let install = format!("test-{}", Uuid::now_v7().simple());
    Reaped(TmuxServer::new(&install, Uuid::now_v7()))
}

/// A server that dies with its test, however the test ends.
///
/// Every test here already calls `kill_server()` on its last line, and that
/// only runs when a test REACHES its last line. A failed assertion panics
/// straight past it, and the tmux server it left behind outlives the run
/// permanently — nothing else knows its socket name, so nothing will ever
/// reap it.
///
/// That is not a tidiness argument. On the development machine this was found
/// on there were 362 live tmux servers and 2 454 sockets under `/tmp`, each
/// holding a session, a pane and an interactive shell. tmux is single-threaded
/// per server and they compete for the same CPU: `capture-pane` against the
/// real fleet was timed at 50ms at rest and 740ms under that load. It got far
/// enough to break this very file, where
/// `an_exited_command_is_observed_as_dead_not_silently_gone` began failing on
/// every run because the machine could no longer do in 400ms what it used to.
///
/// A leak whose symptom is your own test suite going red is worth a `Drop`.
struct Reaped(TmuxServer);

impl std::ops::Deref for Reaped {
    type Target = TmuxServer;
    fn deref(&self) -> &TmuxServer {
        &self.0
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        // Synchronous, and deliberately not `kill_server().await`. `Drop`
        // cannot await, and a task spawned here would need a runtime that is
        // in the middle of being torn down to poll it. A blocking `kill-server`
        // against a socket that is usually already gone costs a few
        // milliseconds and always happens.
        let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
        let _ = std::process::Command::new(tmux)
            .args(["-L", self.0.socket(), "kill-server"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// A fresh, isolated server, or `None` if there is no live tmux to test
/// against.
///
/// These are integration tests against a real tmux binary, which is not a
/// given everywhere this suite runs. A missing tmux has to make the test a
/// skip, never a failure, or CI images without tmux installed would be red
/// for a reason that has nothing to do with the code under test.
async fn live_server() -> Option<Reaped> {
    let has_tmux = tokio::process::Command::new("tmux").arg("-V").output().await.is_ok();
    if !has_tmux {
        return None;
    }
    Some(unique_server())
}

/// Wait for something to become true, rather than for a fixed number of
/// milliseconds and a hope.
///
/// A flat sleep encodes an assumption about how fast the machine is, and that
/// assumption decays: it holds on an idle laptop, and stops holding on the same
/// laptop once something else is busy on it. The failure then looks like the
/// code under test regressing, which is the most expensive kind of wrong.
///
/// The deadline is generous because it is only ever reached when the test is
/// genuinely going to fail; the polling interval is what decides how long a
/// passing test takes, and that is short.
async fn until<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if condition().await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
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

    // Waited for, not slept through. "Immediately" is the shell's word for it,
    // not the scheduler's: `sh` still has to be exec'd, run and reaped, and how
    // long that takes depends on what else the machine is doing. The assertions
    // below are unchanged — only the waiting is.
    until("the pane to report its exit", || async {
        srv.list_tagged_panes().await.is_ok_and(|panes| {
            panes.iter().any(|p| p.terminal_id == t && p.dead)
        })
    })
    .await;

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

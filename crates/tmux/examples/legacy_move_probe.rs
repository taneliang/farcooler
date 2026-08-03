//! Reproduces identity loss when a pane created by an older daemon is moved.
//!
//! Terminals created before identity moved from the window to the pane carry
//! their id as a WINDOW option. That was correct while a window held exactly one
//! pane — and it is exactly the shape of every terminal in a fleet that predates
//! the change. `join-pane` makes the pane inherit the DESTINATION window's
//! options instead, so the id is gone, the record derives as lost, and its
//! workspace as error.
//!
//! An integration test cannot reach this: its fixtures are created by the current
//! code, which already tags the pane. So the legacy shape is built by hand here.

use farcooler_tmux::TmuxServer;
use farcooler_tmux::windows::Axis;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let daemon = Uuid::now_v7();
    let server = TmuxServer::new("legacy-probe", daemon);
    let ws = Uuid::now_v7();

    let t1 = Uuid::now_v7();
    let a = server
        .create_terminal_window(ws, t1, "keep", "/tmp", "sh -c 'sleep 300'")
        .await
        .expect("first window");

    let t2 = Uuid::now_v7();
    let b = server
        .create_terminal_window(ws, t2, "legacy", "/tmp", "sh -c 'sleep 300'")
        .await
        .expect("second window");

    // Make the second one look like a terminal from before the change: identity
    // on the window, nothing on the pane.
    server
        .run(&["set-option", "-w", "-t", &b.window_id, "@farcooler_terminal_id", &t2.to_string()])
        .await
        .expect("window tag");
    server
        .run(&["set-option", "-up", "-t", &b.pane_id, "@farcooler_terminal_id"])
        .await
        .expect("clear pane tag");

    let before = server.list_tagged_panes().await.expect("list");
    let found = before.iter().filter(|p| p.terminal_id == t2).count();
    println!("legacy pane visible before the move: {found} (expect 1)");
    assert_eq!(found, 1, "the legacy shape must be readable to begin with");

    server.join_pane(&b.pane_id, &a.pane_id, Axis::Vertical, false, t2).await.expect("join");

    let after = server.list_tagged_panes().await.expect("list");
    let survived = after.iter().filter(|p| p.terminal_id == t2).count();
    println!("still identified after being joined: {survived} (expect 1)");

    server.kill_server().await.ok();
    assert_eq!(survived, 1, "a moved pane must still know which terminal it is");
    println!("PROBE PASSED");
}

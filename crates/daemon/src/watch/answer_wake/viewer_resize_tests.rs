//! ov-853: a browser fits the pane to its own window and, on leaving, resizes
//! it back to the size the terminal list gave it (`services/web/src/client/
//! lease.ts`). The daemon's half of that: after a smaller viewer's size, the
//! earlier size is the pane's again, to the cell, in real tmux.

use super::*;

/// The pane's size as tmux has it, not as the inventory's view last heard.
async fn size_in_tmux(b: &Board) -> String {
    let out = b
        .svc
        .runtime()
        .tmux
        .run(&["list-panes", "-a", "-F", "#{pane_width}x#{pane_height}"])
        .await
        .expect("list-panes");
    out.stdout.trim().to_string()
}

/// The Mac's 120x40, a browser's 60x20, and the Mac's again.
#[tokio::test]
async fn a_viewers_smaller_size_is_handed_back_to_the_size_before_it() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    b.svc.resize_terminal(agent.id, 120, 40).await.expect("the Mac's size");
    assert_eq!(size_in_tmux(&b).await, "120x40");

    b.svc.resize_terminal(agent.id, 60, 20).await.expect("the browser's size");
    assert_eq!(size_in_tmux(&b).await, "60x20");

    // The view may still hold 120x40 here; the pane must come back anyway.
    b.svc.resize_terminal(agent.id, 120, 40).await.expect("handed back");
    assert_eq!(size_in_tmux(&b).await, "120x40");
}

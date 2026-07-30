use overnight_tmux::TmuxServer;
use overnight_tmux::windows::{Axis, Preset};
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let daemon = Uuid::now_v7();
    let server = TmuxServer::new("probe-run", daemon);
    let ws = Uuid::now_v7();

    let t1 = Uuid::now_v7();
    let win = server
        .create_terminal_window(ws, t1, "probe", "/tmp", "sh -c 'echo ONE; sleep 300'")
        .await
        .expect("create");
    println!("window {} pane {}", win.window_id, win.pane_id);

    // Split it twice: three terminals, one layout.
    let t2 = Uuid::now_v7();
    let p2 = server
        .split_pane(&win.pane_id, Axis::Horizontal, t2, "/tmp", "sh -c 'echo TWO; sleep 300'", false)
        .await
        .expect("split h");
    let t3 = Uuid::now_v7();
    let p3 = server
        .split_pane(&p2, Axis::Vertical, t3, "/tmp", "sh -c 'echo THREE; sleep 300'", false)
        .await
        .expect("split v");
    println!("panes {p2} {p3}");

    server.resize_window(&win.window_id, 120, 40).await.ok();

    for preset in [Preset::EvenHorizontal, Preset::MainVertical, Preset::Tiled] {
        server.select_preset(&win.window_id, preset).await.expect("preset");
        let panes = server.list_tagged_panes().await.expect("list");
        let mut mine: Vec<_> = panes.iter().filter(|p| p.workspace_id == ws).collect();
        mine.sort_by_key(|p| (p.top, p.left));
        println!("\n{}:", preset.as_str());
        for p in &mine {
            println!(
                "  terminal {}  at {},{}  {}x{}  active={}",
                &p.terminal_id.to_string()[..8], p.left, p.top, p.columns, p.rows, p.pane_active
            );
        }
        // Every pane must have a DISTINCT terminal id, which is the whole point
        // of moving the tag from the window to the pane.
        let ids: std::collections::HashSet<_> = mine.iter().map(|p| p.terminal_id).collect();
        assert_eq!(ids.len(), mine.len(), "panes must not share a terminal id");
    }

    let layouts = server.list_layouts().await.expect("layouts");
    println!("\nlayouts owned by this daemon: {}", layouts.len());
    for l in &layouts {
        println!("  {} name={} active={} tree={}", l.window_id, l.name, l.active, l.layout);
    }

    server.select_pane(&p3).await.expect("select");
    server.toggle_zoom(&p3).await.expect("zoom");
    let panes = server.list_tagged_panes().await.expect("list");
    let zoomed: Vec<_> = panes.iter().filter(|p| p.zoomed).collect();
    println!("\nzoomed panes: {} (expect 1)", zoomed.len());
    assert_eq!(zoomed.len(), 1);
    server.unzoom(&win.window_id).await.expect("unzoom");
    let panes = server.list_tagged_panes().await.expect("list");
    assert_eq!(panes.iter().filter(|p| p.zoomed).count(), 0, "unzoom must clear it");

    // Break one out: two layouts.
    let new_window = server.break_pane(&p3, ws).await.expect("break");
    let layouts = server.list_layouts().await.expect("layouts");
    println!("\nafter break-pane: {} layouts, new = {}", layouts.len(), new_window);
    assert_eq!(layouts.len(), 2);

    // And put it back, on the left of pane 1.
    let panes = server.list_tagged_panes().await.expect("list");
    let back = panes.iter().find(|p| p.terminal_id == t3).expect("t3");
    server.join_pane(&back.pane_id, &win.pane_id, Axis::Horizontal, true).await.expect("join");
    let layouts = server.list_layouts().await.expect("layouts");
    println!("after join-pane: {} layouts (expect 1)", layouts.len());
    assert_eq!(layouts.len(), 1);

    let panes = server.list_tagged_panes().await.expect("list");
    let mut mine: Vec<_> = panes.iter().filter(|p| p.workspace_id == ws).collect();
    mine.sort_by_key(|p| p.left);
    println!("\nfinal order left-to-right:");
    for p in &mine {
        println!("  {}  left={}", &p.terminal_id.to_string()[..8], p.left);
    }
    assert_eq!(mine[0].terminal_id, t3, "joined -b must land on the left");

    server.kill_server().await.ok();
    println!("\nALL PROBES PASSED");
}

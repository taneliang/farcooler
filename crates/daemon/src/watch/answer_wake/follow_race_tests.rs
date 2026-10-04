//! A follow that a respawn overtakes (ov-251): it reads the pane's pid, the
//! pane is respawned, and only then does it subscribe. On the board and tmux
//! server of `tests`, whose helpers these use.

use super::*;

/// The follow started with the agent reads the first program's pid, someone
/// respawns the pane, and the answer's check (`streamed_bracketed_paste`)
/// runs before that follow has a record to drop. The follow's record, about
/// the program before, must not stand in for the new one: the next samples
/// follow the program now in the pane. Forced to the tmux 3.4 path, so it
/// runs the same on any tmux.
#[tokio::test]
async fn a_follow_overtaken_by_a_respawn_leaves_the_pane_to_the_next_sample() {
    // Following pipes the pane through the daemon binary, which `--lib`
    // doesn't build. A full run does, and CI's must not skip.
    if crate::runtime::fanout_binary().is_none() {
        assert!(std::env::var_os("CI").is_none(), "no farcoolerd beside the test binary");
        eprintln!("SKIP a_follow_overtaken_by_a_respawn_leaves_the_pane_to_the_next_sample: no farcoolerd");
        return;
    }
    let b = board().await;
    b.svc.assume_tmux_cannot_report_paste_mode();
    let reached = Arc::new(tokio::sync::Notify::new());
    let go = Arc::new(tokio::sync::Notify::new());
    b.svc.hold_next_paste_mode_follow(reached.clone(), go.clone());
    let agent = b.agent("Agent 2", "claude").await;
    tokio::time::timeout(Duration::from_secs(10), reached.notified())
        .await
        .expect("starting the agent follows its pane");
    b.run_unfollowed(&agent, "sleep 600").await;
    assert_eq!(b.svc.streamed_bracketed_paste(agent.id).await, None);
    go.notify_one();
    let mut followed = false;
    for _ in 0..5 {
        b.watcher.sample().await;
        if b.followed(agent.id).await {
            followed = true;
            break;
        }
    }
    assert!(followed, "the record of the program before stood in for the new one");
}

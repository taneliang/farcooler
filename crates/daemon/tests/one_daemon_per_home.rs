//! Only one daemon may own a runtime directory, even when several start at once.
//!
//! The bug this exists to prevent had been running on a developer's machine for
//! days: sixty-three live `farcoolerd` processes, in groups whose start times
//! matched to the second. Each group was one moment when several clients tried
//! to auto-start the daemon together.
//!
//! They raced a check against a condition they were about to change. Every one
//! probed the socket, found nothing listening, spent a hundred milliseconds or
//! more opening SQLite and inventorying tmux, and only then bound — by which
//! point the last to arrive had unlinked everyone else's socket. The losers
//! never noticed and never exited. They held the database open and went on
//! sampling every pane once a second, through a watcher no client could reach,
//! until the machine was restarted.
//!
//! So this test starts them the way that happened: all at once, deliberately.

use std::process::{Command, Stdio};

/// How many to start together.
///
/// More than two, because two can pass by luck — the loser may simply have been
//. slow enough to see the winner's socket. Eight makes the old race essentially
/// certain to produce at least one duplicate.
const RACERS: usize = 8;

fn daemon_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_farcoolerd"))
}

/// Kill the tmux server a daemon started, which killing the daemon does not do.
fn reap_tmux(home: &std::path::Path) {
    let Ok(install) = std::fs::read_to_string(home.join("install-id")) else { return };
    let Some(tmux) = farcooler_core::programs::find("tmux") else { return };
    let _ = Command::new(tmux)
        .args(["-L", &format!("farcooler-{}", install.trim()), "kill-server"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[test]
fn eight_daemons_started_at_once_leave_exactly_one_running() {
    let home = tempfile::tempdir().unwrap();

    let mut started: Vec<std::process::Child> = (0..RACERS)
        .map(|_| {
            Command::new(daemon_binary())
                .env("FARCOOLER_HOME", home.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn farcoolerd")
        })
        .collect();

    // The redundant ones exit on their own; the owner never does. So rather than
    // waiting a fixed time and hoping, poll until exactly one is still up.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut alive = RACERS;
    while std::time::Instant::now() < deadline {
        alive = 0;
        for child in &mut started {
            if matches!(child.try_wait(), Ok(None)) {
                alive += 1;
            }
        }
        if alive <= 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Cleanup before asserting, so a failure does not also leak eight daemons
    // and a tmux server into the machine running the suite.
    for child in &mut started {
        let _ = child.kill();
        let _ = child.wait();
    }
    reap_tmux(home.path());

    assert_eq!(
        alive, 1,
        "exactly one daemon may own a runtime directory; {alive} of {RACERS} were still running, \
         which means the losers bound a socket and then kept the database and a tmux sampler open \
         where nothing could reach them"
    );
}

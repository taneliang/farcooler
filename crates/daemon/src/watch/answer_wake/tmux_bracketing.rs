//! Whether this host's tmux can say which panes take a bracketed paste, and
//! the re-following a restart needs when it can't (ov-965). `tests` takes the
//! flag reader and the board's `relearn_bracketing` from here.

use super::*;

/// Whether this host's tmux reports bracketed paste (`bracket_paste_flag`,
/// new in 3.7), read from `tmux -V` rather than from the flag, so a broken
/// flag read can't switch the tests that need it off. A version it can't
/// read counts as new enough: those tests run, and fail if it isn't.
///
/// The typing tests run on both: below 3.7 the gate reads the pane's output
/// (`paste_mode`), and `settle_stand_in` waits for that record. The tests
/// of what that record can't know run only below 3.7, so Ubuntu's 3.4 leg
/// of CI drives them and macOS's current tmux drives the flag.
///
/// `FARCOOLER_TEST_OLD_TMUX=1` answers false whatever the version, so the
/// below-3.7 path runs on a new tmux too (the train gate's old-tmux stage).
pub(super) fn tmux_tells_bracketing() -> bool {
    static KNOWS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *KNOWS.get_or_init(|| {
        if crate::paste_mode::forced_old_tmux() {
            eprintln!("FARCOOLER_TEST_OLD_TMUX=1: running the below-3.7 path");
            return false;
        }
        let tmux = farcooler_core::programs::find("tmux").expect("these tests need tmux");
        let out = std::process::Command::new(tmux).arg("-V").output().expect("tmux -V");
        let knows = tmux_version_tells_bracketing(&String::from_utf8_lossy(&out.stdout));
        if !knows {
            eprintln!("tmux older than 3.7: skipping the tests that need bracket_paste_flag");
        }
        knows
    })
}

/// `tmux 3.4`, `tmux 3.7c`, `tmux next-3.8`: whether that's 3.7 or later.
fn tmux_version_tells_bracketing(v: &str) -> bool {
    let v = v.trim().trim_start_matches("tmux").trim().trim_start_matches("next-");
    let mut parts = v.split('.');
    let major = parts.next().and_then(|m| m.parse::<u32>().ok());
    let minor = parts.next().map(|m| m.trim_end_matches(|c: char| !c.is_ascii_digit())).and_then(|m| m.parse::<u32>().ok());
    match (major, minor) {
        (Some(major), Some(minor)) => (major, minor) >= (3, 7),
        _ => true,
    }
}

#[test]
fn a_tmux_version_is_read_for_bracketing() {
    for (v, knows) in [("tmux 3.4\n", false), ("tmux 3.3a", false), ("tmux 3.6b", false), ("tmux 3.7", true), ("tmux 3.7c\n", true), ("tmux next-3.8", true), ("tmux 4.0", true), ("tmux master", true), ("", true)] {
        assert_eq!(tmux_version_tells_bracketing(v), knows, "{v:?}");
    }
}

impl Board {
/// After a restart on a tmux older than 3.7, which cannot say whether a
/// pane accepts a paste: follow the pane and have the stand-in set
/// bracketing again, as `settle_stand_in` does for a first start. A newer
/// tmux tells the flag at once and needs nothing.
pub(super) async fn relearn_bracketing(&self, terminal: &Terminal, si: &StandIn) {
    if tmux_tells_bracketing() {
        return;
    }
    self.watcher.sample().await;
    for _ in 0..750 {
        if self.svc.paste_mode_followed(terminal.id).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    si.show("working").await;
    si.show("idle").await;
    for _ in 0..750 {
        if self.svc.streamed_bracketed_paste(terminal.id).await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
}

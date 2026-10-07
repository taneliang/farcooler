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
        farcooler_tmux::reap_server(self.0.socket());
    }
}

/// A fresh, isolated server, or `None` if there is no live tmux to test
/// against.
///
/// These are integration tests against a real tmux binary, which is not a
/// given on every machine this suite runs on, so off CI a missing tmux is a
/// skip, and says so. On CI it is a failure: ci.yml installs tmux for the
/// Rust job ("Install tmux"), so a CI run without it is a broken runner, and
/// four tests passing having run nothing is the one outcome that must not
/// look green. Same rule as the fish half of the launch tests.
///
/// Found the way the crate finds it, through `programs::find`, not a bare
/// `tmux` on PATH: the two disagree whenever PATH lacks the install prefix,
/// and this used to skip while every other test in the file ran tmux fine.
async fn live_server(test: &str) -> Option<Reaped> {
    match farcooler_core::programs::find("tmux") {
        Some(_) => Some(unique_server()),
        None if std::env::var_os("CI").is_some() => {
            panic!("tmux is not installed, and CI must run {test} against a live server")
        }
        None => {
            eprintln!("SKIP {test}: tmux is not installed here");
            None
        }
    }
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
    let worktree = Uuid::now_v7();
    let terminal = Uuid::now_v7();

    let win = srv
        .create_terminal_window(worktree, terminal, "shell", "/tmp", "sleep 30")
        .await
        .expect("create window");

    assert!(win.window_id.starts_with('@'), "stable window id");
    assert!(win.pane_id.starts_with('%'), "stable pane id");

    let panes = srv.list_tagged_panes().await.expect("list panes");
    let mine: Vec<_> = panes.iter().filter(|p| p.terminal_id == terminal).collect();

    assert_eq!(mine.len(), 1, "exactly one pane proves this terminal");
    assert_eq!(mine[0].daemon_id, srv.daemon_id());
    assert_eq!(mine[0].worktree_id, worktree);

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

/// The race behind the test above, polled for rather than stumbled into.
///
/// tmux sets `pane_dead` on the pty's end of file and the exit status on
/// SIGCHLD, a loop pass apart. Reading as fast as possible right after each
/// exit is what lands between the two: on Linux this caught a dead pane with
/// no code on most runs before `list_tagged_panes` learned to wait. macOS
/// orders the two the other way, so there it passes either way.
#[tokio::test]
async fn a_dead_pane_is_never_read_without_its_exit_code() {
    let srv = unique_server();
    let ws = Uuid::now_v7();

    for _ in 0..20 {
        let t = Uuid::now_v7();
        srv.create_terminal_window(ws, t, "quick", "/tmp", "sh -c 'exit 42'").await.unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let panes = srv.list_tagged_panes().await.unwrap();
            let p = panes.iter().find(|p| p.terminal_id == t).expect("remain-on-exit retains the pane");
            if p.dead {
                assert_eq!(p.dead_status, Some(42), "a dead pane is read with its exit code");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "timed out waiting for the exit");
            tokio::task::yield_now().await;
        }
    }

    srv.kill_server().await.unwrap();
}

/// An exit that takes its time to settle is still read with its code.
///
/// A stand-in for a loaded machine, made deterministic: the command closes
/// its tty — on Linux that is the end of file that marks the pane dead — and
/// only exits a second later, so tmux has no status to give for that second.
/// On a busy CI runner the same gap opened on its own and outlasted a 500 ms
/// wait. macOS reports no end of file while the process lives, so there the
/// pane simply turns dead with its code.
#[tokio::test]
async fn an_exit_that_settles_slowly_is_still_read_with_its_code() {
    let srv = unique_server();
    let t = Uuid::now_v7();

    // `trap '' HUP` because tmux closing the pty hangs up the session.
    let cmd = "exec sh -c \"trap '' HUP; exec 0<&- 1>&- 2>&-; sleep 1; exit 42\"";
    srv.create_terminal_window(Uuid::now_v7(), t, "slow", "/tmp", cmd).await.unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let panes = srv.list_tagged_panes().await.unwrap();
        let p = panes.iter().find(|p| p.terminal_id == t).expect("remain-on-exit retains the pane");
        if p.dead {
            assert_eq!(p.dead_status, Some(42), "a dead pane is read with its exit code");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the exit");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn a_command_killed_by_a_signal_is_observed_with_its_signal() {
    let srv = unique_server();
    let t = Uuid::now_v7();

    // `exec`, so the signal kills the pane's own process: the command runs
    // under `default-shell -c`, which would otherwise outlive `sh` and turn its
    // death into exit status 137.
    srv.create_terminal_window(Uuid::now_v7(), t, "killed", "/tmp", "exec sh -c 'kill -9 $$'")
        .await
        .unwrap();

    until("the pane to report its death", || async {
        srv.list_tagged_panes().await.is_ok_and(|panes| {
            panes.iter().any(|p| p.terminal_id == t && p.dead)
        })
    })
    .await;

    let panes = srv.list_tagged_panes().await.unwrap();
    let p = panes.iter().find(|p| p.terminal_id == t).expect("remain-on-exit retains the pane");

    // tmux names the signal `kill` on macOS and `9` on Linux.
    assert!(
        matches!(p.dead_signal.as_deref(), Some("kill" | "KILL" | "9")),
        "the signal is observable, got {:?}",
        p.dead_signal
    );
    assert_eq!(p.dead_status, None, "a signal death has no exit code");
    assert!(!p.exit_unsettled(), "a signal settles the exit");

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
async fn a_scrollback_capture_stops_where_the_screen_starts() {
    // `-E -1` is the whole of `capture_scrollback`, and only a real tmux can
    // say whether it means what this depends on it meaning. Captured without
    // it, the history would arrive with the visible screen glued to the bottom
    // of it, and a client would open able to scroll up into a second copy of
    // what it is already showing.
    let Some(srv) = live_server("a_scrollback_capture_stops_where_the_screen_starts").await else { return };
    let t = Uuid::now_v7();
    let win = srv
        .create_terminal_window(Uuid::now_v7(), t, "scrolled", "/tmp", "seq 1 60; sleep 30")
        .await
        .unwrap();
    srv.resize_window(&win.window_id, 80, 10).await.unwrap();

    until("the pane to hold more than a screenful", || async {
        srv.capture_scrollback(&win.pane_id).await.is_ok_and(|h| h.lines().count() > 10)
    })
    .await;

    let history = srv.capture_scrollback(&win.pane_id).await.unwrap();
    let screen = srv.capture_screen(&win.pane_id).await.unwrap();
    let last_visible = screen.lines().rfind(|l| !l.trim().is_empty()).unwrap().trim();

    assert!(history.contains("\n1\n") || history.starts_with("1\n"), "the oldest line is kept");
    assert!(
        !history.lines().any(|l| l.trim() == last_visible),
        "the screen's last line is the screen's, not the history's: {last_visible:?}"
    );

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn respawning_a_pane_keeps_its_id_its_tag_and_its_place() {
    // The toggle's whole correctness argument. If the pane id changed, the
    // terminal would become unidentifiable and derive as `lost`; if the
    // rectangle changed, a four-tile layout would reflow every time someone
    // opened a chat.
    let Some(server) = live_server("respawning_a_pane_keeps_its_id_its_tag_and_its_place").await else { return };
    let worktree = Uuid::now_v7();
    let terminal = Uuid::now_v7();
    let window = server
        .create_terminal_window(worktree, terminal, "respawn", "/tmp", "/bin/sh -c 'sleep 300'")
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

/// The activity trace's lower half, counted off a REAL pane's real output.
///
/// This is the test that stops `trace::lines_produced` from being a function
/// that agrees with a fixture. Everything here is genuine: a tmux server, a pty,
/// a shell writing to it, and tmux's own VT emulator deciding what the screen
/// looks like afterwards. The expected number is read out of the pane's own
/// text — the last line number visible in each of the two captures — so the
/// assertion is "the counter agrees with what the terminal actually did", not
/// "the counter returns what the test handed it".
#[tokio::test]
async fn output_lines_are_counted_off_a_real_pane() {
    use farcooler_core::trace::{ScreenShape, lines_produced};

    let Some(srv) = live_server("output_lines_are_counted_off_a_real_pane").await else { return };

    // Ten rows, so the pane fills within the first second and everything after
    // that is a genuine scroll rather than the screen merely filling up.
    let win = srv
        .create_terminal_window(
            Uuid::now_v7(),
            Uuid::now_v7(),
            "chatty",
            "/tmp",
            "sh -c 'i=1; while [ $i -le 2000 ]; do echo \"line $i\"; i=$((i+1)); \
             sleep 0.02; done; sleep 60'",
        )
        .await
        .unwrap();
    srv.resize_window(&win.window_id, 80, 10).await.unwrap();

    /// The highest `line N` the pane is showing, which is how far the program
    /// has got. Read off the capture rather than counted by the test, so the
    /// two captures and the expectation all come from the same reality.
    fn furthest(screen: &str) -> Option<u32> {
        screen
            .lines()
            .filter_map(|l| l.trim().strip_prefix("line ")?.trim().parse::<u32>().ok())
            .max()
    }

    // Wait until the screen is full, so the comparison below is across a
    // scrolling pane and not a filling one.
    until("the pane to fill", || async {
        srv.capture_screen(&win.pane_id).await.ok().and_then(|s| furthest(&s)).is_some_and(|n| n > 12)
    })
    .await;

    let before = srv.capture_screen(&win.pane_id).await.unwrap();
    let at_before = furthest(&before).expect("the pane should be printing numbered lines");

    // Wait for the pane to MOVE, rather than sleeping and hoping it did.
    //
    // This was a flat 120ms sleep, and it failed on CI with "got 0": on a
    // loaded runner the shell had not printed a single line in that window, so
    // the precondition below failed for a reason that has nothing to do with
    // what this test is about. A fixed sleep is a bet on the machine's speed at
    // both ends — too slow and the delta is zero, too fast and it saturates the
    // pane and trips the upper bound.
    //
    // Polling every 25ms bounds it from both sides instead. It returns on the
    // first captured advance, which is one line more often than not and cannot
    // be zero; and the 25ms cadence is far below the time this pane takes to
    // print the ten lines that would make the measurement a saturation rather
    // than a count.
    until("the pane to print another line", || async {
        srv.capture_screen(&win.pane_id)
            .await
            .ok()
            .and_then(|s| furthest(&s))
            .is_some_and(|n| n > at_before)
    })
    .await;
    let after = srv.capture_screen(&win.pane_id).await.unwrap();
    let at_after = furthest(&after).expect("the pane should still be printing numbered lines");

    let actually_printed = at_after - at_before;
    // Both ends asserted, because a test whose expected value could be zero is
    // a test that passes on a counter that always returns zero — and a value
    // at or above the pane height would be the documented saturation rather
    // than a measurement.
    assert!(
        actually_printed > 0 && actually_printed < 10,
        "this test needs a partial screenful between the two captures, got {actually_printed}"
    );

    let counted = lines_produced(&ScreenShape::of(&before), &ScreenShape::of(&after));
    assert_eq!(
        counted, actually_printed,
        "the pane printed lines {at_before}..{at_after} and the counter said {counted}"
    );

    srv.kill_server().await.unwrap();
}

/// A pane that printed nothing produced nothing.
///
/// The other half of the test above, and the one that fails if `lines_produced`
/// ever starts reporting a screenful for a screen that merely still exists.
#[tokio::test]
async fn a_quiet_real_pane_produces_no_output_lines() {
    use farcooler_core::trace::{ScreenShape, lines_produced};

    let Some(srv) = live_server("a_quiet_real_pane_produces_no_output_lines").await else { return };
    let win = srv
        .create_terminal_window(
            Uuid::now_v7(),
            Uuid::now_v7(),
            "quiet",
            "/tmp",
            "sh -c 'echo settled; sleep 60'",
        )
        .await
        .unwrap();
    srv.resize_window(&win.window_id, 80, 10).await.unwrap();

    until("the pane to print its one line", || async {
        srv.capture_screen(&win.pane_id).await.is_ok_and(|s| s.contains("settled"))
    })
    .await;

    let before = srv.capture_screen(&win.pane_id).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let after = srv.capture_screen(&win.pane_id).await.unwrap();

    assert_eq!(
        lines_produced(&ScreenShape::of(&before), &ScreenShape::of(&after)),
        0,
        "a sleeping pane was counted as producing output"
    );

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn a_narrowed_window_keeps_every_panes_share() {
    // Live checklist F1. A worktree window of an agent on the left and two
    // shells stacked on the right, which the Mac sized to 105x36 and then to
    // 55x36. `resize-window` alone took all fifty columns from the right and
    // left the shells one column wide; nothing but a real tmux can say what it
    // does with a layout, so nothing but a real tmux can test the fix.
    let Some(srv) = live_server("a_narrowed_window_keeps_every_panes_share").await else { return };
    use farcooler_tmux::windows::Axis;
    let worktree = Uuid::now_v7();
    let win = srv
        .create_terminal_window(worktree, Uuid::now_v7(), "agent", "/tmp", "sleep 60")
        .await
        .unwrap();
    let right = srv
        .split_pane(&win.pane_id, Axis::Horizontal, Uuid::now_v7(), "/tmp", "sleep 60", false)
        .await
        .unwrap();
    srv.split_pane(&right, Axis::Vertical, Uuid::now_v7(), "/tmp", "sleep 60", false).await.unwrap();

    srv.resize_window_keeping_shares(&win.window_id, 105, 36).await.unwrap();
    srv.set_pane_size(&win.pane_id, 78, 36).await.unwrap();

    let widths = || async {
        let mut panes: Vec<_> = srv
            .list_tagged_panes()
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.window_id == win.window_id)
            .map(|p| (p.pane_id, p.columns, p.rows))
            .collect();
        panes.sort();
        panes
    };
    let agent_width = |panes: &[(String, u32, u32)]| {
        panes.iter().find(|p| p.0 == win.pane_id).map(|p| p.1).unwrap()
    };
    let wide = widths().await;
    assert_eq!(wide.len(), 3);
    assert_eq!(agent_width(&wide), 78, "the divider is where it was put: {wide:?}");

    srv.resize_window_keeping_shares(&win.window_id, 55, 36).await.unwrap();
    let narrow = widths().await;
    for (pane, columns, rows) in &narrow {
        let was = wide.iter().find(|p| &p.0 == pane).unwrap();
        // Its share of the 54 columns left after the divider, give or take one
        // for rounding, but never less than ten where ten will fit.
        let proportional = (f64::from(was.1) * 54.0 / 104.0).floor() as u32;
        assert!(
            *columns >= proportional.min(10),
            "{pane} is {columns} columns at 55 wide, from {} at 105: {narrow:?}",
            was.1
        );
        assert!(columns.abs_diff(proportional) <= 1, "{pane}: {columns} vs {proportional}: {narrow:?}");
        assert_eq!(*rows, was.2, "a change in width leaves the rows alone: {narrow:?}");
    }

    srv.resize_window_keeping_shares(&win.window_id, 105, 36).await.unwrap();
    let back = widths().await;
    assert!(agent_width(&back).abs_diff(78) <= 1, "widening puts the ratio back: {back:?}");

    srv.kill_server().await.unwrap();
}

#[tokio::test]
async fn a_zoomed_window_is_still_zoomed_on_the_same_pane_after_a_resize() {
    // `select-layout` unzooms, so keeping the shares must not cost the zoom,
    // and the pane zoomed again must be the one that was: here the lower
    // shell, not the first pane.
    let Some(srv) = live_server("a_zoomed_window_is_still_zoomed_on_the_same_pane_after_a_resize").await
    else {
        return;
    };
    use farcooler_tmux::windows::Axis;
    let win = srv
        .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "agent", "/tmp", "sleep 60")
        .await
        .unwrap();
    let right = srv
        .split_pane(&win.pane_id, Axis::Horizontal, Uuid::now_v7(), "/tmp", "sleep 60", false)
        .await
        .unwrap();
    let lower = srv
        .split_pane(&right, Axis::Vertical, Uuid::now_v7(), "/tmp", "sleep 60", false)
        .await
        .unwrap();
    srv.resize_window_keeping_shares(&win.window_id, 105, 36).await.unwrap();
    srv.set_pane_size(&win.pane_id, 78, 36).await.unwrap();
    srv.select_pane(&lower).await.unwrap();
    srv.toggle_zoom(&lower).await.unwrap();

    srv.resize_window_keeping_shares(&win.window_id, 55, 36).await.unwrap();
    let panes = srv.list_tagged_panes().await.unwrap();
    let shell = panes.iter().find(|p| p.pane_id == lower).unwrap();
    assert!(shell.zoomed && shell.pane_active, "still zoomed, on the shell: {shell:?}");
    assert_eq!((shell.columns, shell.rows), (55, 36), "and the zoomed pane is the window");

    srv.unzoom(&win.window_id).await.unwrap();
    let panes = srv.list_tagged_panes().await.unwrap();
    let agent = panes.iter().find(|p| p.pane_id == win.pane_id).unwrap();
    assert!(agent.columns.abs_diff(40) <= 1, "the shares held underneath: {panes:?}");

    srv.kill_server().await.unwrap();
}

/// A paste is one `send_bytes_hex` call, however long, and every byte of it
/// reaches the pane in order (ov-349).
///
/// tmux refuses a command past 1,000 arguments (3.7) or 16 KB of them (3.4),
/// and one `send-keys -H` per paste put a byte in each argument: a paste
/// over 996 bytes was dropped whole, locally and over ssh alike, with only
/// `command too long` on a stderr nobody reads. Sized past both limits, and
/// carrying what a paste carries: newlines, the bracket, and UTF-8.
#[tokio::test]
async fn a_long_paste_reaches_the_pane_whole() {
    let Some(srv) = live_server("a_long_paste_reaches_the_pane_whole").await else { return };
    let got = std::env::temp_dir().join(format!("fc-paste-{}", Uuid::now_v7().simple()));
    // Raw, so the line discipline neither edits nor echoes: what cat writes
    // is exactly what the pane was sent.
    let command = format!("sh -c 'stty raw -echo; exec cat > {}'", got.display());
    let win = srv
        .create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "paste", "/tmp", &command)
        .await
        .unwrap();
    until("the recorder to start", || async { got.exists() }).await;

    let mut paste = b"\x1b[200~".to_vec();
    let mut line = 0;
    while paste.len() < 6_000 {
        paste.extend(format!("line {line}: a \u{201c}quoted\u{201d} word \u{2014} and more\n").into_bytes());
        line += 1;
    }
    paste.extend(b"\x1b[201~");
    let hex: String = paste.iter().map(|b| format!("{b:02x}")).collect();

    srv.send_bytes_hex(&win.pane_id, &hex).await.expect("the paste is sent");
    until("every pasted byte", || async {
        std::fs::metadata(&got).map(|m| m.len() as usize >= paste.len()).unwrap_or(false)
    })
    .await;
    assert!(std::fs::read(&got).unwrap() == paste, "the pane got other bytes than were pasted");

    let _ = std::fs::remove_file(&got);
    srv.kill_server().await.unwrap();
}

/// A runner with no tmux server spends nothing asking whether there is one.
///
/// The inventory is read every second, and with no terminal open there is no
/// server to read. The first read learns that, from tmux's own error, which
/// names the socket it could not find; after it, the socket is a stat. The read
/// that follows the server's birth spawns again, so a pane is never missed.
#[tokio::test]
async fn no_server_is_asked_about_once() {
    let Some(srv) = live_server("no_server_is_asked_about_once").await else { return };
    for _ in 0..5 {
        let read = srv.read_panes().await.expect("no server is not an error");
        assert!(read.panes.is_empty() && read.unfinished.is_empty());
    }
    // The first failure is not believed on its own (it may be a server that bound
    // a moment later), so the second asks too; after that, nothing.
    assert!(srv.calls_total() <= 2, "five reads of nothing, two spawns at most: {:?}", srv.call_counts());

    srv.create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "x", "/tmp", "sleep 30").await.unwrap();
    let before = srv.calls_total();
    assert_eq!(srv.read_panes().await.unwrap().panes.len(), 1, "the new server's pane is read at once");
    assert!(srv.calls_total() > before, "a socket that appeared is asked");

    // The server unlinks its socket a moment after it answers `kill-server`, and
    // a read in that gap is refused rather than not found, which says nothing
    // about a missing socket. So wait for the first read that learns it.
    srv.kill_server().await.unwrap();
    let mut learned = false;
    for _ in 0..40 {
        let before = srv.calls_total();
        assert!(srv.read_panes().await.unwrap().panes.is_empty());
        if srv.calls_total() == before {
            learned = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(learned, "a server that went away is learned: {:?}", srv.call_counts());
    let settled = srv.calls_total();
    for _ in 0..3 {
        assert!(srv.read_panes().await.unwrap().panes.is_empty());
    }
    assert_eq!(srv.calls_total(), settled, "and then not asked again");

    // A server that comes back binds a new socket over the stale one, and is
    // read on the very next call.
    srv.create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "y", "/tmp", "sleep 30").await.unwrap();
    assert_eq!(srv.read_panes().await.unwrap().panes.len(), 1, "a restarted server is not skipped");
}

/// One `list-panes` carries the panes and the unfinished opens, and the stamp
/// of each pane moves when its screen does.
#[tokio::test]
async fn one_read_carries_the_panes_the_stamp_and_the_unfinished_opens() {
    let Some(srv) = live_server("one_read_carries_the_panes_the_stamp_and_the_unfinished_opens").await else {
        return;
    };
    let terminal = Uuid::now_v7();
    let win = srv.create_terminal_window(Uuid::now_v7(), terminal, "x", "/tmp", "cat").await.unwrap();
    // An open that never got its tag: marked, in our session, no terminal id.
    srv.run(&[
        "new-window",
        "-d",
        "-t",
        "farcooler:",
        &farcooler_tmux::windows::marked("sleep 30", Uuid::now_v7()),
    ])
    .await
    .unwrap();

    let before = srv.calls_total();
    let read = srv.read_panes().await.unwrap();
    assert_eq!(srv.calls_total(), before + 1, "one spawn for both answers");
    assert_eq!(read.panes.len(), 1);
    assert_eq!(read.unfinished.len(), 1, "the marked, untagged pane is the unfinished open");
    let first = read.panes[0].stamp;
    assert!(first.activity > 0, "tmux reported the window's activity");
    assert!(first.pid > 0);

    srv.send_keys(&win.pane_id, "hello").await.unwrap();
    let read = srv.read_panes().await.unwrap();
    let now = read.panes[0].stamp;
    assert!(!now.unchanged_since(&first), "typing into the pane moved its stamp: {first:?} then {now:?}");
    assert!(now.unchanged_since(&now), "a stamp is unchanged from itself");
}

/// A server that starts while a read of the empty socket is in flight is read
/// from then on. The skip once believed the socket it found AFTER the error,
/// which in a race is the new server's own, and then read nothing for as long as
/// that server lived: every terminal derived Lost (review, 7 of 800).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_born_during_a_read_is_never_skipped() {
    let Some(srv) = live_server("a_server_born_during_a_read_is_never_skipped").await else { return };
    for i in 0..400u64 {
        srv.kill_server().await.unwrap();
        // No settling reads: the racing read below must be the first to fail, or
        // the socket is already believed silent and that read never spawns.
        let reader: TmuxServer = (*srv).clone();
        // Swept across the window where the client has failed and the server is
        // about to bind: with trust forced on, rounds 4 to 8 ms in got stuck.
        let delay = std::time::Duration::from_micros(i * 53 % 10_000);
        let racing = tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            reader.read_panes().await
        });
        srv.create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "x", "/tmp", "sleep 30").await.unwrap();
        racing.await.unwrap().unwrap();
        for _ in 0..2 {
            assert_eq!(srv.read_panes().await.unwrap().panes.len(), 1, "round {i}: a running server read as empty");
        }
    }
}

/// `window_activity` alone moves when a program draws and puts the cursor back:
/// a dialog painted over a full-screen program and restored. The cache's whole
/// case for such a screen rests on it, and on tmux 3.4 as on 3.7.
#[tokio::test]
async fn drawing_and_restoring_the_cursor_still_moves_the_windows_activity() {
    let Some(srv) = live_server("drawing_and_restoring_the_cursor_still_moves_the_windows_activity").await else {
        return;
    };
    // Parks the cursor, waits out a whole second, then saves it, draws and
    // restores it.
    let program = r"printf '[10;10H'; sleep 2.5; printf '7[5;5HDIALOG8'; sleep 30";
    srv.create_terminal_window(Uuid::now_v7(), Uuid::now_v7(), "x", "/tmp", &format!("sh -c \"{program}\""))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let first = srv.read_panes().await.unwrap().panes[0].stamp;
    assert_eq!(first.cursor, (9, 9), "the program parked the cursor");

    let mut moved = None;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let now = srv.read_panes().await.unwrap().panes[0].stamp;
        if now.activity != first.activity {
            moved = Some(now);
            break;
        }
    }
    let now = moved.expect("tmux never reported the draw as activity");
    assert!(now.activity > first.activity);
    assert_eq!((now.cursor, now.history, now.pid), (first.cursor, first.history, first.pid), "only the activity moved");
    assert!(!now.unchanged_since(&first));
}

//! ov-1287: a cancelled native owner can end by itself while the runner ends it, as a reviewer whose Stop hook loses
//! its answer does. Quiescence gives one answer whichever is observed first: the owner's exit or the runner's own stop.
use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::enqueue};
/// An owner over a pane running `command`, recorded as run_native records one.
async fn owned_pane(command: &str) -> (tempfile::TempDir, Runtime, Host, NativeOwner) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
    let window = host.0.create_terminal_window(id, id, "quiesce fixture", dir.path().to_str().unwrap(), command).await.unwrap();
    let pane = host.0.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding, terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone() };
    (dir, runtime, host, owner)
}
fn on_tty(owner: &NativeOwner) -> Vec<i32> {
    snapshot().unwrap().under_tty(owner.tty.trim_start_matches("/dev/"))
}
async fn closed(host: &Host, owner: &NativeOwner) -> bool {
    host.0.pane_life(&owner.pane, owner.terminal).await.unwrap() == farcooler_tmux::PaneLife::Gone
}

#[tokio::test]
async fn the_runner_stopping_a_live_owner_first_ends_and_closes_it() {
    let (_dir, runtime, host, owner) = owned_pane("/bin/sleep 30").await;
    let pids = on_tty(&owner);
    assert!(!pids.is_empty());
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(pids.iter().all(|&pid| gone(pid)));
    assert!(closed(&host, &owner).await);
}

#[tokio::test]
async fn an_owner_that_exited_before_quiescence_looked_is_quiescent() {
    let (_dir, runtime, host, owner) = owned_pane("/usr/bin/true").await;
    assert!(super::super::wait_until(tokio::time::Instant::now() + Duration::from_secs(10), Duration::from_millis(10),
        || on_tty(&owner).is_empty()).await);
    for _ in 0..500 {
        if host.0.pane_life(&owner.pane, owner.terminal).await.unwrap() != farcooler_tmux::PaneLife::Live { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_ne!(host.0.pane_life(&owner.pane, owner.terminal).await.unwrap(), farcooler_tmux::PaneLife::Live);
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(closed(&host, &owner).await);
}

#[tokio::test]
async fn a_dead_panes_reassigned_tty_cannot_end_another_native_pane() {
    let (_owner_dir, runtime, host, mut owner) = owned_pane("/bin/sleep 30").await;
    let pids = on_tty(&owner);
    assert!(!pids.is_empty());
    for pid in pids { unsafe { libc::kill(pid, libc::SIGKILL); } }
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.0.pane_life(&owner.pane, owner.terminal).await.unwrap() != farcooler_tmux::PaneLife::Dead {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();

    let (_other_dir, _other_runtime, other_host, other) = owned_pane("/bin/sleep 30").await;
    // Model the tty number being handed out again regardless of which PTY
    // numbers concurrent tests have open when the kernel allocates ours.
    owner.tty.clone_from(&other.tty);
    let result = runtime.quiesce_native(&owner).await;
    assert_eq!(other_host.0.pane_life(&other.pane, other.terminal).await.unwrap(),
        farcooler_tmux::PaneLife::Live, "quiescing a dead owner must leave the other pane running");
    assert!(result.is_err(), "a busy former tty cannot prove quiescence");
}

#[tokio::test]
async fn an_owner_that_exits_after_its_pane_was_seen_live_is_quiescent() {
    let (_dir, runtime, host, owner) = owned_pane("/bin/sleep 30").await;
    let pids = on_tty(&owner);
    assert!(!pids.is_empty());
    let tty = owner.clone();
    AFTER_LIVE.with(|h| *h.borrow_mut() = Some(Box::new(move || {
        for &pid in &pids { unsafe { libc::kill(pid, libc::SIGKILL); } }
        for _ in 0..500 {
            if on_tty(&tty).is_empty() { return; }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the owner never left its tty");
    })));
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(AFTER_LIVE.with(|h| h.borrow().is_none()), "the exit landed between the two observations");
    assert!(closed(&host, &owner).await);
}

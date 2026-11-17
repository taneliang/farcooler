//! ov-1287, ov-1330: a native owner is ended through its hold, whichever of its own exit and the runner's end comes
//! first, and nothing the runner finds by tty is ever signalled.
use super::*;
use super::super::{coding_tests::{configured, admitted}, tests::enqueue};
/// An owner over a pane running `script` under its hold, recorded as run_native records one.
async fn owned_pane(script: &str, ready: bool) -> (tempfile::TempDir, Runtime, Host, NativeOwner) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
    runtime.pump(2).await.unwrap();
    let binding = admitted(&runtime).await;
    let host = Host(server(&runtime.root, &binding)); let id = terminal(&binding);
    let held = runtime.held(&binding, script).unwrap();
    let window = host.0.create_terminal_window(id, id, "quiesce fixture", dir.path().to_str().unwrap(), &held).await.unwrap();
    if ready { assert!(runtime.hold_ready(&binding).await); }
    let pane = host.0.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let owner = NativeOwner { binding, terminal: id, pane: pane.pane_id.clone(), tty: pane.tty.clone(), hold: true, supervisor: None };
    (dir, runtime, host, owner)
}
fn on_tty(tty: &str) -> Vec<i32> {
    snapshot().unwrap().under_tty(tty.trim_start_matches("/dev/"))
}
fn alive(pid: i32) -> bool { unsafe { libc::kill(pid, 0) == 0 } }
async fn closed(host: &Host, owner: &NativeOwner) -> bool {
    host.0.pane_life(&owner.pane, owner.terminal).await.unwrap() == farcooler_tmux::PaneLife::Gone
}
async fn wait_not_live(host: &Host, owner: &NativeOwner) {
    for _ in 0..500 {
        if host.0.pane_life(&owner.pane, owner.terminal).await.unwrap() != farcooler_tmux::PaneLife::Live { return; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the owner's pane stayed live");
}
/// A pid of the agent's own, written by the script into `name` under the fixture dir.
async fn written_pid(dir: &tempfile::TempDir, name: &str) -> i32 {
    let path = dir.path().join(name);
    for _ in 0..500 {
        if let Ok(text) = std::fs::read_to_string(&path) && let Ok(pid) = text.trim().parse() { return pid; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the fixture never wrote {name}");
}

#[tokio::test]
async fn the_runner_stopping_a_live_owner_first_ends_its_whole_group_and_closes_it() {
    // A background job in the agent's group, and the agent itself; the barrier holds both until released.
    let (dir, runtime, host, owner) = owned_pane("read -r go; /bin/sleep 30 & echo $! > job; exec /bin/sleep 30", true).await;
    host.0.send_keys(&owner.pane, "\n").await.unwrap();
    let job = written_pid(&dir, "job").await;
    assert!(alive(job));
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(!alive(job), "the agent's background job ended with it");
    assert!(!runtime.native_hold_socket(&owner.binding).exists(), "the hold removes its socket once its group is empty");
    assert!(closed(&host, &owner).await);
}

#[tokio::test]
async fn an_owner_that_exited_before_quiescence_looked_is_quiescent() {
    let (_dir, runtime, host, owner) = owned_pane("exec /usr/bin/true", false).await;
    wait_not_live(&host, &owner).await;
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(closed(&host, &owner).await);
}

#[tokio::test]
async fn an_owner_that_exits_while_the_runner_ends_it_is_quiescent() {
    for delay in [0, 50, 150] {
        let (_dir, runtime, host, owner) = owned_pane("exec /bin/sleep 0.1", true).await;
        tokio::time::sleep(Duration::from_millis(delay)).await;
        runtime.quiesce_native(&owner).await.unwrap();
        assert!(closed(&host, &owner).await, "after {delay} ms");
    }
}

/// `end` gives the agent a second to end its own work, then kills what ignores it.
#[tokio::test]
async fn an_agent_that_ignores_sigterm_is_killed_after_its_grace() {
    let (dir, runtime, host, owner) = owned_pane("trap '' TERM; /bin/sleep 30 & echo $! > job; wait", true).await;
    let job = written_pid(&dir, "job").await;
    let started = std::time::Instant::now();
    runtime.quiesce_native(&owner).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1), "SIGTERM was ignored, so only the kill after the grace ends it");
    assert!(!alive(job));
    assert!(closed(&host, &owner).await);
}

/// The six findings of ov-1319 and ov-1325 were all one shape: the owner's tty (or a pid on it) came to belong to
/// someone else, and the runner signalled whatever it found there. Here the retained owner names a tty a foreign
/// session holds, after the owner itself has ended. Quiescence neither signals the foreign process nor mistakes it
/// for the owner.
#[tokio::test]
async fn a_tty_reused_by_a_foreign_session_is_never_signalled() {
    let (_dir, runtime, host, mut owner) = owned_pane("exec /usr/bin/true", false).await;
    wait_not_live(&host, &owner).await;
    let foreign = TmuxServer::in_dir("ov-1330-foreign", Uuid::now_v7(), &farcooler_core::scratch::dir("ov-1330-foreign"));
    let other = Uuid::now_v7();
    let window = foreign.create_terminal_window(other, other, "foreign", "/", "exec /bin/sleep 30").await.unwrap();
    let pane = foreign.list_tagged_panes().await.unwrap().into_iter().find(|p| p.pane_id == window.pane_id).unwrap();
    let mut victims = Vec::new();
    for _ in 0..500 { victims = on_tty(&pane.tty); if !victims.is_empty() { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
    assert!(!victims.is_empty());
    owner.tty = pane.tty.clone();
    runtime.quiesce_native(&owner).await.unwrap();
    // A hold that was killed leaves its socket behind and answers nothing: the runner refuses. It still signals nothing.
    let stale = std::os::unix::net::UnixListener::bind(runtime.native_hold_socket(&owner.binding)).unwrap();
    drop(stale);
    assert!(runtime.quiesce_native(&owner).await.is_err());
    std::fs::remove_file(runtime.native_hold_socket(&owner.binding)).unwrap();
    assert!(victims.iter().all(|&pid| alive(pid)), "the foreign session was signalled");
    assert_eq!(foreign.pane_life(&pane.pane_id, other).await.unwrap(), farcooler_tmux::PaneLife::Live);
    farcooler_tmux::reap_server(foreign.socket());
}

/// ov-1330 review (fix round 2): a hold killed while its agent's command, in a session of its own, keeps writing. The
/// pane is gone and the command holds no tty, so nothing the runner can observe shows it working; only the hold could
/// have proved it ended, and it cannot answer. Settlement is refused while the command writes.
#[tokio::test]
async fn a_killed_hold_refuses_settlement_while_a_detached_command_writes() {
    let (dir, runtime, host, owner) = owned_pane(
        "echo $PPID > hold; echo $$ > agent; perl -MPOSIX -e 'setsid; open(STDIN, q(</dev/null)); open(STDOUT, q(>/dev/null)); \
         open(STDERR, q(>/dev/null)); while (1) { open(F, q(>>work)); print F 1; close F; select(undef, undef, undef, 0.02) }' & \
         echo $! > writer; exec /bin/sleep 30", true).await;
    let (held, writer) = (written_pid(&dir, "hold").await, written_pid(&dir, "writer").await);
    struct Reap(i32);
    impl Drop for Reap { fn drop(&mut self) { unsafe { libc::kill(self.0, libc::SIGKILL); } } }
    let (_writer, _agent) = (Reap(writer), Reap(written_pid(&dir, "agent").await));
    // The test kills the hold (a pid its own agent reported); the runner never does.
    unsafe { libc::kill(held, libc::SIGKILL); }
    for _ in 0..500 { if !alive(held) { break; } tokio::time::sleep(Duration::from_millis(10)).await; }
    let size = || std::fs::metadata(dir.path().join("work")).map_or(0, |m| m.len());
    let at = size();
    assert!(runtime.quiesce_native(&owner).await.is_err(), "a hold that cannot answer proves nothing ended");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(size() > at, "and the detached command was still writing");
    assert!(runtime.native_hold_socket(&owner.binding).exists(), "the killed hold's socket is still there");
    assert!(alive(writer), "the runner signalled nothing");
    farcooler_tmux::reap_server(host.0.socket());
}

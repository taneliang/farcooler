use super::*;
use farcooler_store::engine_cutover::transfer;

/// A pane whose root is 100 under the tmux server 50: its shell 101, and the
/// CLI 102 that shell ran. 200 is another pane's root under the same server,
/// and 300 an orphan its shell left to pid 1.
fn table() -> (HashMap<i32, Proc>, HashSet<i32>) {
    let row = |ppid| Proc { ppid, tty: "ttys001".into() };
    let procs = HashMap::from([(50, row(1)), (100, row(50)), (101, row(100)), (102, row(101)), (200, row(50)), (201, row(200)), (300, row(1))]);
    (procs, HashSet::from([50]))
}
fn pinned(pane_pid: i32) -> Delegate {
    Delegate { attempt: "a-1".into(), terminal: Uuid::nil(), workspace: Uuid::nil(), pane_pid, pane_start: 7 }
}

#[test]
fn only_a_process_inside_the_pinned_pane_is_the_orchestrator() {
    let (procs, servers) = table();
    let started = |pid: i32| (pid == 100).then_some(7);
    for caller in [100, 101, 102] {
        assert!(within(caller, &procs, &servers, &pinned(100), &started), "{caller}: in the pane");
    }
    for caller in [201, 200, 300, 50, 1, 0, -1, 999] {
        assert!(!within(caller, &procs, &servers, &pinned(100), &started), "{caller}: another pane, an orphan, the server, nobody");
    }
    let restarted = |pid: i32| (pid == 100).then_some(8);
    assert!(!within(102, &procs, &servers, &pinned(100), &restarted), "a pane root started since is another process");
    assert!(!within(102, &procs, &servers, &pinned(50), &|_| Some(7)), "the walk stops at the tmux server, never past it");
}

#[test]
fn a_delegate_is_read_only_for_the_attempt_in_hand_and_only_for_its_pane() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let db = crate::cutover_admin::database(root);
    farcooler_store::Store::open(&db).unwrap();
    // This test's own process stands in for the pane root.
    let me = std::process::id() as i32;
    let start = crate::claude_registry::process_incarnation(me).unwrap();
    let delegate = Delegate { attempt: "a-1".into(), terminal: Uuid::now_v7(), workspace: Uuid::now_v7(), pane_pid: me, pane_start: start };
    write(root, &delegate).unwrap();
    assert_eq!(admits(root, me), None, "nothing is held: no attempt is in hand");
    transfer::hold(&db, "a-2").unwrap();
    assert_eq!(admits(root, me), None, "a pane pinned for another attempt");
    write(root, &Delegate { attempt: "a-2".into(), ..delegate.clone() }).unwrap();
    assert_eq!(admits(root, me).map(|d| d.terminal), Some(delegate.terminal), "the pinned pane, for the attempt in hand");
    write(root, &Delegate { attempt: "a-2".into(), pane_start: start + 1, ..delegate.clone() }).unwrap();
    assert_eq!(admits(root, me), None, "a pid since reused");
    write(root, &Delegate { attempt: "a-2".into(), pane_pid: 1, ..delegate.clone() }).unwrap();
    assert_eq!(admits(root, me), None, "a pane this process isn't in");
    forget(root).unwrap();
    forget(root).unwrap();
    assert_eq!(admits(root, me), None);
}

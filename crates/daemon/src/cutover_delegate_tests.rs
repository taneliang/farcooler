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
    Delegate { attempt: "a-1".into(), terminal: Uuid::nil(), workspace: Uuid::nil(), pane_pid, pane_start: 7, grant: Uuid::nil() }
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

/// The owner's orchestrator in a scratch runner's main workspace, by the
/// database: the pane, its workspace, and the database.
async fn the_owners_orchestrator() -> (crate::test_support::ScratchDir, std::sync::Arc<crate::service::Service>, Uuid, Uuid) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap()[0].id;
    let pane = svc.store.create_terminal_for_test(worktree, workspace);
    svc.store.set_terminal_role(pane, farcooler_store::models::TerminalRole::Orchestrator).unwrap();
    (dir, svc, pane, workspace)
}

#[tokio::test]
async fn a_delegate_is_read_only_for_the_attempt_in_hand_and_only_for_its_pane() {
    let (_dir, svc, terminal, workspace) = the_owners_orchestrator().await;
    let root = svc.root_dir();
    let db = crate::cutover_admin::database(root);
    // This test's own process stands in for the pane root.
    let me = std::process::id() as i32;
    let start = crate::claude_registry::process_incarnation(me).unwrap();
    let delegate = Delegate { attempt: "a-1".into(), terminal, workspace, pane_pid: me, pane_start: start, grant: Uuid::now_v7() };
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
    write(root, &Delegate { attempt: "a-2".into(), terminal: Uuid::now_v7(), ..delegate.clone() }).unwrap();
    assert_eq!(admits(root, me), None, "a pane the database doesn't know");
    forget(root).unwrap();
    forget(root).unwrap();
    assert_eq!(admits(root, me), None);
}

/// Each recovery step asks the database again: the role, the login and the
/// lead, as the live Prepare did.
#[tokio::test]
async fn every_recovery_step_rechecks_role_login_and_lead() {
    let (_dir, svc, terminal, workspace) = the_owners_orchestrator().await;
    let db = crate::cutover_admin::database(svc.root_dir());
    let pinned = Delegate { attempt: "a-1".into(), terminal, workspace, pane_pid: 9, pane_start: 1, grant: Uuid::nil() };
    assert!(still_the_owners(&db, &pinned), "the owner's orchestrator");
    assert!(!still_the_owners(&db, &Delegate { workspace: Uuid::now_v7(), ..pinned.clone() }), "another workspace's");

    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
    assert!(!still_the_owners(&db, &pinned), "Bob leads the workspace now");
    svc.store.set_workspace_lead(workspace, None).unwrap();
    assert!(still_the_owners(&db, &pinned));

    let login_id = svc.store.ensure_member_login("bob").unwrap();
    svc.store.set_terminal_login(terminal, &farcooler_store::claude_logins::TerminalLogin { handle: "bob".into(), login_id, config_dir: "/r/claude/bob".into(), shared: false }).unwrap();
    assert!(!still_the_owners(&db, &pinned), "the pane is on Bob's login");

    let (_dir, svc, terminal, workspace) = the_owners_orchestrator().await;
    let db = crate::cutover_admin::database(svc.root_dir());
    let pinned = Delegate { terminal, workspace, ..pinned };
    svc.store.set_terminal_role(terminal, farcooler_store::models::TerminalRole::Agent).unwrap();
    assert!(!still_the_owners(&db, &pinned), "no longer the orchestrator");
    assert!(!still_the_owners(&svc.root_dir().join("nowhere.sqlite"), &pinned), "a database that can't be read");
}

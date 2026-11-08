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

/// The one card `granted` made on `workspace`.
fn task_of(svc: &crate::service::Service, workspace: Uuid) -> Uuid {
    let repository = svc.store.get_workspace(workspace).unwrap().repository_id;
    let hits = svc.store.search_notes(repository, crate::cutover_grant::ALLOW, Some(farcooler_store::models::NoteKind::Answer)).unwrap();
    hits[0].note.task_id
}

/// The owner's "Allow cutover once" on a card of `workspace`, said from the
/// app, and spent on `attempt` as the live Prepare spends it.
fn granted(svc: &crate::service::Service, workspace: Uuid, attempt: &str) -> Uuid {
    use crate::cutover_grant::{ALLOW, SAID_FROM};
    use farcooler_store::models::{Actor, NoteKind};
    let task = svc.store.create_task(workspace, "Data: the engine cutover", Actor::Manager).unwrap().id;
    svc.store.add_note(task, NoteKind::Question, Actor::Manager, "Run it once?", serde_json::json!({ "options": [ALLOW] })).unwrap();
    svc.store.add_note(task, NoteKind::Answer, Actor::User, ALLOW, serde_json::json!({ SAID_FROM: "app" })).unwrap();
    let now = crate::step_up_rpc::now_ms();
    let grant = crate::cutover_grant::required(&svc.store, workspace, now).unwrap();
    crate::cutover_grant::spend(&svc.store, &grant, attempt, now).unwrap();
    grant.answer
}

#[tokio::test]
async fn a_delegate_is_read_only_for_the_attempt_in_hand_and_only_for_its_pane() {
    let (_dir, svc, terminal, workspace) = the_owners_orchestrator().await;
    let root = svc.root_dir();
    let db = crate::cutover_admin::database(root);
    // This test's own process stands in for the pane root.
    let me = std::process::id() as i32;
    let start = crate::claude_registry::process_incarnation(me).unwrap();
    let grant = granted(&svc, workspace, "a-2");
    let delegate = Delegate { attempt: "a-1".into(), terminal, workspace, pane_pid: me, pane_start: start, grant };
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
    write(root, &Delegate { attempt: "a-2".into(), grant: Uuid::now_v7(), ..delegate.clone() }).unwrap();
    assert_eq!(admits(root, me), None, "a grant the database doesn't know");
    // M4: the pinned pane, its grant past a day mid-recovery. Refused, and
    // told why: only the owner finishes it now.
    write(root, &Delegate { attempt: "a-2".into(), ..delegate.clone() }).unwrap();
    let (now, max) = (crate::step_up_rpc::now_ms(), crate::cutover_grant::MAX_AGE_MS);
    assert!(admits_at(root, me, now + max - 60_000).is_some() && !lapsed_at(root, me, now + max - 60_000), "a day less a minute on");
    assert_eq!(admits_at(root, me, now + max + 60_000), None, "a day and a minute on");
    assert!(lapsed_at(root, me, now + max + 60_000), "lapsed, and said so");
    assert!(!lapsed_at(root, 1, now + max + 60_000), "a caller outside the pane hears nothing of it");
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

/// Each recovery step asks the database again about the grant, as the live
/// Prepare did: under a day old, and spent on this attempt and no other.
#[tokio::test]
async fn every_recovery_step_rechecks_the_grants_age_and_spend() {
    let (_dir, svc, terminal, workspace) = the_owners_orchestrator().await;
    let db = crate::cutover_admin::database(svc.root_dir());
    let grant = granted(&svc, workspace, "a-1");
    let now = crate::step_up_rpc::now_ms();
    let pinned = Delegate { attempt: "a-1".into(), terminal, workspace, pane_pid: 9, pane_start: 1, grant };
    assert!(grant_holds(&db, &pinned, now), "the owner's grant, spent on this attempt");
    let max = crate::cutover_grant::MAX_AGE_MS;
    assert!(grant_holds(&db, &pinned, now + max - 60_000), "a day less a minute on");
    assert!(!grant_holds(&db, &pinned, now + max + 60_000), "a day and a minute on: lapsed mid-recovery");
    assert!(!grant_holds(&db, &Delegate { attempt: "a-2".into(), ..pinned.clone() }, now), "spent on another attempt");
    assert!(!grant_holds(&db, &Delegate { grant: Uuid::now_v7(), ..pinned.clone() }, now), "no such answer");

    // Spent again on another attempt (a restored file): it holds for neither.
    let card = task_of(&svc, workspace);
    svc.store.add_note(card, farcooler_store::models::NoteKind::Progress, farcooler_store::models::Actor::Runner, "spent again",
        serde_json::json!({ "cutover_grant": { "answer": grant.to_string(), "attempt": "a-3" } })).unwrap();
    assert!(!grant_holds(&db, &pinned, now), "a grant spent twice");
    assert!(!grant_holds(&svc.root_dir().join("nowhere.sqlite"), &pinned, now), "a database that can't be read");
}

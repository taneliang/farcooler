use farcooler_core::DomainError;
use farcooler_protocol::v1::TerminalIntent;
use uuid::Uuid;

use super::Launch;
use crate::models::Terminal;
use crate::store::Store;

fn terminal(s: &Store) -> Terminal {
    let host = Uuid::now_v7();
    let root = s.create_repository_root(host, "/wt/launch", 1_000).unwrap();
    let repo = s.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let ws = s.create_worktree(repo.id, "feature/x", "/wt/launch", false).unwrap();
    s.create_terminal(ws.id, "Agent", "claude", TerminalIntent::Running, 120, 40).unwrap()
}

#[test]
fn a_new_launch_retires_the_one_before_it() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    assert_eq!(s.current_launch(t.id).unwrap(), None, "a pane nothing has launched into has no launch");

    let (t1, first) = s.begin_launch(t.id, t.resource_version).unwrap();
    assert_eq!(first.prior, None);
    assert!(t1.resource_version > t.resource_version, "a launch moves the version, so a racing restart loses");
    assert_eq!(s.terminal_for_launch(first.id).unwrap(), Some(t.id));

    let (_, second) = s.begin_launch(t.id, t1.resource_version).unwrap();
    assert_eq!(second.prior, Some(first.id));
    assert_ne!(second.id, first.id);
    assert_eq!(s.terminal_for_launch(first.id).unwrap(), None, "the restart retired the first launch");
    assert_eq!(s.terminal_for_launch(second.id).unwrap(), Some(t.id));
    assert_eq!(s.current_launch(t.id).unwrap(), Some(second.id));
}

#[test]
fn of_two_restarts_from_one_version_only_one_launches() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (_, won) = s.begin_launch(t.id, t.resource_version).unwrap();
    let lost = s.begin_launch(t.id, t.resource_version);
    assert!(matches!(lost, Err(DomainError::ResourceConflict)), "{lost:?}");
    assert_eq!(s.current_launch(t.id).unwrap(), Some(won.id), "the loser wrote nothing");
    assert!(matches!(s.begin_launch(Uuid::now_v7(), 1), Err(DomainError::NotFound)));
}

#[test]
fn a_failed_spawn_puts_back_the_launch_it_replaced() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (t1, first) = s.begin_launch(t.id, t.resource_version).unwrap();
    let (_, failed) = s.begin_launch(t.id, t1.resource_version).unwrap();
    assert!(s.roll_back_launch(t.id, &failed).unwrap());
    assert_eq!(s.current_launch(t.id).unwrap(), Some(first.id));
    assert_eq!(s.terminal_for_launch(failed.id).unwrap(), None, "the failed launch reaches nothing");
}

#[test]
fn a_failed_first_launch_leaves_no_launch() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (_, failed) = s.begin_launch(t.id, t.resource_version).unwrap();
    assert!(s.roll_back_launch(t.id, &failed).unwrap());
    assert_eq!(s.current_launch(t.id).unwrap(), None);
}

/// The rollback a failed restart does must never undo a restart that came
/// after it and won.
#[test]
fn a_late_rollback_leaves_a_newer_launch_alone() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (t1, first) = s.begin_launch(t.id, t.resource_version).unwrap();
    let (t2, failing) = s.begin_launch(t.id, t1.resource_version).unwrap();
    let (_, newer) = s.begin_launch(t.id, t2.resource_version).unwrap();

    assert!(!s.roll_back_launch(t.id, &failing).unwrap(), "nothing to roll back: a newer launch won");
    assert_eq!(s.current_launch(t.id).unwrap(), Some(newer.id));
    assert_eq!(s.terminal_for_launch(first.id).unwrap(), None, "the launch before the failed one stays retired");

    // The same with no prior: a rollback that would delete the row.
    let fresh = Launch { id: Uuid::now_v7(), prior: None };
    assert!(!s.roll_back_launch(t.id, &fresh).unwrap());
    assert_eq!(s.current_launch(t.id).unwrap(), Some(newer.id));
}

#[test]
fn a_deleted_panes_launch_goes_with_it() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (t1, launch) = s.begin_launch(t.id, t.resource_version).unwrap();
    s.delete_terminal(t.id, t1.resource_version).unwrap();
    assert_eq!(s.terminal_for_launch(launch.id).unwrap(), None);
}

/// Durable: a daemon that restarts reads the current launch straight back, so
/// a hook from a launch retired before the restart is still refused after it.
#[test]
fn the_current_launch_survives_a_reopen() {
    let dir = farcooler_core::scratch::dir("pane-launch");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.db");
    let (id, first, second) = {
        let s = Store::open(&path).unwrap();
        let t = terminal(&s);
        let (t1, first) = s.begin_launch(t.id, t.resource_version).unwrap();
        let (_, second) = s.begin_launch(t.id, t1.resource_version).unwrap();
        (t.id, first.id, second.id)
    };
    let s = Store::open(&path).unwrap();
    assert_eq!(s.terminal_for_launch(second).unwrap(), Some(id));
    assert_eq!(s.terminal_for_launch(first).unwrap(), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A pane deleted by cascade (its worktree going) leaves its `meta` row
/// behind; that row must route nothing.
#[test]
fn a_launch_whose_pane_went_by_cascade_routes_nothing() {
    let s = Store::open_in_memory().unwrap();
    let t = terminal(&s);
    let (_, launch) = s.begin_launch(t.id, t.resource_version).unwrap();
    s.conn().execute("DELETE FROM terminals WHERE id = ?1", [t.id.as_bytes().as_slice()]).unwrap();
    assert_eq!(s.current_launch(t.id).unwrap(), Some(launch.id), "the row is still there");
    assert_eq!(s.terminal_for_launch(launch.id).unwrap(), None);
}

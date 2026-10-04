use super::*;

use crate::models::{Actor, Task};

const NOW: i64 = 10_000_000_000;

/// A store with one board and `n` tasks on it.
fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn open(task: &Task, ms: i64) -> ReadsDelta {
    ReadsDelta { opened: vec![(task.id, ms)], ..ReadsDelta::default() }
}

fn floor(ms: i64) -> ReadsDelta {
    ReadsDelta { floor_ms: Some(ms), ..ReadsDelta::default() }
}

fn marks(reads: &BoardReads) -> Vec<i64> {
    reads.opened.iter().map(|(_, ms)| *ms).collect()
}

/// Two devices write out of order and the later time wins, whichever arrives
/// last: a mark never goes back.
#[test]
fn marks_merge_by_max() {
    let (store, main, t) = board(1);
    let base = NOW - FIRST_LOOK_MS;
    let (reads, changed) = store.merge_board_reads(main, &open(&t[0], base + 500), NOW).unwrap();
    assert!(changed);
    assert_eq!(reads.opened, vec![(t[0].id, base + 500)]);
    // The other device's older open arrives second.
    let (reads, changed) = store.merge_board_reads(main, &open(&t[0], base + 100), NOW).unwrap();
    assert!(!changed, "an older mark changes nothing");
    assert_eq!(reads.opened, vec![(t[0].id, base + 500)]);
    // And a newer one still raises it.
    let (reads, _) = store.merge_board_reads(main, &open(&t[0], base + 900), NOW).unwrap();
    assert_eq!(reads.opened, vec![(t[0].id, base + 900)]);
}

#[test]
fn the_floor_only_rises() {
    let (store, main, _) = board(0);
    let first = store.board_reads(main, NOW).unwrap().floor_ms;
    let (reads, _) = store.merge_board_reads(main, &floor(first + 1000), NOW).unwrap();
    assert_eq!(reads.floor_ms, first + 1000);
    let (reads, changed) = store.merge_board_reads(main, &floor(first + 10), NOW).unwrap();
    assert!(!changed);
    assert_eq!(reads.floor_ms, first + 1000, "a lower floor is a no-op");
}

/// Only the runner's own first-look default is replaced by a seed; one a
/// device has written is raised by it, never lowered.
#[test]
fn a_seed_replaces_only_the_implicit_floor() {
    let (store, main, _) = board(0);
    let implicit = store.board_reads(main, NOW).unwrap().floor_ms;
    assert_eq!(implicit, NOW - FIRST_LOOK_MS);
    let seed = ReadsDelta { floor_ms: Some(implicit - 5000), seeds_floor: true, ..ReadsDelta::default() };
    let (reads, _) = store.merge_board_reads(main, &seed, NOW).unwrap();
    assert_eq!(reads.floor_ms, implicit - 5000, "the first seed wins over the default, even lower");
    // A second seed (another Mac) is a plain raise.
    let lower = ReadsDelta { floor_ms: Some(implicit - 9000), seeds_floor: true, ..ReadsDelta::default() };
    let (reads, _) = store.merge_board_reads(main, &lower, NOW).unwrap();
    assert_eq!(reads.floor_ms, implicit - 5000);
    let higher = ReadsDelta { floor_ms: Some(implicit + 7000), seeds_floor: true, ..ReadsDelta::default() };
    let (reads, _) = store.merge_board_reads(main, &higher, NOW).unwrap();
    assert_eq!(reads.floor_ms, implicit + 7000);
}

/// A phone's opened marks, sent with no floor, leave the default to be
/// replaced by the Mac's seed afterwards.
#[test]
fn marks_alone_do_not_spend_the_seed() {
    let (store, main, t) = board(1);
    let implicit = store.board_reads(main, NOW).unwrap().floor_ms;
    store.merge_board_reads(main, &open(&t[0], implicit + 10), NOW).unwrap();
    let seed = ReadsDelta { floor_ms: Some(implicit - 1), seeds_floor: true, ..ReadsDelta::default() };
    let (reads, _) = store.merge_board_reads(main, &seed, NOW).unwrap();
    assert_eq!(reads.floor_ms, implicit - 1);
}

/// A floor reads every older ticket as read, so its marks are dropped, and a
/// mark at or under one is never stored.
#[test]
fn marks_at_or_under_the_floor_are_dropped() {
    let (store, main, t) = board(3);
    let base = NOW - FIRST_LOOK_MS;
    let delta = ReadsDelta { opened: vec![(t[0].id, base + 100), (t[1].id, base + 200), (t[2].id, base + 300)], ..ReadsDelta::default() };
    store.merge_board_reads(main, &delta, NOW).unwrap();
    let (reads, changed) = store.merge_board_reads(main, &floor(base + 200), NOW).unwrap();
    assert!(changed);
    assert_eq!(marks(&reads), vec![base + 300], "the two at or under the floor are gone");
    let stored: i64 =
        store.conn().query_row("SELECT COUNT(*) FROM task_reads", [], |r| r.get(0)).unwrap();
    assert_eq!(stored, 1, "dropped from the table, not only hidden");
    let (reads, _) = store.merge_board_reads(main, &open(&t[0], base + 150), NOW).unwrap();
    assert_eq!(marks(&reads), vec![base + 300], "a late older mark is not stored");
}

/// Two reads, one floor: the first look is written once.
#[test]
fn a_first_look_holds_still() {
    let (store, main, _) = board(0);
    let a = store.board_reads(main, NOW).unwrap();
    let b = store.board_reads(main, NOW + 3_600_000).unwrap();
    assert_eq!(a.floor_ms, b.floor_ms);
}

#[test]
fn a_mark_follows_its_task_across_a_move() {
    let (store, main, t) = board(1);
    let repo = t[0].repository_id;
    let other = store.create_workspace(repo, "Billing", "bil").unwrap().id;
    let at = NOW - 1000;
    store.merge_board_reads(main, &open(&t[0], at), NOW).unwrap();
    store.move_tasks(&[t[0].id], other, Actor::Manager).unwrap();
    assert!(store.board_reads(main, NOW).unwrap().opened.is_empty(), "no longer on the first board");
    assert_eq!(store.board_reads(other, NOW).unwrap().opened, vec![(t[0].id, at)]);
}

#[test]
fn a_mark_for_another_boards_task_is_refused() {
    let (store, main, t) = board(1);
    let other = store.create_workspace(t[0].repository_id, "Billing", "bil").unwrap().id;
    let err = store.merge_board_reads(other, &open(&t[0], NOW - 5), NOW).unwrap_err();
    assert!(matches!(err, DomainError::InvalidArgument { what: "other_board" }), "{err:?}");
    // And nothing of the delta landed: the whole write was refused first.
    let mixed = ReadsDelta { floor_ms: Some(NOW), opened: vec![(t[0].id, NOW - 5)], ..ReadsDelta::default() };
    store.merge_board_reads(other, &mixed, NOW).unwrap_err();
    assert_eq!(store.board_reads(other, NOW).unwrap().floor_ms, NOW - FIRST_LOOK_MS);
    let _ = main;
}

#[test]
fn a_mark_for_a_deleted_task_is_skipped() {
    let (store, main, _) = board(0);
    let (reads, changed) =
        store.merge_board_reads(main, &ReadsDelta { opened: vec![(Uuid::now_v7(), NOW)], ..ReadsDelta::default() }, NOW).unwrap();
    assert!(reads.opened.is_empty());
    assert!(!changed);
}

/// Deleting the task takes its mark with it.
#[test]
fn a_task_takes_its_mark_when_it_goes() {
    let (store, main, t) = board(1);
    store.merge_board_reads(main, &open(&t[0], NOW - 1), NOW).unwrap();
    store.conn().execute("DELETE FROM tasks WHERE id = ?1", [uuid_blob(t[0].id)]).unwrap();
    let stored: i64 = store.conn().query_row("SELECT COUNT(*) FROM task_reads", [], |r| r.get(0)).unwrap();
    assert_eq!(stored, 0);
}

/// An old store, with no read state anywhere, migrates and reads as a first
/// look with nothing opened.
#[test]
fn an_empty_store_migrates_to_a_first_look() {
    let (store, main, _) = board(0);
    let reads = store.board_reads(main, NOW).unwrap();
    assert_eq!(reads, BoardReads { workspace_id: main, floor_ms: NOW - FIRST_LOOK_MS, opened: vec![] });
}

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

/// A seed is a plain raise: the floor merges by max like every other value.
#[test]
fn a_floor_never_lowers_even_from_a_first_upload() {
    let (store, main, _) = board(0);
    let first = store.board_reads(main, NOW).unwrap().floor_ms;
    let (reads, changed) = store.merge_board_reads(main, &floor(first - 5000), NOW).unwrap();
    assert!(!changed);
    assert_eq!(reads.floor_ms, first);
}

/// The same writes in either order end in the same state: a phone's mark
/// between a Mac's older floor and the default is kept whether it arrives
/// before or after.
#[test]
fn the_same_writes_in_any_order_end_in_the_same_state() {
    let writes = |t: &Task| {
        vec![open(t, NOW - FIRST_LOOK_MS + 10), floor(NOW - FIRST_LOOK_MS - 5000), floor(NOW - 1000), open(t, NOW - 500)]
    };
    let mut ends = Vec::new();
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 0, 3, 2], [2, 3, 0, 1]] {
        let (store, main, t) = board(1);
        let all = writes(&t[0]);
        for i in order {
            store.merge_board_reads(main, &all[i], NOW).unwrap();
        }
        let r = store.board_reads(main, NOW).unwrap();
        ends.push((r.floor_ms, marks(&r)));
    }
    assert!(ends.windows(2).all(|w| w[0] == w[1]), "{ends:?}");
}

/// A device's clock ahead of the runner's cannot hide later news: every time
/// is clamped to the runner's now.
#[test]
fn a_future_time_cannot_hide_later_news() {
    let (store, main, t) = board(2);
    let later = NOW + 60_000;
    let (reads, _) = store
        .merge_board_reads(
            main,
            &ReadsDelta { floor_ms: Some(NOW + 10 * 86_400_000), opened: vec![(t[0].id, NOW + 86_400_000)] },
            NOW,
        )
        .unwrap();
    assert_eq!(reads.floor_ms, NOW, "the floor lands at runner now");
    assert!(reads.opened.is_empty(), "a mark clamped to the floor is dropped");
    let (reads, _) = store.merge_board_reads(main, &open(&t[1], NOW + 86_400_000), later).unwrap();
    assert_eq!(reads.opened, vec![(t[1].id, later)], "a mark lands at the runner's now, not the device's");
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

/// A ticket now on another board is skipped, not refused: the one-time
/// upload sends every local mark, and one moved ticket must not wedge it.
#[test]
fn a_mark_for_another_boards_task_is_skipped_and_the_rest_land() {
    let (store, main, t) = board(2);
    let other = store.create_workspace(t[0].repository_id, "Billing", "bil").unwrap().id;
    store.move_tasks(&[t[0].id], other, Actor::Manager).unwrap();
    let at = NOW - 1000;
    let delta = ReadsDelta { floor_ms: None, opened: vec![(t[0].id, at), (t[1].id, at)] };
    let (reads, _) = store.merge_board_reads(main, &delta, NOW).unwrap();
    assert_eq!(reads.opened, vec![(t[1].id, at)]);
    assert!(store.board_reads(other, NOW).unwrap().opened.is_empty(), "and not applied to the board it moved to");
}

/// A board already known is read without taking the write lock, so a read at
/// `read` scope never waits behind a writer.
#[test]
fn reading_a_known_board_takes_no_write_lock() {
    let dir = std::env::temp_dir().join(format!("farcooler-reads-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.db");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    store.board_reads(main, NOW).unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();
    other.busy_timeout(std::time::Duration::from_millis(0)).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    store.board_reads(main, NOW).expect("a read waits for no writer");
    other.execute_batch("ROLLBACK").unwrap();
    std::fs::remove_dir_all(&dir).ok();
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

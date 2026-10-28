use uuid::Uuid;

use super::*;
use crate::board_reads::{FIRST_LOOK_MS, ReadsDelta, Reader};
use crate::models::Actor;
use crate::review::ReviewedMark;
use crate::store::Store;

const NOW: i64 = 10_000_000_000;

/// A board with two tasks, a worktree, and Bob and Carol, members.
fn runner() -> (Store, Uuid, Vec<Uuid>, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..2).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap().id).collect();
    let worktree = store.create_worktree(repo, "feat/x", "/wt/marks", false).unwrap().id;
    store.set_member("bob", "Bob", &[]).unwrap();
    store.set_member("carol", "Carol", &[]).unwrap();
    (store, main, tasks, worktree)
}

fn open(task: Uuid, ms: i64) -> ReadsDelta {
    ReadsDelta { opened: vec![(task, ms)], ..ReadsDelta::default() }
}

fn rows(store: &Store, table: &str) -> i64 {
    store.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

fn mark(at: i64) -> ReviewedMark {
    ReviewedMark { head_commit: "head".into(), worktree_digest: "digest".into(), gate_head: 1, gate_index: 2, marked_at: at }
}

/// ov-740: Bob opening a card and marking the board read changes his read
/// state alone. The owner's, and Carol's, are what they were.
#[test]
fn a_members_board_reads_are_theirs_alone() {
    let (store, main, t, _) = runner();
    let base = NOW - FIRST_LOOK_MS;
    let bob = Reader::Member("bob");
    let (owners, _) = store.merge_board_reads(main, &open(t[0], base + 500), NOW).unwrap();

    let (his, changed) = store.merge_board_reads_of(bob, main, &open(t[1], base + 700), NOW).unwrap();
    assert!(changed);
    assert_eq!(his.opened, vec![(t[1], base + 700)], "his own first look, without the owner's mark");
    let (his, _) =
        store.merge_board_reads_of(bob, main, &ReadsDelta { floor_ms: Some(base + 900), ..ReadsDelta::default() }, NOW).unwrap();
    assert_eq!(his.floor_ms, base + 900);

    assert_eq!(store.board_reads(main, NOW).unwrap(), owners, "the owner's Unread is untouched");
    let carols = store.board_reads_of(Reader::Member("carol"), main, NOW).unwrap();
    assert_eq!((carols.floor_ms, carols.opened.len()), (base, 0), "and so is Carol's");
}

/// And the owner's marks reach no member.
#[test]
fn the_owners_board_reads_reach_no_member() {
    let (store, main, t, _) = runner();
    let base = NOW - FIRST_LOOK_MS;
    store.merge_board_reads(main, &open(t[0], base + 500), NOW).unwrap();
    store.merge_board_reads(main, &ReadsDelta { floor_ms: Some(base + 600), ..ReadsDelta::default() }, NOW).unwrap();
    let his = store.board_reads_of(Reader::Member("bob"), main, NOW).unwrap();
    assert_eq!((his.floor_ms, his.opened.len()), (base, 0));
}

/// A runner with no members writes only the tables it always did, so a
/// build from before ov-740 reads every mark it made.
#[test]
fn the_owner_writes_where_the_owner_always_did() {
    let (store, main, t, worktree) = runner();
    store.merge_board_reads(main, &open(t[0], NOW - 10), NOW).unwrap();
    store.mark_reviewed_with_gate(worktree, "feat/x", "head", "digest", 1, 2, NOW).unwrap();
    assert_eq!((rows(&store, "board_read_floors"), rows(&store, "task_reads"), rows(&store, "review_reviewed")), (1, 1, 1));
    for table in ["member_board_read_floors", "member_task_reads", "member_review_reviewed"] {
        assert_eq!(rows(&store, table), 0, "{table}");
    }
}

/// ov-740: a member's review mark is theirs, and the owner's theirs.
#[test]
fn a_members_review_mark_is_theirs_alone() {
    let (store, _, _, worktree) = runner();
    let bob = Reader::Member("bob");
    store.mark_reviewed_as(bob, worktree, "feat/x", &mark(5)).unwrap();
    assert_eq!(store.reviewed_mark(worktree, "feat/x").unwrap(), None, "the owner hasn't reviewed it");
    assert_eq!(store.reviewed_mark_of(Reader::Member("carol"), worktree, "feat/x").unwrap(), None, "nor Carol");
    assert_eq!(store.reviewed_mark_of(bob, worktree, "feat/x").unwrap(), Some(mark(5)));

    store.mark_reviewed_with_gate(worktree, "feat/x", "head", "digest", 1, 2, 9).unwrap();
    store.mark_reviewed_as(bob, worktree, "feat/x", &mark(7)).unwrap();
    assert_eq!(store.reviewed_mark(worktree, "feat/x").unwrap(), Some(mark(9)));
    assert_eq!(store.reviewed_mark_of(bob, worktree, "feat/x").unwrap(), Some(mark(7)), "a later mark replaces his");
}

/// A member removed takes their marks with them, so a handle used again
/// starts with a first look of its own.
#[test]
fn a_removed_members_marks_go_with_them() {
    let (store, main, t, worktree) = runner();
    let bob = Reader::Member("bob");
    store.merge_board_reads_of(bob, main, &open(t[0], NOW - 10), NOW).unwrap();
    store.mark_reviewed_as(bob, worktree, "feat/x", &mark(5)).unwrap();
    store.remove_member("bob").unwrap();
    for table in ["member_board_read_floors", "member_task_reads", "member_review_reviewed"] {
        assert_eq!(rows(&store, table), 0, "{table}");
    }
    store.set_member("bob", "Another Bob", &[]).unwrap();
    assert!(store.board_reads_of(bob, main, NOW).unwrap().opened.is_empty());
}

/// The migration is the 50th (0059) and `Welcome`: a build from before it
/// never names the tables.
#[test]
fn the_migration_is_welcome() {
    let it = &crate::migrate::MIGRATIONS[49];
    assert!(std::ptr::fn_addr_eq(it.0, migration_0059_member_marks as fn(&rusqlite::Transaction) -> rusqlite::Result<()>));
    assert_eq!(it.1, crate::compat::Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 55);
}

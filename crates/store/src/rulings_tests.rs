use super::*;

use crate::models::Task;
use crate::plan::NewTheme;

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn new(decision: &str) -> NewRuling {
    NewRuling {
        decision: decision.into(),
        why: "It reads as one app.".into(),
        reversal: "One token, about ten minutes.".into(),
        theme_id: None,
    }
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn shorts(store: &Store, main: Uuid, since: i64) -> Vec<String> {
    store.plan(main, since).unwrap().rulings.iter().map(Ruling::short).collect()
}

/// The migration is the 26th and `Welcome`, so the build before it can still
/// open the file; ov-333's reversal column follows it as 0029, also `Welcome`.
#[test]
fn the_migration_is_welcome() {
    use crate::compat::Older;
    let last = &crate::migrate::MIGRATIONS[25];
    assert!(std::ptr::fn_addr_eq(last.0, migration_0026_rulings as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    let col = &crate::migrate::MIGRATIONS[29];
    assert!(std::ptr::fn_addr_eq(col.0, migration_0030_ruling_reversals as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(col.1, Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 34);
}

/// A ruling starts standing, keeps what it was given trimmed, and takes the
/// board's next number; another board counts from one.
#[test]
fn a_ruling_takes_the_board_s_next_number() {
    let (store, main, t) = board(2);
    let a = store.add_ruling(main, &new("  The inbox is amber. "), &[t[0].id, t[1].id], Actor::Manager).unwrap();
    let b = store.add_ruling(main, &new("The gutter is 12 points."), &[], Actor::User).unwrap();
    assert_eq!((a.number, a.short().as_str(), b.number), (1, "R-1", 2));
    assert_eq!(a.decision, "The inbox is amber.");
    assert_eq!(a.state, RulingState::Standing);
    assert_eq!(a.tasks, vec![t[0].id, t[1].id]);
    assert_eq!((a.actor.as_str(), b.actor.as_str()), ("manager", "user"));
    assert_eq!((a.settled_at, a.settled_by.as_deref()), (None, None));
    assert!(a.created_at > 0);

    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    assert_eq!(store.add_ruling(other.id, &new("Elsewhere."), &[], Actor::Manager).unwrap().number, 1);
}

/// Standing goes to confirmed or reversed, confirmed can still be reversed,
/// and reversed is the end. Each move records who, when and the note.
#[test]
fn a_ruling_is_confirmed_or_reversed_once() {
    let (store, main, _) = board(0);
    let a = store.add_ruling(main, &new("A"), &[], Actor::Manager).unwrap();
    assert_eq!(refused(store.set_ruling(a.id, RulingState::Standing, None, Actor::Manager)), "ruling_state");
    let a = store.set_ruling(a.id, RulingState::Confirmed, Some(" Keep it. "), Actor::User).unwrap();
    assert_eq!((a.state, a.note.as_str(), a.settled_by.as_deref()), (RulingState::Confirmed, "Keep it.", Some("user")));
    assert!(a.settled_at.is_some());
    assert_eq!(refused(store.set_ruling(a.id, RulingState::Confirmed, None, Actor::Manager)), "ruling_state");
    let a = store.set_ruling(a.id, RulingState::Reversed, None, Actor::Manager).unwrap();
    assert_eq!((a.state, a.note.as_str()), (RulingState::Reversed, ""));
    assert_eq!(a.resource_version, 3);
    for to in [RulingState::Standing, RulingState::Confirmed, RulingState::Reversed] {
        assert_eq!(refused(store.set_ruling(a.id, to, None, Actor::Manager)), "ruling_state");
    }
    let b = store.add_ruling(main, &new("B"), &[], Actor::Manager).unwrap();
    assert_eq!(store.set_ruling(b.id, RulingState::Reversed, None, Actor::Manager).unwrap().state, RulingState::Reversed);
    assert!(matches!(store.set_ruling(Uuid::now_v7(), RulingState::Confirmed, None, Actor::Manager), Err(DomainError::NotFound)));
}

/// A decision, a reason and what reversing costs are all required, and each
/// has a ceiling; so does a note.
#[test]
fn every_part_is_required_and_bounded() {
    let (store, main, _) = board(0);
    let blank = |f: fn(&mut NewRuling)| {
        let mut r = new("A");
        f(&mut r);
        refused(store.add_ruling(main, &r, &[], Actor::Manager))
    };
    assert_eq!(blank(|r| r.decision = "  ".into()), "decision");
    assert_eq!(blank(|r| r.why = String::new()), "why");
    assert_eq!(blank(|r| r.reversal = " ".into()), "reversal");
    assert_eq!(blank(|r| r.decision = "x".repeat(301)), "decision");
    assert_eq!(blank(|r| r.decision = "Two\nlines".into()), "decision");
    assert_eq!(blank(|r| r.why = "x".repeat(601)), "why");
    assert_eq!(blank(|r| r.reversal = "x".repeat(301)), "reversal");
    let a = store.add_ruling(main, &new(&"x".repeat(300)), &[], Actor::Manager).unwrap();
    let long = "x".repeat(301);
    assert_eq!(refused(store.set_ruling(a.id, RulingState::Confirmed, Some(&long), Actor::Manager)), "note");
    // Nothing was written by a refusal: the next number is still 2.
    assert_eq!(store.add_ruling(main, &new("B"), &[], Actor::Manager).unwrap().number, 2);
}

/// A ruling's cards and theme are on its own board.
#[test]
fn another_board_s_cards_and_theme_are_refused() {
    let (store, main, t) = board(1);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theirs = store.create_task(other.id, "theirs", Actor::Manager).unwrap();
    let their_theme = store.create_theme(other.id, &NewTheme { name: "Theirs".into(), ..Default::default() }, &[], Actor::Manager).unwrap();
    assert_eq!(refused(store.add_ruling(main, &new("A"), &[t[0].id, theirs.id], Actor::Manager)), "other_board");
    let with_theme = NewRuling { theme_id: Some(their_theme.id), ..new("A") };
    assert_eq!(refused(store.add_ruling(main, &with_theme, &[], Actor::Manager)), "other_board");
    let ours = store.create_theme(main, &NewTheme { name: "Ours".into(), ..Default::default() }, &[], Actor::Manager).unwrap();
    let r = store.add_ruling(main, &NewRuling { theme_id: Some(ours.id), ..new("A") }, &[t[0].id], Actor::Manager).unwrap();
    assert_eq!(r.theme_id, Some(ours.id));
}

/// The plan read lists standing rulings first, newest first, then settled
/// ones by when they were settled. A settled ruling doesn't age out (it is
/// Past Decisions' history), only the cap trims the oldest.
#[test]
fn the_plan_reads_standing_first_newest_first() {
    let (store, main, _) = board(0);
    let ids: Vec<Uuid> =
        (1..=4).map(|i| store.add_ruling(main, &new(&format!("R{i}")), &[], Actor::Manager).unwrap().id).collect();
    store.set_ruling(ids[0], RulingState::Confirmed, None, Actor::User).unwrap();
    store.set_ruling(ids[2], RulingState::Reversed, None, Actor::User).unwrap();
    assert_eq!(shorts(&store, main, 0), ["R-4", "R-2", "R-3", "R-1"]);
    assert_eq!(shorts(&store, main, i64::MAX), ["R-4", "R-2", "R-3", "R-1"], "a settled ruling is history, not aged out");
}

/// A read that didn't ask for everything carries the last `SETTLED_READ_CAP`
/// settled rulings, newest settled first; the read for everything carries all.
#[test]
fn the_settled_history_is_capped_unless_all_is_asked_for() {
    let (store, main, _) = board(0);
    let total = SETTLED_READ_CAP + 3;
    for i in 1..=total {
        let r = store.add_ruling(main, &new(&format!("R{i}")), &[], Actor::Manager).unwrap();
        store.set_ruling(r.id, RulingState::Confirmed, None, Actor::User).unwrap();
        // Settled in order, a millisecond apart, whatever the clock's grain.
        store.conn().execute("UPDATE board_rulings SET settled_at = ?1 WHERE id = ?2", params![i as i64, uuid_blob(r.id)]).unwrap();
    }
    let open = store.add_ruling(main, &new("open"), &[], Actor::Manager).unwrap();
    let capped = shorts(&store, main, 1);
    assert_eq!(capped.len(), SETTLED_READ_CAP + 1, "every open one plus the cap");
    assert_eq!(capped[0], open.short());
    assert_eq!(capped[1], format!("R-{total}"), "the newest settled first");
    assert!(!capped.contains(&"R-1".to_string()), "the oldest fell off the end");
    assert_eq!(shorts(&store, main, 0).len(), total + 1, "all, when asked for");
    // The daemon asks for all with `i64::MIN` (`include_closed`), not 0.
    assert_eq!(shorts(&store, main, i64::MIN).len(), total + 1, "i64::MIN is all too");
}

/// Keep is the owner's mark: it settles the ruling as confirmed, records who
/// and when, and never touches `reversed_sha`.
#[test]
fn keeping_records_who_and_when() {
    let (store, main, _) = board(0);
    let a = store.add_ruling(main, &new("A"), &[], Actor::Manager).unwrap();
    let kept = store.set_ruling(a.id, RulingState::Confirmed, None, Actor::User).unwrap();
    assert_eq!((kept.state, kept.settled_by.as_deref(), kept.reversed_sha.as_deref()), (RulingState::Confirmed, Some("user"), None));
    assert!(kept.settled_at.unwrap() >= kept.created_at);
}

/// Reverse-marking takes the commit, refuses one that isn't a hex SHA, and
/// stores it lowercase; a kept ruling can still be marked reversed.
#[test]
fn a_reversal_is_marked_with_its_commit() {
    let (store, main, _) = board(0);
    let a = store.add_ruling(main, &new("A"), &[], Actor::Manager).unwrap();
    for bad in ["", "abc", "xyz1234", "not a sha", &"a".repeat(65)] {
        assert_eq!(refused(store.reverse_ruling(a.id, bad, None, Actor::Manager)), "reversed_sha", "{bad:?}");
    }
    assert_eq!(store.ruling(a.id).unwrap().state, RulingState::Standing, "a refusal wrote nothing");
    store.set_ruling(a.id, RulingState::Confirmed, None, Actor::User).unwrap();
    let r = store.reverse_ruling(a.id, " 6E7E5618 ", Some("Done."), Actor::Manager).unwrap();
    assert_eq!((r.state, r.reversed_sha.as_deref(), r.note.as_str()), (RulingState::Reversed, Some("6e7e5618"), "Done."));
    assert_eq!(r.settled_by.as_deref(), Some("manager"));
    assert_eq!(refused(store.reverse_ruling(a.id, "6e7e5618", None, Actor::Manager)), "ruling_state", "reversed is final");
}

/// Keep All keeps every open ruling on this board, once, and no other
/// board's, and none already settled.
#[test]
fn keep_all_keeps_every_open_ruling_on_this_board_only() {
    let (store, main, _) = board(0);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let ids: Vec<Uuid> = (1..=3).map(|i| store.add_ruling(main, &new(&format!("R{i}")), &[], Actor::Manager).unwrap().id).collect();
    let theirs = store.add_ruling(other.id, &new("Elsewhere"), &[], Actor::Manager).unwrap();
    store.reverse_ruling(ids[0], "abcd1234", None, Actor::Manager).unwrap();
    let kept = store.keep_all_rulings(main, Actor::User).unwrap();
    assert_eq!(kept.iter().map(Ruling::short).collect::<Vec<_>>(), ["R-3", "R-2"]);
    assert!(kept.iter().all(|r| r.state == RulingState::Confirmed && r.settled_by.as_deref() == Some("user")));
    assert_eq!(store.ruling(ids[0]).unwrap().state, RulingState::Reversed, "a reversed one stays reversed");
    assert_eq!(store.ruling(theirs.id).unwrap().state, RulingState::Standing, "another board's stays open");
    assert!(store.keep_all_rulings(main, Actor::User).unwrap().is_empty(), "nothing left to keep");
}

/// A ruling's cards come with their keys, and stay out of the plan's own
/// cards: a card a ruling names but no theme or lane does is in no lane by
/// design, and must not read as "in progress with no lane" (review 1005a F2).
#[test]
fn a_ruling_s_cards_carry_keys_and_stay_out_of_the_plan_s_cards() {
    let (store, main, t) = board(2);
    store.set_task_status(t[1].id, crate::models::TaskStatus::InProgress, Actor::Manager).unwrap();
    store.add_ruling(main, &new("R1"), &[t[1].id, t[0].id], Actor::Manager).unwrap();
    let plan = store.plan(main, 0).unwrap();
    assert!(plan.cards.is_empty(), "{:?}", plan.cards);
    assert!(plan.coverage.is_empty());
    assert_eq!(plan.rulings[0].task_keys, vec![t[1].key.clone(), t[0].key.clone()]);
    let added = store.ruling(plan.rulings[0].id).unwrap();
    assert_eq!(added.task_keys, vec![t[1].key.clone(), t[0].key.clone()]);
}

/// A card moved to another board leaves its rulings' cards in the read, as it
/// leaves a theme's.
#[test]
fn a_moved_card_leaves_the_ruling_s_cards() {
    let (store, main, t) = board(2);
    store.add_ruling(main, &new("A"), &[t[0].id, t[1].id], Actor::Manager).unwrap();
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    store.move_tasks(&[t[0].id], other.id, Actor::Manager).unwrap();
    assert_eq!(store.plan(main, 0).unwrap().rulings[0].tasks, vec![t[1].id]);
}

/// A deleted board takes its rulings, and their cards, with it by cascade.
#[test]
fn rulings_go_with_their_board() {
    let (store, main, _) = board(0);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let task = store.create_task(other.id, "theirs", Actor::Manager).unwrap();
    let r = store.add_ruling(other.id, &new("A"), &[task.id], Actor::Manager).unwrap();
    // A board with a card can't be deleted; the card moves first.
    store.move_tasks(&[task.id], main, Actor::Manager).unwrap();
    store.delete_workspace(other.id).unwrap();
    assert!(matches!(store.ruling(r.id), Err(DomainError::NotFound)));
    let left: i64 = store.conn().query_row("SELECT count(*) FROM board_ruling_tasks", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
}

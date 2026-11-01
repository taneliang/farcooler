//! The collecting landing (ov-520, ov-523, ov-872): tickets that pass review
//! join it, it departs when nothing is ahead, a ticket that leaves landing
//! comes off, and the orchestrator is told of each departure once.

use super::*;

use farcooler_core::DomainError;

use crate::board_ci::{CiRead, CiStatus};
use crate::landings::{LandingUpdate, NewLanding};
use crate::models::Task;
use crate::ticket_lines::{LineUpdate, NewTicketLine};

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn refused(r: farcooler_core::Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn to(store: &Store, task: Uuid, stage: Stage) {
    store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager).unwrap();
}

/// Put `task` on the line and take it through review into landing.
fn reviewed(store: &Store, task: Uuid) {
    store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
    for stage in [Stage::Building, Stage::Reviewing, Stage::Landing] {
        to(store, task, stage);
    }
}

fn main_ci(store: &Store, main: Uuid, status: CiStatus) {
    let read = CiRead {
        subject: crate::board_ci::MAIN_SUBJECT.into(),
        sha: "abc1234".into(),
        status,
        url: String::new(),
        jobs: vec![],
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    };
    store.record_ci(main, &read).unwrap();
}

/// A ticket that passes review joins the collecting landing and, with nothing
/// ahead, departs with it at once; the next one waits behind it.
#[test]
fn a_reviewed_ticket_joins_and_departs_when_nothing_is_ahead() {
    let (store, _, t) = board(2);
    reviewed(&store, t[0].id);
    let first = store.landing_of(t[0].id).unwrap().expect("it joined");
    assert_eq!(first.state, LandingState::Gating, "nothing ahead: it left");
    assert!(!first.collecting);
    reviewed(&store, t[1].id);
    let second = store.landing_of(t[1].id).unwrap().unwrap();
    assert_eq!((second.state, second.collecting), (LandingState::Open, true), "waits behind the first");
    assert_eq!(second.number, first.number + 1);
}

/// Main red holds the collecting landing; main green again lets it go.
#[test]
fn main_red_holds_the_departure() {
    let (store, main, t) = board(1);
    main_ci(&store, main, CiStatus::Failed);
    reviewed(&store, t[0].id);
    assert_eq!(store.landing_of(t[0].id).unwrap().unwrap().state, LandingState::Open);
    main_ci(&store, main, CiStatus::Passed);
    assert_eq!(store.landing_of(t[0].id).unwrap().unwrap().state, LandingState::Gating, "main green: it left");
}

/// A ticket sent back from landing comes off a landing whose SHA isn't out; on
/// a pushed one it stays, and its timeline says so.
#[test]
fn a_ticket_that_leaves_landing_comes_off_unless_the_push_carries_it() {
    let (store, main, t) = board(2);
    main_ci(&store, main, CiStatus::Failed);
    reviewed(&store, t[0].id);
    reviewed(&store, t[1].id);
    let open = store.landing_of(t[0].id).unwrap().unwrap();
    to(&store, t[0].id, Stage::Building);
    assert_eq!(store.landing_of(t[0].id).unwrap(), None, "off the open landing");
    assert_eq!(store.landing(open.id).unwrap().tickets, [t[1].id]);

    main_ci(&store, main, CiStatus::Passed);
    let gone = store.landing_of(t[1].id).unwrap().unwrap();
    store.update_landing(gone.id, &LandingUpdate { sha: Some("deadbee".into()), ..Default::default() }, Actor::Manager).unwrap();
    to(&store, t[1].id, Stage::Building);
    assert_eq!(store.landing_of(t[1].id).unwrap().map(|l| l.id), Some(gone.id), "the push carries it");
    let said = store.line_events(LineSubject::Ticket(t[1].id), 0).unwrap();
    assert!(said.iter().any(|e| e.body.starts_with("Stays on Landing")));
}

/// A ticket taken off by hand stays off: the scan skips it until it leaves
/// landing and comes back through review.
#[test]
fn a_ticket_taken_off_by_hand_stays_off() {
    let (store, main, t) = board(1);
    main_ci(&store, main, CiStatus::Failed);
    reviewed(&store, t[0].id);
    let open = store.landing_of(t[0].id).unwrap().unwrap();
    let off = LandingUpdate { remove_tickets: vec![t[0].id], ..Default::default() };
    store.update_landing(open.id, &off, Actor::Manager).unwrap();
    assert!(store.ticket_line(t[0].id).unwrap().held_off);
    main_ci(&store, main, CiStatus::Passed);
    assert_eq!(store.collect_ready_tickets(main, Actor::Runner).unwrap().joined, Vec::<Uuid>::new());
    to(&store, t[0].id, Stage::Building);
    to(&store, t[0].id, Stage::Reviewing);
    to(&store, t[0].id, Stage::Landing);
    assert!(store.landing_of(t[0].id).unwrap().is_some(), "back through review, it joins again");
}

/// A ticket stranded in landing by an older build is caught up by the scan.
#[test]
fn the_scan_collects_a_stranded_ticket() {
    let (store, main, t) = board(1);
    store.put_on_line(t[0].id, &NewTicketLine::default(), Actor::Manager).unwrap();
    crate::testing::strand_in_landing(&store, t[0].id);
    assert_eq!(store.workspaces_to_collect().unwrap(), [main]);
    let collected = store.collect_ready_tickets(main, Actor::Runner).unwrap();
    assert_eq!(collected.joined, [t[0].id]);
    assert!(collected.departed.is_some());
    assert!(store.workspaces_to_collect().unwrap().is_empty());
}

/// The orchestrator is told of a departure once, and not of one whose
/// landing has a SHA out.
#[test]
fn a_departure_is_told_once() {
    let (store, main, t) = board(1);
    reviewed(&store, t[0].id);
    let untold = store.departures_untold(main).unwrap();
    assert_eq!(untold.len(), 1);
    assert_eq!(untold[0].cards, [t[0].id]);
    assert!(store.record_departure_told(&untold[0]).unwrap());
    assert!(!store.record_departure_told(&untold[0]).unwrap(), "only once");
    assert!(store.departures_untold(main).unwrap().is_empty());
}

/// A departed landing that lost every ticket is dropped, so it isn't ahead for
/// good, and the one behind it can go.
#[test]
fn an_emptied_departure_is_dropped_and_the_next_goes() {
    let (store, main, t) = board(2);
    reviewed(&store, t[0].id);
    reviewed(&store, t[1].id);
    let first = store.landing_of(t[0].id).unwrap().unwrap();
    to(&store, t[0].id, Stage::Building);
    assert_eq!(store.landing(first.id).unwrap().state, LandingState::Dropped);
    assert_eq!(store.landing_of(t[1].id).unwrap().unwrap().state, LandingState::Gating);
    let _ = main;
}

/// A landing started by hand adopts the collecting one: same number, exactly
/// the tickets listed, and the rest move to a fresh collecting landing.
#[test]
fn a_landing_started_by_hand_adopts_the_collecting_one() {
    let (store, main, t) = board(2);
    main_ci(&store, main, CiStatus::Failed);
    reviewed(&store, t[0].id);
    reviewed(&store, t[1].id);
    let collecting = store.landing_of(t[0].id).unwrap().unwrap();
    let new = NewLanding { route: Route::Push, base: "origin/main".into(), title: "Hand".into(), lander: None, collecting: false };
    let started = store.start_landing(main, &new, &[t[0].id], Actor::Manager).unwrap();
    assert_eq!((started.id, started.number), (collecting.id, collecting.number));
    assert!(!started.collecting);
    assert_eq!(started.tickets, [t[0].id]);
    let rest = store.landing_of(t[1].id).unwrap().unwrap();
    assert!(rest.collecting && rest.id != started.id);
    let stray = store.create_task(main, "not on the line", Actor::Manager).unwrap();
    assert_eq!(refused(store.start_landing(main, &new, &[stray.id], Actor::Manager)), "not_on_the_line");
}

/// Landing a landing lands its tickets at its SHA and takes off one never
/// ready; dropping one lets its tickets go.
#[test]
fn landing_lands_its_tickets_and_dropping_lets_them_go() {
    let (store, main, t) = board(2);
    reviewed(&store, t[0].id);
    let l = store.landing_of(t[0].id).unwrap().unwrap();
    let done = LandingUpdate { sha: Some("abc1234".into()), state: Some(LandingState::Landed), ..Default::default() };
    store.update_landing(l.id, &done, Actor::Manager).unwrap();
    let line = store.ticket_line(t[0].id).unwrap();
    assert_eq!((line.stage, line.landed_sha.as_deref()), (Stage::Landed, Some("abc1234")));

    reviewed(&store, t[1].id);
    let next = store.landing_of(t[1].id).unwrap().unwrap();
    store.update_landing(next.id, &LandingUpdate { state: Some(LandingState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(store.ticket_line(t[1].id).unwrap().stage, Stage::Landing, "it waits to land another way");
    let _ = main;
}

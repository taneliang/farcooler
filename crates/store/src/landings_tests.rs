use super::*;

use crate::agents::{AgentPlace, NewAgent};
use crate::line_migration::store_with_the_line;
use crate::ticket_lines::{LineUpdate, NewTicketLine};

fn board(n: usize) -> (Store, Uuid, Vec<Uuid>) {
    let store = store_with_the_line();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("card {i}"), Actor::Manager).unwrap().id).collect();
    (store, main, tasks)
}

/// Put `task` on the line and walk it to `stage`.
fn at(store: &Store, task: Uuid, stage: Stage) {
    store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
    let path: &[Stage] = match stage {
        Stage::NeedsYou => &[Stage::Building, Stage::NeedsYou],
        Stage::Waiting => &[],
        _ => &[Stage::Building, Stage::Reviewing, Stage::Landing],
    };
    for s in path {
        store.update_line(task, &LineUpdate { stage: Some(*s), ..Default::default() }, Actor::Manager).unwrap();
    }
}

fn push() -> NewLanding {
    NewLanding { route: Route::Push, base: "origin/main".into(), title: String::new(), lander: None, collecting: false }
}

fn set(store: &Store, landing: &Landing, update: LandingUpdate) -> Result<Landing> {
    store.update_landing(landing.id, &update, Actor::Manager)
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The normal case: one ticket, pushed, landed. The ticket lands at the
/// landing's SHA, and both timelines say so.
#[test]
fn one_ticket_lands_on_its_own() {
    let (store, main, t) = board(1);
    at(&store, t[0], Stage::Landing);
    let landing = store.start_landing(main, &push(), &[t[0]], Actor::Manager).unwrap();
    assert_eq!((landing.number, landing.state, landing.tickets.clone()), (1, LandingState::Open, vec![t[0]]));
    assert_eq!(store.landing_of(t[0]).unwrap().map(|l| l.id), Some(landing.id));
    let pushed = set(&store, &landing, LandingUpdate { sha: Some("abc1234".into()), ..Default::default() }).unwrap();
    assert_eq!(pushed.state, LandingState::Pushed);
    let landed = set(&store, &landing, LandingUpdate { state: Some(LandingState::Landed), ..Default::default() }).unwrap();
    assert!(landed.landed_at.is_some());
    let line = store.ticket_line(t[0]).unwrap();
    assert_eq!((line.stage, line.landed_sha.as_deref()), (Stage::Landed, Some("abc1234")));
    assert_eq!(store.landing_of(t[0]).unwrap(), None, "a landed landing is not open");
    let ticket = store.line_events(LineSubject::Ticket(t[0]), 0).unwrap();
    assert!(ticket.iter().any(|e| e.kind == "stage" && e.body == "Landed." && e.landing_id == Some(landing.id)));
    let history = store.line_events(LineSubject::Landing(landing.id), 0).unwrap();
    assert!(history.iter().any(|e| e.kind == "ticket_on"));
    assert!(history.iter().any(|e| e.body == "Landed."));
    assert_eq!(refused(set(&store, &landed, LandingUpdate { title: Some("x".into()), ..Default::default() })), "landing_settled");
}

/// Only a ticket that is ready lands, and only on one open landing at a time.
#[test]
fn a_ticket_must_be_ready_and_on_one_landing() {
    let (store, main, t) = board(2);
    at(&store, t[0], Stage::Landing);
    at(&store, t[1], Stage::Waiting);
    assert_eq!(refused(store.start_landing(main, &push(), &[t[1]], Actor::Manager)), "not_ready");
    let first = store.start_landing(main, &push(), &[t[0]], Actor::Manager).unwrap();
    assert_eq!(refused(store.start_landing(main, &push(), &[t[0]], Actor::Manager)), "on_a_landing");
    set(&store, &first, LandingUpdate { state: Some(LandingState::Dropped), ..Default::default() }).unwrap();
    assert_eq!(store.ticket_line(t[0]).unwrap().stage, Stage::Landing, "dropping leaves it to land another way");
    let second = store.start_landing(main, &push(), &[t[0]], Actor::Manager).unwrap();
    assert_eq!(second.number, 2);
    let off = set(&store, &second, LandingUpdate { remove_tickets: vec![t[0]], ..Default::default() }).unwrap();
    assert!(off.tickets.is_empty());
    let back = set(&store, &second, LandingUpdate { add_tickets: vec![t[0]], ..Default::default() }).unwrap();
    assert_eq!(back.tickets, vec![t[0]]);
}

/// On the ask route, a ticket waiting on you lands when you tap Land.
#[test]
fn the_ask_route_takes_a_ticket_that_needs_you() {
    let (store, main, t) = board(1);
    at(&store, t[0], Stage::NeedsYou);
    assert_eq!(refused(store.start_landing(main, &push(), &[t[0]], Actor::Manager)), "not_ready");
    let ask = NewLanding { route: Route::Ask, ..push() };
    let landing = store.start_landing(main, &ask, &[t[0]], Actor::User).unwrap();
    assert_eq!(landing.route, Route::Ask);
    assert_eq!(store.ticket_line(t[0]).unwrap().stage, Stage::Landing);
}

/// A stacked ticket lands with its parent or after it; one landing lands the
/// parent first.
#[test]
fn a_stack_lands_parent_first() {
    let (store, main, t) = board(2);
    at(&store, t[0], Stage::Landing);
    at(&store, t[1], Stage::Landing);
    store.update_line(t[1], &LineUpdate { stack_parent: Some(Some(t[0])), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(refused(store.start_landing(main, &push(), &[t[1]], Actor::Manager)), "parent_not_landed");
    let both = store.start_landing(main, &push(), &[t[1], t[0]], Actor::Manager).unwrap();
    set(&store, &both, LandingUpdate { sha: Some("def5678".into()), ..Default::default() }).unwrap();
    set(&store, &both, LandingUpdate { state: Some(LandingState::Landed), ..Default::default() }).unwrap();
    for task in &t {
        assert_eq!(store.ticket_line(*task).unwrap().stage, Stage::Landed);
    }
}

/// Tickets that share a branch land together.
#[test]
fn tickets_sharing_a_branch_land_together() {
    let (store, main, t) = board(2);
    at(&store, t[0], Stage::Landing);
    at(&store, t[1], Stage::Landing);
    store.conn().execute("UPDATE ticket_lines SET same_branch = ?1", [uuid_blob(Uuid::now_v7())]).unwrap();
    assert_eq!(refused(store.start_landing(main, &push(), &[t[0]], Actor::Manager)), "same_branch");
    store.start_landing(main, &push(), &[t[0], t[1]], Actor::Manager).unwrap();
}

/// The runner lands what it read as green only if nothing moved since, and
/// nothing lands without a SHA.
#[test]
fn a_landing_moves_only_as_expected() {
    let (store, main, t) = board(1);
    at(&store, t[0], Stage::Landing);
    let landing = store.start_landing(main, &push(), &[t[0]], Actor::Manager).unwrap();
    assert_eq!(refused(set(&store, &landing, LandingUpdate { state: Some(LandingState::Landed), ..Default::default() })), "no_sha");
    set(&store, &landing, LandingUpdate { sha: Some("aaa1111".into()), ..Default::default() }).unwrap();
    set(&store, &landing, LandingUpdate { state: Some(LandingState::Green), ..Default::default() }).unwrap();
    let stale = LandingUpdate { state: Some(LandingState::Landed), expect: Some((LandingState::Pushed, "aaa1111".into())), ..Default::default() };
    assert_eq!(refused(set(&store, &landing, stale)), "landing_changed");
    let fresh = LandingUpdate { state: Some(LandingState::Landed), expect: Some((LandingState::Green, "aaa1111".into())), ..Default::default() };
    assert_eq!(set(&store, &landing, fresh).unwrap().state, LandingState::Landed);
    for word in LandingState::WORDS {
        assert_eq!(LandingState::parse(word).map(LandingState::as_str), Some(word));
    }
}

/// A landing's lander is an open lander (or the orchestrator) on its board.
#[test]
fn the_lander_must_be_one() {
    let (store, main, _) = board(0);
    let agent = |role: AgentRole, id: &str| {
        let place = AgentPlace::Subagent { id: id.into(), parent: None };
        let new = NewAgent { role, place, harness: "codex".into(), session_id: None, session_cwd: None, model: None, title: None };
        store.record_agent(main, &new).unwrap().id
    };
    let builder = agent(AgentRole::Builder, "b");
    let lander = agent(AgentRole::Lander, "l");
    assert_eq!(refused(store.start_landing(main, &NewLanding { lander: Some(builder), ..push() }, &[], Actor::Manager)), "lander");
    let landing = store.start_landing(main, &NewLanding { lander: Some(lander), ..push() }, &[], Actor::Manager).unwrap();
    assert_eq!(landing.lander_id, Some(lander));
}

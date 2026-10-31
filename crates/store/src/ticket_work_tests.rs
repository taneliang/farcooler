use super::*;

use crate::agents::{AgentPlace, NewAgent};
use crate::line_migration::store_with_the_line;
use crate::ticket_lines::LineSubject;

fn board(n: usize) -> (Store, Uuid, Vec<Uuid>) {
    let store = store_with_the_line();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("card {i}"), Actor::Manager).unwrap().id).collect();
    (store, main, tasks)
}

fn agent(store: &Store, main: Uuid, id: &str, role: AgentRole) -> Uuid {
    let new = NewAgent {
        role,
        place: AgentPlace::Subagent { id: id.into(), parent: None },
        harness: "claude".into(),
        session_id: None,
        session_cwd: None,
        model: None,
        title: None,
    };
    store.record_agent(main, &new).unwrap().id
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// Each role does its own work; the orchestrator may do any of it.
#[test]
fn the_role_must_fit_the_agent() {
    let (store, main, t) = board(1);
    let build = |agent: Uuid, role: WorkRole| store.start_work(agent, t[0], role, LinkedBy::Orchestrator, Actor::Manager);
    let reviewer = agent(&store, main, "r", AgentRole::Reviewer);
    let builder = agent(&store, main, "b", AgentRole::Builder);
    let orchestrator = agent(&store, main, "o", AgentRole::Orchestrator);
    assert_eq!(refused(build(reviewer, WorkRole::Build)), "role");
    assert_eq!(refused(build(builder, WorkRole::Review)), "role");
    assert_eq!(refused(build(builder, WorkRole::Land)), "role");
    build(builder, WorkRole::Build).unwrap();
    build(builder, WorkRole::Fix).unwrap();
    build(reviewer, WorkRole::Review).unwrap();
    build(orchestrator, WorkRole::Land).unwrap();
    assert_eq!(store.work_on(t[0]).unwrap().len(), 4);
}

/// A reviewer may hold many tickets, from any builder, on its own board only.
#[test]
fn a_reviewer_holds_tickets_from_any_builder() {
    let (store, main, t) = board(2);
    let reviewer = agent(&store, main, "r", AgentRole::Reviewer);
    for (i, b) in ["b1", "b2"].iter().enumerate() {
        let builder = agent(&store, main, b, AgentRole::Builder);
        store.start_work(builder, t[i], WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager).unwrap();
        store.start_work(reviewer, t[i], WorkRole::Review, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    }
    assert_eq!(store.work_of(reviewer).unwrap().len(), 2);
    let repo = store.register_repository_for_test("other");
    let other = store.ensure_main_workspace(repo).unwrap().id;
    let elsewhere = store.create_task(other, "theirs", Actor::Manager).unwrap().id;
    let r = store.start_work(reviewer, elsewhere, WorkRole::Review, LinkedBy::Orchestrator, Actor::Manager);
    assert_eq!(refused(r), "other_board");
}

/// Ending keeps the first ending; starting again reopens, and both are in the
/// ticket's timeline.
#[test]
fn work_ends_once_and_reopens() {
    let (store, main, t) = board(1);
    let reviewer = agent(&store, main, "r", AgentRole::Reviewer);
    store.start_work(reviewer, t[0], WorkRole::Review, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    let ended = store.end_work(reviewer, t[0], WorkRole::Review, Verdict::ChangesAsked, Actor::Manager).unwrap();
    assert_eq!(ended.verdict, Some(Verdict::ChangesAsked));
    let again = store.end_work(reviewer, t[0], WorkRole::Review, Verdict::Approved, Actor::Manager).unwrap();
    assert_eq!((again.ended_at, again.verdict), (ended.ended_at, Some(Verdict::ChangesAsked)));
    let reopened = store.start_work(reviewer, t[0], WorkRole::Review, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    assert_eq!((reopened.ended_at, reopened.verdict), (None, None));
    let said: Vec<String> = store.line_events(LineSubject::Ticket(t[0]), 0).unwrap().into_iter().map(|e| e.body).collect();
    assert_eq!(said.iter().filter(|b| b.starts_with("A reviewer started")).count(), 2, "{said:?}");
    assert!(said.contains(&"A reviewer asked for changes.".to_string()));
    let missing = store.end_work(reviewer, t[0], WorkRole::Fix, Verdict::Finished, Actor::Manager);
    assert!(matches!(missing, Err(DomainError::NotFound)));
}

/// The orchestrator recording a subagent on a ticket ends the link the runner
/// guessed from its description to another ticket.
#[test]
fn the_orchestrators_link_relinks_a_guess() {
    let (store, main, t) = board(2);
    let builder = agent(&store, main, "b", AgentRole::Builder);
    store.start_work(builder, t[0], WorkRole::Build, LinkedBy::Description, Actor::Runner).unwrap();
    store.start_work(builder, t[1], WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    let guess = &store.work_on(t[0]).unwrap()[0];
    assert_eq!((guess.linked_by, guess.verdict), (LinkedBy::Description, Some(Verdict::Relinked)));
    assert!(store.work_on(t[1]).unwrap()[0].ended_at.is_none());
}

/// An agent that ended takes no new work.
#[test]
fn an_ended_agent_takes_no_work() {
    let (store, main, t) = board(1);
    let builder = agent(&store, main, "b", AgentRole::Builder);
    store.end_agent(builder, Verdict::Finished).unwrap();
    let r = store.start_work(builder, t[0], WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager);
    assert_eq!(refused(r), "agent_ended");
}

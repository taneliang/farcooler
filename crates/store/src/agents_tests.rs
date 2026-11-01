use super::*;

use crate::ticket_work::{LinkedBy, WorkRole};

fn board() -> (Store, Uuid, Uuid) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "card", Actor::Manager).unwrap().id;
    (store, main, task)
}

fn subagent(id: &str, role: AgentRole) -> NewAgent {
    NewAgent {
        role,
        place: AgentPlace::Subagent { id: id.into(), parent: None },
        harness: "claude".into(),
        session_id: None,
        session_cwd: None,
        model: Some("opus".into()),
        title: Some("Build the store".into()),
    }
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The dispatch and the start hook may both record one subagent: the second
/// fills in what is new and keeps the rest.
#[test]
fn a_subagent_recorded_twice_is_one_agent() {
    let (store, main, _) = board();
    let first = store.record_agent(main, &subagent("a1", AgentRole::Builder)).unwrap();
    assert_eq!(first.role, AgentRole::Builder);
    assert_eq!(first.place.kind(), "subagent");
    let parent = Uuid::now_v7();
    let again = NewAgent {
        place: AgentPlace::Subagent { id: "a1".into(), parent: Some(parent) },
        session_id: Some("s".into()),
        model: None,
        title: None,
        ..subagent("a1", AgentRole::Builder)
    };
    let second = store.record_agent(main, &again).unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.place, AgentPlace::Subagent { id: "a1".into(), parent: Some(parent) });
    assert_eq!(second.session_id.as_deref(), Some("s"));
    assert_eq!((second.model.as_str(), second.title.as_str()), ("opus", "Build the store"), "kept");
    assert_eq!(store.subagent("Claude", "a1").unwrap().map(|a| a.id), Some(first.id));
    assert_eq!(store.agents(main, false).unwrap().len(), 1);
}

/// A pane has one open agent at a time; once it ends, the pane's next agent
/// is a new one.
#[test]
fn a_pane_starts_a_new_agent_after_its_last_ends() {
    let (store, main, _) = board();
    let terminal = Uuid::now_v7();
    let pane = NewAgent { place: AgentPlace::Pane { terminal }, harness: "cursor".into(), ..subagent("", AgentRole::Lander) };
    let first = store.record_agent(main, &pane).unwrap();
    assert_eq!(store.record_agent(main, &pane).unwrap().id, first.id);
    assert_eq!(store.pane_agent(terminal).unwrap().map(|a| a.id), Some(first.id));
    store.end_agent(first.id, Verdict::Finished).unwrap();
    assert_eq!(store.pane_agent(terminal).unwrap(), None);
    let next = store.record_agent(main, &pane).unwrap();
    assert_ne!(next.id, first.id);
    assert_eq!(store.agents(main, false).unwrap().len(), 1, "only the open one");
    assert_eq!(store.agents(main, true).unwrap().len(), 2);
}

/// Any harness is a short word; it is stored lowercase.
#[test]
fn a_harness_is_any_short_word() {
    let (store, main, _) = board();
    let with = |harness: &str| store.record_agent(main, &NewAgent { harness: harness.into(), ..subagent(harness, AgentRole::Builder) });
    assert_eq!(with("Cursor").unwrap().harness, "cursor");
    assert_eq!(refused(with("two words")), "harness");
    assert_eq!(refused(with(" ")), "harness");
    assert_eq!(refused(store.record_agent(main, &subagent("", AgentRole::Builder))), "agent_id");
    assert!(matches!(store.record_agent(Uuid::now_v7(), &subagent("a", AgentRole::Builder)), Err(DomainError::NotFound)));
}

/// Asking an agent to stop keeps the first ask; an agent that ended can't be
/// asked.
#[test]
fn asking_to_stop_keeps_the_first_ask() {
    let (store, main, _) = board();
    let a = store.record_agent(main, &subagent("a1", AgentRole::Builder)).unwrap();
    let asked = store.ask_agent_stop(a.id, Actor::User).unwrap();
    assert_eq!(asked.stop_asked_by.as_deref(), Some("user"));
    let at = asked.stop_asked_at.expect("when");
    let again = store.ask_agent_stop(a.id, Actor::Manager).unwrap();
    assert_eq!((again.stop_asked_by.as_deref(), again.stop_asked_at), (Some("user"), Some(at)));
    store.end_agent(a.id, Verdict::Stopped).unwrap();
    assert_eq!(refused(store.ask_agent_stop(a.id, Actor::User)), "agent_ended");
}

/// Ending an agent ends the tickets it still works, with the same reason, and
/// a second ending changes nothing.
#[test]
fn ending_an_agent_ends_its_work() {
    let (store, main, task) = board();
    let a = store.record_agent(main, &subagent("a1", AgentRole::Builder)).unwrap();
    store.start_work(a.id, task, WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    let ended = store.end_agent(a.id, Verdict::Failed).unwrap();
    assert_eq!(ended.end_reason, Some(Verdict::Failed));
    let work = store.work_of(a.id).unwrap();
    assert_eq!((work[0].ended_at, work[0].verdict), (ended.ended_at, Some(Verdict::Failed)));
    let again = store.end_agent(a.id, Verdict::Finished).unwrap();
    assert_eq!((again.ended_at, again.end_reason), (ended.ended_at, Some(Verdict::Failed)));
}

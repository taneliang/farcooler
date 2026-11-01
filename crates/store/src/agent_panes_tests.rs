//! A dispatched pane (ov-872): recorded as an agent on its tickets, which go
//! on the line, and the first of which a builder starts building.

use super::*;

use crate::models::Task;
use crate::ticket_lines::NewTicketLine;

fn board(n: usize) -> (Store, Uuid, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let worktree = store.create_worktree(repo, "main", "/wt/main", true).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, worktree, tasks)
}

fn start(terminal: Uuid, role: AgentRole, tickets: &[Uuid], worktree: Uuid) -> PaneStart {
    PaneStart {
        terminal,
        preset: "codex:gpt-5".into(),
        role,
        tickets: tickets.to_vec(),
        worktree: Some(worktree),
        worktree_path: "/wt/b".into(),
        branch: "ov-1-b".into(),
        title: String::new(),
    }
}

/// A builder's pane works its tickets in order: both go on the line, the first
/// starts building on the pane's branch, and the pane is the one a message to
/// either reaches. Its terminal exiting ends it and its work.
#[test]
fn a_builder_s_pane_works_its_tickets() {
    let (store, main, worktree, t) = board(2);
    let terminal = store.create_terminal_for_test(worktree, main);
    let agent = store.start_pane_agent(main, &start(terminal, AgentRole::Builder, &[t[0].id, t[1].id], worktree), Actor::Runner).unwrap();
    assert_eq!((agent.harness.as_str(), agent.model.as_str()), ("codex", "gpt-5"));
    let first = store.ticket_line(t[0].id).unwrap();
    assert_eq!((first.stage, first.branch.as_str(), first.worktree_id), (Stage::Building, "ov-1-b", Some(worktree)));
    assert_eq!(store.ticket_line(t[1].id).unwrap().stage, Stage::Waiting, "the next waits its turn");
    assert_eq!(store.tickets_of_pane(terminal).unwrap(), [t[0].id, t[1].id]);
    assert_eq!(store.pane_of_ticket(t[1].id).unwrap(), Some(terminal));

    store.end_pane_agent(terminal).unwrap();
    assert!(store.pane_agent(terminal).unwrap().is_none());
    assert_eq!(store.pane_of_ticket(t[0].id).unwrap(), None);
    assert!(store.work_on(t[0].id).unwrap().iter().all(|w| w.ended_at.is_some()));
}

/// A reviewer's pane reviews: its ticket stays where it is, and keeps the
/// branch it had.
#[test]
fn a_reviewer_s_pane_leaves_the_stage_alone() {
    let (store, main, worktree, t) = board(1);
    store.put_on_line(t[0].id, &NewTicketLine { branch: "theirs".into(), ..Default::default() }, Actor::Manager).unwrap();
    let terminal = store.create_terminal_for_test(worktree, main);
    let agent = store.start_pane_agent(main, &start(terminal, AgentRole::Reviewer, &[t[0].id], worktree), Actor::Runner).unwrap();
    let line = store.ticket_line(t[0].id).unwrap();
    assert_eq!((line.stage, line.branch.as_str()), (Stage::Waiting, "theirs"));
    assert_eq!(store.work_of(agent.id).unwrap()[0].role, WorkRole::Review);
}

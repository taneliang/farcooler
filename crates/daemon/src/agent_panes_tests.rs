//! A pane opened as an agent (ov-457, ov-872), on a scratch runner.

use super::*;

use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::Stage;
use farcooler_store::ticket_work::WorkRole;

struct Runner {
    _dir: crate::test_support::ScratchDir,
    svc: std::sync::Arc<crate::service::Service>,
    workspace: Uuid,
    checkout: Uuid,
}

async fn a_runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let checkout = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap().id;
    Runner { _dir: dir, svc, workspace, checkout }
}

impl Runner {
    fn task(&self, title: &str) -> farcooler_store::models::Task {
        self.svc.store.create_task(self.workspace, title, Actor::Manager).unwrap()
    }
}

fn dispatch(role: pb::AgentRole, more: &[&str]) -> pb::AgentDispatch {
    pb::AgentDispatch { role: role as i32, more_tickets: more.iter().map(|k| k.to_string()).collect(), title: String::new() }
}

fn refused<T: std::fmt::Debug>(r: Result<T>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A dispatch is refused without a task to put it on, with a ticket that isn't
/// on the task's board, and without a role; asking for none is no agent.
#[tokio::test]
async fn a_dispatch_must_name_tickets_on_the_board() {
    let r = a_runner().await;
    let (one, two) = (r.task("One"), r.task("Two"));
    let store = &r.svc.store;
    assert!(resolve(store, Some(one.id), None).unwrap().is_none());
    let both = resolve(store, Some(one.id), Some(&dispatch(pb::AgentRole::Builder, &[&two.key]))).unwrap().unwrap();
    assert_eq!(both.role, AgentRole::Builder);
    assert_eq!(both.tickets.iter().map(|t| t.id).collect::<Vec<_>>(), vec![one.id, two.id]);
    let again = resolve(store, Some(one.id), Some(&dispatch(pb::AgentRole::Builder, &[&one.key]))).unwrap().unwrap();
    assert_eq!(again.tickets.len(), 1, "the first ticket named again is not worked twice");
    assert_eq!(refused(resolve(store, Some(one.id), Some(&dispatch(pb::AgentRole::Builder, &["nope-1"])))), "task_key");
    assert_eq!(refused(resolve(store, None, Some(&dispatch(pb::AgentRole::Builder, &[])))), "task_key", "no task");
    assert_eq!(refused(resolve(store, Some(one.id), Some(&dispatch(pb::AgentRole::Unspecified, &[])))), "role");
}

/// The brief names the role and each ticket in order, and how to read one.
#[tokio::test]
async fn the_brief_names_every_ticket_in_order() {
    let r = a_runner().await;
    let (one, two) = (r.task("Sidebar"), r.task("Toolbar"));
    let d = resolve(&r.svc.store, Some(two.id), Some(&dispatch(pb::AgentRole::Builder, &[&one.key]))).unwrap().unwrap();
    let said = brief(&d, "fc");
    let (k1, k2) = (&one.key, &two.key);
    assert!(said.contains(&format!("This pane is a builder, working {k2} (Toolbar), then {k1} (Sidebar).")), "{said}");
    assert!(said.contains("fc task show <key>") && said.contains("fc agent list"), "{said}");
    assert!(!said.to_lowercase().contains("lane"), "{said}");
    let prompt = with_brief(Some(&d), Some("Opening.".into())).unwrap();
    assert!(prompt.starts_with("Opening.\n\nThis pane is a builder"), "{prompt}");
    assert_eq!(with_brief(None, Some("Opening.".into())).as_deref(), Some("Opening."));
}

/// Opening a pane as a builder records it as an agent working its tickets,
/// moves the first to building on the pane's worktree, and the pane exports
/// its role; with nobody recording anything by hand.
#[tokio::test]
async fn a_dispatched_pane_is_recorded_and_exports_its_role() {
    let r = a_runner().await;
    let (one, two) = (r.task("One"), r.task("Two"));
    let d = dispatch(pb::AgentRole::Builder, &[&two.key]);
    let pane = r.svc.create_terminal_as_agent(r.checkout, "Agent", "codex", None, Some(one.id), Some(&d)).await.unwrap();
    let agent = r.svc.store.pane_agent(pane.id).unwrap().expect("the pane is an agent");
    assert_eq!((agent.role, agent.harness.as_str(), agent.workspace_id), (AgentRole::Builder, "codex", r.workspace));
    assert_eq!(r.svc.store.tickets_of_pane(pane.id).unwrap(), vec![one.id, two.id]);
    let work = r.svc.store.work_of(agent.id).unwrap();
    assert!(work.iter().all(|w| w.role == WorkRole::Build), "{work:?}");
    let first = r.svc.store.ticket_line(one.id).unwrap();
    assert_eq!(first.stage, Stage::Building);
    assert_eq!(first.worktree_id, Some(r.checkout));
    assert_eq!(r.svc.store.ticket_line(two.id).unwrap().stage, Stage::Waiting, "only the first starts building");
    assert_eq!(env(&r.svc.store, &pane), Some(("FARCOOLER_ROLE".to_string(), "builder".to_string())));
    assert_eq!(pane.task_id, Some(one.id), "the pane starts on the ticket it was opened for");
    assert_eq!(r.svc.store.pane_of_ticket(two.id).unwrap(), Some(pane.id));
}

/// A reviewer's pane works its ticket as a review and moves nothing.
#[tokio::test]
async fn a_reviewer_pane_reviews_and_moves_nothing() {
    let r = a_runner().await;
    let one = r.task("One");
    let d = dispatch(pb::AgentRole::Reviewer, &[]);
    let pane = r.svc.create_terminal_as_agent(r.checkout, "Agent", "claude", None, Some(one.id), Some(&d)).await.unwrap();
    let agent = r.svc.store.pane_agent(pane.id).unwrap().unwrap();
    assert_eq!(r.svc.store.work_of(agent.id).unwrap().iter().map(|w| w.role).collect::<Vec<_>>(), vec![WorkRole::Review]);
    assert_eq!(r.svc.store.ticket_line(one.id).unwrap().stage, Stage::Waiting);
    assert_eq!(env(&r.svc.store, &pane), Some(("FARCOOLER_ROLE".to_string(), "reviewer".to_string())));
}

/// A dispatch naming a ticket off the board opens nothing.
#[tokio::test]
async fn a_refused_dispatch_opens_no_pane() {
    let r = a_runner().await;
    let one = r.task("One");
    let before = r.svc.store.list_terminals_for_worktree(r.checkout).unwrap().len();
    let d = dispatch(pb::AgentRole::Builder, &["nope-9"]);
    let opened = r.svc.create_terminal_as_agent(r.checkout, "Agent", "claude", None, Some(one.id), Some(&d)).await;
    assert_eq!(refused(opened), "task_key");
    assert_eq!(r.svc.store.list_terminals_for_worktree(r.checkout).unwrap().len(), before);
    assert!(r.svc.store.ticket_lines(r.workspace).unwrap().is_empty(), "nothing went on the line");
}

/// A pane that isn't an agent exports no role.
#[tokio::test]
async fn a_plain_pane_has_no_role() {
    let r = a_runner().await;
    let one = r.task("One");
    let pane = r.svc.create_terminal_with_prompt(r.checkout, "Agent", "claude", None, Some(one.id)).await.unwrap();
    assert_eq!(env(&r.svc.store, &pane), None);
    assert!(r.svc.store.pane_agent(pane.id).unwrap().is_none());
}

/// Removing an agent's pane ends the agent and its work, so it exports no
/// role and no ticket shows it still working.
#[tokio::test]
async fn a_removed_pane_has_ended_its_agent() {
    let r = a_runner().await;
    let one = r.task("One");
    let d = dispatch(pb::AgentRole::Builder, &[]);
    let pane = r.svc.create_terminal_as_agent(r.checkout, "Agent", "claude", None, Some(one.id), Some(&d)).await.unwrap();
    assert_eq!(r.svc.store.pane_of_ticket(one.id).unwrap(), Some(pane.id));
    r.svc.stop_terminal(pane.id).await.unwrap();
    r.svc.remove_terminal(pane.id).await.unwrap();
    assert_eq!(r.svc.store.pane_of_ticket(one.id).unwrap(), None);
    assert!(r.svc.store.pane_agent(pane.id).unwrap().is_none());
}

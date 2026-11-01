//! A pane opened as an agent (ov-457, ov-872) as a client meets it:
//! `terminal.create` with an `agent` over a real socket, through the real
//! dispatch table, with every agent launch stubbed. The runner records the
//! pane as that agent working its tickets, puts them on the line, exports its
//! role, and lists it on the plan; a ticket off the board is refused before
//! anything is made.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::{BOARD_LINE, BOARD_PLAN, TERMINAL_TASK};
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, request as payload, result};
use farcooler_store::agents::{AgentPlace, AgentRole};
use farcooler_store::models::Actor;
use farcooler_store::ticket_lines::Stage;
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

/// A real git repository registered with `h`'s runner: its Main workspace and
/// its main checkout, which a pane needs on disk.
async fn a_checkout(h: &Harness, dir: &std::path::Path) -> (Uuid, Uuid) {
    let repo = dir.join("repos").join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "."][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        assert!(std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
    }
    h.service.add_root(repo.parent().unwrap()).await.expect("root");
    let repository = h.service.register_repository(&repo).await.expect("registered").id;
    let workspace = h.service.store.ensure_main_workspace(repository).unwrap().id;
    let worktree = h.service.store.list_worktrees_for_repository(repository).unwrap().into_iter().next().expect("the main checkout");
    (workspace, worktree.id)
}

fn create(worktree: Uuid, key: &str, agent: pb::AgentDispatch) -> pb::Request {
    let mut r = request("terminal.create");
    r.target_resource_id = Some(id(worktree));
    r.required_capabilities = vec![TERMINAL_TASK.into(), BOARD_LINE.into()];
    r.payload = Some(payload::Payload::TerminalCreate(pb::TerminalCreate {
        title: "builder".into(),
        command_preset: "claude".into(),
        join_active_group: false,
        prompt: None,
        task_key: Some(key.into()),
        agent: Some(agent),
    }));
    r
}

/// What tmux was told to run in every pane on the runner's server.
async fn start_commands(h: &Harness) -> Vec<String> {
    let out = h.service.tmux.run(&["list-panes", "-a", "-F", "#{pane_start_command}"]).await.expect("tmux answered");
    out.stdout.lines().map(str::to_string).collect()
}

/// A builder dispatched on two tickets: the pane is recorded as a builder
/// working both in order, the first goes to building on the pane's worktree,
/// the pane runs with `FARCOOLER_ROLE=builder`, and the plan lists the agent
/// with its terminal and tickets.
#[tokio::test]
async fn a_pane_opened_as_a_builder_is_recorded_as_that_agent() {
    let h = start(Scope::Control).await;
    let dir = tempfile::Builder::new().prefix("ap").tempdir_in("/tmp").unwrap();
    let (workspace, worktree) = a_checkout(&h, dir.path()).await;
    let store = &h.service.store;
    let first = store.create_task(workspace, "Mac: first", Actor::User).unwrap();
    let second = store.create_task(workspace, "Mac: second", Actor::User).unwrap();
    let mut link = connect(&h).await;

    let dispatch = pb::AgentDispatch {
        role: pb::AgentRole::Builder as i32,
        more_tickets: vec![second.key.clone()],
        title: "Mac polish".into(),
    };
    let made = link.call(create(worktree, &first.key, dispatch)).await.expect("terminal.create");
    let Some(result::Value::Terminal(term)) = made.value else { panic!("wrong result") };
    let terminal = Uuid::from_slice(&term.id).unwrap();

    let agent = store.pane_agent(terminal).unwrap().expect("the pane is an agent");
    assert_eq!((agent.role, agent.harness.as_str(), agent.title.as_str()), (AgentRole::Builder, "claude", "Mac polish"));
    assert_eq!(agent.place, AgentPlace::Pane { terminal });
    assert_eq!(store.tickets_of_pane(terminal).unwrap(), vec![first.id, second.id], "its tickets, in order");
    let line = store.ticket_line(first.id).unwrap();
    assert_eq!((line.stage, line.worktree_id), (Stage::Building, Some(worktree)), "a builder starts on the first");
    assert_eq!(store.ticket_line(second.id).unwrap().stage, Stage::Waiting, "the rest wait their turn");
    assert_eq!(store.pane_of_ticket(second.id).unwrap(), Some(terminal));

    let commands = start_commands(&h).await;
    assert!(commands.iter().any(|c| c.contains("FARCOOLER_ROLE=builder")), "the pane knows its role: {commands:?}");

    let mut get = request("plan.get");
    get.required_capabilities = vec![BOARD_PLAN.into(), BOARD_LINE.into()];
    get.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(workspace), include_closed: false }));
    let Some(result::Value::Plan(plan)) = link.call(get).await.expect("plan.get").value else { panic!("wrong result") };
    let row = plan.agents.iter().find(|a| a.id == id(agent.id)).unwrap_or_else(|| panic!("not listed: {:?}", plan.agents));
    assert_eq!(row.terminal_id, Some(id(terminal)));
    assert_eq!(row.tickets.iter().map(|t| t.task_id.clone()).collect::<Vec<_>>(), vec![id(first.id), id(second.id)]);
    assert!(row.tickets.iter().all(|t| t.role == pb::WorkRole::Build as i32));
}

/// A ticket off the first one's board, and a dispatch with no role, are
/// refused before any pane is made.
#[tokio::test]
async fn a_dispatch_off_the_board_or_without_a_role_makes_nothing() {
    let h = start(Scope::Control).await;
    let dir = tempfile::Builder::new().prefix("ap").tempdir_in("/tmp").unwrap();
    let (workspace, worktree) = a_checkout(&h, dir.path()).await;
    let store = &h.service.store;
    let first = store.create_task(workspace, "Mac: first", Actor::User).unwrap();
    let mut link = connect(&h).await;

    let off_board = pb::AgentDispatch { role: pb::AgentRole::Reviewer as i32, more_tickets: vec!["zz-404".into()], title: String::new() };
    let no_role = pb::AgentDispatch { role: pb::AgentRole::Unspecified as i32, more_tickets: vec![], title: String::new() };
    for (dispatch, word) in [(off_board, "task_key"), (no_role, "role")] {
        match link.call(create(worktree, &first.key, dispatch)).await {
            Err(ClientError::Daemon { code, what, .. }) => {
                assert_eq!((code, what.as_str()), (ErrorCode::InvalidArgument as i32, word));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
    assert!(start_commands(&h).await.is_empty(), "no pane was made");
    assert!(store.agents(workspace, true).unwrap().is_empty(), "no agent was recorded");
    assert!(store.ticket_line(first.id).is_err(), "nothing went on the line");
}

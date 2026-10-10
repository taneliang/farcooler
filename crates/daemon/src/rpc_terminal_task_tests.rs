//! `terminal.create` for a task, through the daemon's own handler: see
//! `rpc.rs`, where this module is declared.

use farcooler_protocol::v1::{Request, Scope, TerminalCreate, response};
use farcooler_transport::Handler;

use super::*;
use crate::service::test_agent;

async fn a_handler() -> (crate::test_support::ScratchDir, Arc<Service>, RpcFactory, models::Worktree) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    crate::reconcile::repository(&svc, repo).await.unwrap();
    let ws = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().next().expect("the main checkout");
    let factory = RpcFactory::new(
        svc.clone(),
        crate::watch::Watcher::new(svc.clone()),
        Arc::new(tokio::sync::Notify::new()),
        Peer { client_id: None, scope: Scope::HostAdmin },
    );
    (dir, svc, factory, ws)
}

fn create(ws: Uuid, preset: &str, key: Option<&str>, join: bool) -> Request {
    Request {
        method: "terminal.create".into(),
        target_resource_id: Some(crate::wire::id_bytes(ws)),
        required_capabilities: vec![farcooler_protocol::capability::TERMINAL_TASK.into()],
        payload: Some(request::Payload::TerminalCreate(TerminalCreate {
            title: "worker".into(),
            command_preset: preset.into(),
            join_active_group: join,
            prompt: None,
            task_key: key.map(str::to_string),
            lane: None,
        })),
        ..Default::default()
    }
}

async fn terminal(factory: &RpcFactory, req: Request) -> farcooler_protocol::v1::Terminal {
    match factory.handle(req).await.outcome {
        Some(response::Outcome::Result(r)) => match r.value {
            Some(result::Value::Terminal(t)) => t,
            other => panic!("not a terminal: {other:?}"),
        },
        other => panic!("refused: {other:?}"),
    }
}

/// `#{window_id} #{pane_start_command}` for every pane on the fixture's server.
async fn panes(svc: &Service) -> Vec<String> {
    let out = svc
        .tmux
        .run(&["list-panes", "-a", "-F", "#{window_id} #{pane_start_command}"])
        .await
        .expect("tmux answered");
    out.stdout.lines().map(str::to_string).collect()
}

#[tokio::test]
async fn a_terminal_for_a_task_on_its_own_board_exports_the_key() {
    let (_dir, svc, factory, ws) = a_handler().await;
    let main = svc.store.ensure_main_workspace(ws.repository_id).unwrap();
    let task = svc.store.create_task(main.id, "the work", models::Actor::User).unwrap();

    let made = terminal(&factory, create(ws.id, "claude", Some(&task.key), false)).await;
    assert_eq!(made.task_id.as_deref(), Some(task.id.as_bytes().as_slice()), "the terminal says which task");

    let panes = panes(&svc).await;
    assert!(panes.iter().all(|p| p.contains(test_agent::MARKER)), "the stub: {panes:?}");
    assert!(
        panes.iter().any(|c| c.contains(&format!("FARCOOLER_TASK={}", task.key))),
        "the pane names its task: {panes:?}"
    );
}

/// `join_active_group` makes the pane by splitting the focused one: the
/// other of the two first launches, and the one `prefix %` takes.
#[tokio::test]
async fn a_terminal_split_into_the_layout_for_a_task_exports_the_key() {
    let (_dir, svc, factory, ws) = a_handler().await;
    let main = svc.store.ensure_main_workspace(ws.repository_id).unwrap();
    let task = svc.store.create_task(main.id, "the work", models::Actor::User).unwrap();
    // A layout to join: one ordinary pane first.
    terminal(&factory, create(ws.id, "shell", None, false)).await;

    terminal(&factory, create(ws.id, "claude", Some(&task.key), true)).await;

    let panes = panes(&svc).await;
    assert_eq!(panes.len(), 2, "{panes:?}");
    let window = |line: &str| line.split_whitespace().next().unwrap_or_default().to_string();
    assert_eq!(window(&panes[0]), window(&panes[1]), "a split, in the same window: {panes:?}");
    assert!(
        panes.iter().any(|c| c.contains(&format!("FARCOOLER_TASK={}", task.key))
            && c.contains(test_agent::MARKER)),
        "the split pane names its task, and runs the stub: {panes:?}"
    );
}

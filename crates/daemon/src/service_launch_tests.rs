//! Every launch of an agent into a pane is a launch of its own (ov-886): a
//! fresh id, recorded as the pane's current one and exported to it, so a hook
//! from the process a restart replaced reaches nothing. On a real tmux server,
//! reading `#{pane_start_command}` back.

use farcooler_protocol::v1::TerminalIntent;
use uuid::Uuid;

use super::restart_wiring_tests::{a_worktree, pane_start_command};
use super::*;

/// The launch id a pane was started with, read off its command.
fn exported(command: &str) -> Option<Uuid> {
    let (_, rest) = command.split_once(&format!("{}=", farcooler_core::pane_env::LAUNCH))?;
    Uuid::parse_str(rest.split_whitespace().next()?).ok()
}

/// A claude pane on the record, naming a session, with no pane yet.
fn a_claude_pane(svc: &Service, ws: &models::Worktree) -> Uuid {
    let term = svc.store.create_terminal(ws.id, "agent", "claude", TerminalIntent::Running, 80, 24).unwrap();
    svc.store
        .set_pane_mode(term.id, term.resource_version, models::PaneMode::Terminal, Some(Uuid::now_v7().to_string()), false)
        .unwrap();
    term.id
}

#[tokio::test]
async fn each_restart_exports_a_fresh_launch_and_retires_the_one_before() {
    let (_dir, svc, ws) = a_worktree().await;
    let id = a_claude_pane(&svc, &ws);

    svc.restart_terminal(id).await.expect("restart");
    let first = exported(&pane_start_command(&svc, id).await).expect("the launch exports its id");
    assert_eq!(svc.store.current_launch(id).unwrap(), Some(first), "and it is the pane's current launch");

    svc.restart_terminal(id).await.expect("restart again");
    let second = exported(&pane_start_command(&svc, id).await).expect("so does the next");
    assert_ne!(second, first);
    assert_eq!(svc.store.current_launch(id).unwrap(), Some(second));
    assert_eq!(svc.store.terminal_for_launch(first).unwrap(), None, "the restart retired the first launch");
}

#[tokio::test]
async fn a_created_agent_pane_exports_its_launch_and_a_shell_none() {
    let (_dir, svc, ws) = a_worktree().await;
    let agent = svc.create_terminal(ws.id, "agent", "claude").await.expect("an agent pane");
    let launch = exported(&pane_start_command(&svc, agent.id).await).expect("the launch exports its id");
    assert_eq!(svc.store.current_launch(agent.id).unwrap(), Some(launch));

    let shell = svc.create_terminal(ws.id, "shell", "shell").await.expect("a shell");
    let command = pane_start_command(&svc, shell.id).await;
    assert_eq!(exported(&command), None, "a shell runs no agent and has no hooks to fence: {command}");
    assert_eq!(svc.store.current_launch(shell.id).unwrap(), None);
}

/// Kill pane `id` from inside the restart, just before it spawns, so the
/// respawn fails at tmux after the new launch was recorded. Then `also`.
fn fail_the_spawn(svc: &Arc<Service>, id: Uuid, also: impl FnOnce() + Send + 'static) {
    let tmux = svc.tmux.clone();
    let inventory = svc.inventory.clone();
    *svc.claude_logins.before_spawn.lock().unwrap() = Some(Box::new(move || {
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let snapshot = inventory.refresh().await;
                let pane = snapshot.claimants(id).into_iter().next().expect("a pane").pane_id.clone();
                tmux.run(&["kill-pane", "-t", &pane]).await.expect("killed");
            })
        })
        .join()
        .unwrap();
        also();
    }));
}

#[tokio::test]
async fn a_restart_whose_spawn_fails_puts_back_the_launch_it_replaced() {
    let (_dir, svc, ws) = a_worktree().await;
    let id = a_claude_pane(&svc, &ws);
    svc.restart_terminal(id).await.expect("restart");
    let running = svc.store.current_launch(id).unwrap().expect("a launch");

    fail_the_spawn(&svc, id, || {});
    svc.restart_terminal(id).await.expect_err("the pane it would respawn is gone");
    assert_eq!(svc.store.current_launch(id).unwrap(), Some(running), "the failed restart's launch was rolled back");
}

/// The rollback a failed restart does never undoes a launch that came after
/// it: here, one recorded while the failed restart was spawning.
#[tokio::test]
async fn a_failed_restart_leaves_a_newer_launch_alone() {
    let (_dir, svc, ws) = a_worktree().await;
    let id = a_claude_pane(&svc, &ws);
    svc.restart_terminal(id).await.expect("restart");

    let newer = Arc::new(std::sync::Mutex::new(None));
    {
        let store = svc.store.clone();
        let newer = newer.clone();
        fail_the_spawn(&svc, id, move || {
            let t = store.get_terminal(id).unwrap();
            let (_, launch) = store.begin_launch(id, t.resource_version).unwrap();
            *newer.lock().unwrap() = Some(launch.id);
        });
    }
    svc.restart_terminal(id).await.expect_err("the pane it would respawn is gone");
    let newer = newer.lock().unwrap().expect("the newer launch was recorded");
    assert_eq!(svc.store.current_launch(id).unwrap(), Some(newer), "the newer launch survived the rollback");
}

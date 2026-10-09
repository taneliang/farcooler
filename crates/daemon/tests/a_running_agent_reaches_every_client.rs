//! `Terminal.running_agent` over the socket (ov-443): the agent the watcher
//! saw in a pane is what a list says, so a claude typed into a shell, whose
//! `command_preset` is `shell` and whose `current_command` is its session's
//! title, is still offered the conversation view.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::{Scope, result};
use farcooler_transport::request;
use in_process::*;

/// Goes red when `with_activity` stops reading the watcher's agent: the list
/// then says nothing runs in a pane where claude does.
#[tokio::test]
async fn a_list_names_the_agent_the_watcher_saw() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let pane = a_pane(&h, repo.worktree, None);
    let quiet = a_pane(&h, repo.worktree, None);
    h.watcher.running_agent_for_tests(pane, Some("claude")).await;

    let Some(result::Value::TerminalList(list)) =
        connect(&h).await.call(request("terminal.list")).await.expect("terminal.list").value
    else {
        panic!("wrong result")
    };
    let agent = |id: uuid::Uuid| {
        list.items.iter().find(|t| t.id.as_ref() == id.as_bytes()).map(|t| t.running_agent.clone())
    };
    assert_eq!(agent(pane), Some(Some("claude".to_string())), "the agent the watcher saw");
    assert_eq!(agent(quiet), Some(None), "nothing seen is absent, never a guess");
}

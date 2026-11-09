//! A running engine agent's terminal, found by workspace and item, for watching only (ov-972).
//!
//! The engine runs each native build in a tmux server of its own, which the daemon's `Runtime` never
//! lists. So no terminal id the wire knows can name that pane, and `terminal.write`, `terminal.resize`,
//! `terminal.tell` and `terminal.paste_file` cannot reach it. This registry is the one door in, and it
//! hands out a `Runtime` for `attach` (output only) and nothing that types.
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use farcooler_core::inventory::TaggedPane;
use farcooler_tmux::{LiveInventory, TmuxServer};

use crate::runtime::Runtime;

type Key = (String, String);
static LIVE: LazyLock<Mutex<HashMap<Key, (Runtime, TaggedPane)>>> = LazyLock::new(Default::default);

/// Listed for as long as it lives; dropping it unlists the pane, before the server is reaped.
pub struct Listed(Key);

pub fn publish(workspace: &str, item: &str, server: TmuxServer, pane: TaggedPane) -> Listed {
    let key = (workspace.to_string(), item.to_string());
    // `marks: None`: nothing here ever records typing, because nothing here types.
    let runtime = Runtime { inventory: LiveInventory::new(server.clone()), tmux: server, marks: None };
    if let Ok(mut live) = LIVE.lock() {
        live.insert(key.clone(), (runtime, pane));
    }
    Listed(key)
}

impl Drop for Listed {
    fn drop(&mut self) {
        if let Ok(mut live) = LIVE.lock() {
            live.remove(&self.0);
        }
    }
}

/// The pane to watch, if that item's agent is running now.
pub fn lookup(workspace: &str, item: &str) -> Option<(Runtime, TaggedPane)> {
    LIVE.lock().ok()?.get(&(workspace.to_string(), item.to_string())).cloned()
}

/// The item a `workflow.watch` names: `{"item": "<key>"}` in the command payload, nothing else.
pub fn item_of(payload: &farcooler_protocol::v1::WorkflowCommand) -> Option<String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Ask { item: String }
    if payload.schema != 1 || payload.json.len() > 1024 { return None; }
    serde_json::from_str::<Ask>(&payload.json).ok().map(|a| a.item)
}

/// How often a running watch asks whether its watcher may still watch (`start`'s `still`).
pub(crate) const RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// Attach to the agent `payload` names, read-only: the owner's, or any member's (ov-1000), who reads every
/// workspace. A released device is turned away before this (`people::gate`) and here. Returns the task that streams the
/// pane to `push` and the reply that announces the pane's size ahead of its picture.
///
/// `still` is asked every `RECHECK` while the watch runs: once it says no (the person was removed, or their
/// device released), the watch ends and the watcher is told so (R-111, L2).
pub(crate) fn start(
    push: farcooler_transport::PushSender, caller: crate::people::Person, workspace: uuid::Uuid,
    payload: &farcooler_protocol::v1::WorkflowCommand, still: impl Fn() -> bool + Send + 'static,
) -> farcooler_core::Result<(tokio::task::JoinHandle<()>, farcooler_protocol::v1::TerminalAttachResult)> {
    use farcooler_core::DomainError;
    if !matches!(caller, crate::people::Person::Owner | crate::people::Person::Member(_)) {
        return Err(DomainError::ScopeDenied { needed: "owner" });
    }
    let item = item_of(payload).ok_or(DomainError::NotFound)?;
    let (runtime, pane) = lookup(&workspace.to_string(), &item).ok_or(DomainError::NotFound)?;
    let (columns, rows) = (pane.columns, pane.rows);
    let sink = crate::resync::TerminalSink::new(push.clone(), pane.terminal_id, 0);
    let ending = crate::resync::TerminalSink::new(push, pane.terminal_id, 0);
    let task = tokio::spawn(async move {
        let (watched, shown) = (runtime.clone(), pane.clone());
        let attach = runtime.attach(pane, sink);
        tokio::pin!(attach);
        let turned_away = loop {
            tokio::select! {
                _ = &mut attach => break false,
                () = tokio::time::sleep(RECHECK) => if !still() { break true },
            }
        };
        if turned_away {
            ending.watch_failed();
            return;
        }
        // Say "finished" only when tmux confirms the pane is gone. An attach that failed
        // while the agent still runs must not tell the owner it ended.
        if crate::stream_end::pane_is_over(&watched, &shown).await {
            ending.pane_gone();
        } else {
            // The attach ended and the agent still runs: tell the watcher, or it waits on a picture for ever.
            ending.watch_failed();
        }
    });
    let reply = farcooler_protocol::v1::TerminalAttachResult {
        epoch: 0, next_sequence: 0, oldest_sequence: 0, columns, rows, resume_token: String::new(),
    };
    Ok((task, reply))
}

#[cfg(test)]
#[path = "watch_view_tests.rs"]
mod tests;

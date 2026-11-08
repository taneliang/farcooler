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

#[cfg(test)]
#[path = "watch_view_tests.rs"]
mod tests;

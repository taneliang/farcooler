//! Panes bound to their claude sessions by claude's own registry (ov-365).
//!
//! Three places used to guess which conversation a pane is running, and each
//! now asks `claude_registry` first, falling back to its guess only where the
//! registry has no answer (a claude older than the registry, a pid the walk
//! could not name, a session file not yet written):
//!
//! - the watcher's log join (`watch::registry_join`), by the pane's pid;
//! - adoption when a pane switches to chat (`Service::session_to_adopt`), by
//!   the pane's pid, retiring `session_discovery`'s worktree-and-mtime guess;
//! - hook routing for a claude session no terminal row names (`bind_pane`
//!   below), by the registry's tmux pane and the process's tty. This is what
//!   makes `SessionStart` rebind: after `/clear` the pane's claude is a new
//!   session, its hooks name an id no row has, and the registry says which
//!   pane it is in.

use std::path::PathBuf;

use farcooler_core::inventory::RuntimeSnapshot;
use farcooler_store::Store;
use farcooler_store::models::PaneMode;
use uuid::Uuid;

use crate::claude_registry::Registry;

/// The terminal whose pane claude's registry places `session` in, with the
/// row's session rewritten to `session` when it named another.
///
/// The pane must match twice: the registry's tmux pane id, and the process's
/// controlling tty against the pane's. The tmux field does not name a server,
/// and every Far Cooler daemon's session is called `farcooler`, so `%3` alone
/// can be a pane in Canary's server. A chat pane is never bound: its
/// conversation arrives over its shim.
pub fn bind_pane(registry: &Registry, store: &Store, snapshot: &RuntimeSnapshot, session: &str) -> Option<Uuid> {
    let entry = registry.by_session(session)?;
    let place = entry.tmux.as_ref()?;
    let mut panes = snapshot
        .panes
        .iter()
        .filter(|p| !p.dead && p.pane_id == place.pane && registry.runs_on(&entry, &p.tty));
    let pane = panes.next()?;
    if panes.next().is_some() {
        return None;
    }
    let term = store.get_terminal(pane.terminal_id).ok()?;
    if term.pane_mode == PaneMode::Agent {
        return None;
    }
    if term.agent_session_id.as_deref() != Some(session) {
        match store.set_pane_mode(term.id, term.resource_version, term.pane_mode, Some(session.to_string()), false) {
            Ok(_) => tracing::info!(
                terminal = %term.id,
                from = ?term.agent_session_id,
                to = session,
                pid = entry.pid,
                "claude's registry moved this pane to another session"
            ),
            Err(e) => {
                tracing::warn!(error = %e, terminal = %term.id, "could not record the session claude's registry names");
                return None;
            }
        }
    }
    Some(term.id)
}

/// The session log claude's registry names for the process `pid`, for a pane
/// whose preset is claude. `None` sends the caller to its own lookup.
pub fn registered_log(registry: &Registry, preset: Option<&str>, pid: Option<i32>, cwd: &str) -> Option<PathBuf> {
    if !preset?.starts_with("claude") {
        return None;
    }
    let entry = registry.by_pid(pid?)?;
    registry.transcript(&entry, cwd)
}

/// The session to adopt for the process `pid` in a pane switching to chat:
/// the registry's, when it is live, has a transcript filed under the
/// worktree's own project directory (where the chat will look for it), and no
/// other terminal claims it.
pub fn session_to_adopt(registry: &Registry, pid: i32, worktree: &str, claimed: &[String]) -> Option<String> {
    let entry = registry.by_pid(pid)?;
    registry.transcript_in(&entry, worktree)?;
    (!claimed.contains(&entry.session_id)).then_some(entry.session_id)
}

#[cfg(test)]
#[path = "registry_binding_tests.rs"]
mod tests;

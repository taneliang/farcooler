//! Where a pane's conversation is, for `agent.rows`' first open, found the
//! way the watcher's tick finds it (ov-830): by the process in the pane's
//! foreground, claude's own registry for a claude and the rollout a codex
//! holds open. A claude typed into a shell (the owner's adopted orchestrator)
//! has no session the runner launched it on, and a codex pane none at all,
//! so without this their first page waited for the pane's next tick.

use std::path::PathBuf;

use farcooler_core::inventory::RuntimeInventory as _;
use farcooler_store::models;

use crate::claude_registry::Registry;
use crate::service::Service;

/// The conversation of the agent in `term`'s foreground, if one can be
/// named now. `None` sends the caller back to waiting for the tick.
pub(crate) async fn joined(svc: &Service, term: &models::Terminal, worktree: &str) -> Option<PathBuf> {
    let snapshot = svc.inventory.snapshot();
    let pane = snapshot.panes.iter().find(|p| p.terminal_id == term.id && !p.dead)?;
    let tty = pane.tty.trim_start_matches("/dev/").to_string();
    let foreground = crate::foreground::read().await;
    let running = foreground.pane(&tty)?;
    let agent = crate::running_agent::of(&svc.registry(), &running.command, &foreground.foreground_args(&tty));
    let (pid, registry, worktree) = (running.pid, svc.hooks().claude_registry(), worktree.to_string());
    tokio::task::spawn_blocking(move || transcript_for(&agent, pid, registry, &worktree)).await.ok().flatten()
}

/// The conversation `agent`, running as `pid`, is writing: claude's by its
/// registry entry, codex's by the rollout it holds open.
pub(crate) fn transcript_for(agent: &str, pid: i32, registry: &Registry, worktree: &str) -> Option<PathBuf> {
    if agent.starts_with("claude") {
        let entry = registry.by_pid(pid)?;
        registry.transcript(&entry, worktree)
    } else if agent.starts_with("codex") {
        crate::watch::answer_wake::codex_turn::rollout_of(pid)
    } else {
        None
    }
}

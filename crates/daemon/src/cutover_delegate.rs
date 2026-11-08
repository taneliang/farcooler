//! ov-974: the owner's orchestrator runs the cutover for the owner.
//!
//! **Decided where the rules live.** The live daemon's Prepare admits a
//! person, as before, or an agent in a pane by `Admission::capture` for that
//! pane's own workspace: the rule `workflow.move` already holds an
//! orchestrator to (ov-951). The pane must be that workspace's orchestrator,
//! on its lead's login (`refuse_another_leads_login`), and its person must be
//! the owner (`caller_person`). So a builder pane, a member's orchestrator and
//! an orchestrator on another lead's login are refused there.
//!
//! **Pinned for recovery.** A recovery daemon has no Store, inventory or
//! registry, and may not open the database it is about to prove nobody else
//! has open (`JoinedOwners`). So the live daemon writes the admitted pane down
//! here, as its root process (pid and start time) and the attempt, before it
//! marks the hold. Recovery admits a caller this names only when the attempt
//! is the one in hand and the caller's parents reach that very process before
//! any tmux server: an orphan the pane left to pid 1, another pane, and a pane
//! restarted since are all refused, and the owner runs the rest by hand.
//!
//! **No new way in.** This file sits beside the database and its hold mark,
//! and anything that can write it can already write those. The seat walk was
//! always a guardrail against an agent *asking* for the owner's steps
//! (`caller_seat`), and it still is: only a pane the live daemon admitted is
//! ever named, and a person's Prepare, a Recover and a Finish forget it.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use farcooler_core::{DomainError, Result};
use farcooler_transport::Peer;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::caller_seat::{Proc, Seat};
use crate::workflow_runtime::Admission;

/// The file, in the daemon's root, that names the admitted pane.
pub const FILE: &str = "cutover-delegate.json";

/// The owner's orchestrator, as the live daemon admitted it to `attempt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegate {
    pub attempt: String,
    pub terminal: Uuid,
    pub workspace: Uuid,
    /// The pane's root process, the one tmux started, and its start time
    /// (`claude_registry::process_incarnation`).
    pub pane_pid: i32,
    pub pane_start: i64,
}

impl Delegate {
    /// Who asked, for the journal: `caller` is the process's own note.
    pub fn note(&self, caller: &str) -> String {
        format!("the owner's orchestrator (terminal {}, workspace {}), acting for the owner: {caller}", self.terminal, self.workspace)
    }
}

fn path(root: &Path) -> PathBuf { root.join(FILE) }

/// Name `delegate`, replacing any earlier one, whole or not at all.
pub fn write(root: &Path, delegate: &Delegate) -> std::io::Result<()> {
    use std::io::Write;
    let private = root.join(format!("{FILE}.{}", std::process::id()));
    let mut f = std::fs::File::create(&private)?;
    f.write_all(&serde_json::to_vec(delegate)?)?;
    f.sync_all()?;
    std::fs::rename(&private, path(root))?;
    std::fs::File::open(root)?.sync_all()
}

/// Name no pane.
pub fn forget(root: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path(root)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The pane named for the attempt in hand, if any.
fn read(root: &Path) -> Option<Delegate> {
    let delegate: Delegate = serde_json::from_slice(&std::fs::read(path(root)).ok()?).ok()?;
    let db = crate::cutover_admin::database(root);
    (farcooler_store::engine_cutover::transfer::attempt_in_hand(&db).as_deref() == Some(delegate.attempt.as_str())).then_some(delegate)
}

/// The delegate a recovery daemon admits caller `pid` as: the one named for
/// the attempt in hand, when `pid` runs inside its pane.
pub fn admits(root: &Path, pid: i32) -> Option<Delegate> {
    let delegate = read(root)?;
    let table = crate::proc_table::snapshot()?;
    let procs = crate::caller_seat::procs(&table);
    let servers = crate::caller_seat::tmux_servers(&procs);
    within(pid, &procs, &servers, &delegate, &crate::claude_registry::process_incarnation).then_some(delegate)
}

/// Whether `pid`, or a parent of it met before any tmux server, is the
/// delegate's pane root, started when it was.
pub(crate) fn within(pid: i32, procs: &HashMap<i32, Proc>, servers: &HashSet<i32>, delegate: &Delegate, start_of: &dyn Fn(i32) -> Option<i64>) -> bool {
    let mut at = pid;
    for _ in 0..64 {
        if at <= 1 || servers.contains(&at) { return false; }
        if at == delegate.pane_pid { return start_of(at) == Some(delegate.pane_start); }
        let Some(row) = procs.get(&at) else { return false };
        if row.ppid == at { return false; }
        at = row.ppid;
    }
    false
}

/// The live daemon's Prepare admission: a person, or the owner's
/// orchestrator, with the pane to pin for recovery when it is the latter.
pub(crate) async fn admit(
    svc: &Arc<crate::service::Service>, peer: Peer, session: Arc<crate::sessions::Session>, caller_pid: Option<i32>,
    caller_pane: Arc<OnceLock<Uuid>>, attempt: &str,
) -> Result<(Admission, Option<Delegate>)> {
    let workspace = match crate::caller_seat::caller_seat(svc, caller_pid).await {
        Seat::Agent(Some(pane)) => svc.store.get_terminal(pane)?.workspace_id.ok_or(DomainError::ScopeDenied { needed: "workspace orchestrator" })?,
        _ => Admission::HOST_WIDE,
    };
    let admission = Admission::capture(svc.clone(), peer, session, caller_pid, caller_pane, workspace).await?;
    let Some(term) = admission.orchestrator() else {
        admission.require_person()?;
        return Ok((admission, None));
    };
    let pane_pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == term.id).map(|p| p.stamp.pid as i32).ok_or(DomainError::AuthRequired)?;
    let pane_start = crate::claude_registry::process_incarnation(pane_pid).ok_or(DomainError::AuthRequired)?;
    let delegate = Delegate { attempt: attempt.into(), terminal: term.id, workspace, pane_pid, pane_start };
    Ok((admission, Some(delegate)))
}

#[cfg(test)]
#[path = "cutover_delegate_tests.rs"]
mod tests;

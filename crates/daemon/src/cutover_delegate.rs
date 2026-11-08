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
//! **On the owner's word.** The live Prepare admits the orchestrator only on
//! the owner's grant, an "Allow cutover once" said from the app or their phone,
//! and spends it (`cutover_grant`); the pin names the grant, and Commit's
//! journal entry says which one. Each recovery step asks the database again,
//! read-only, whether the pane is still the workspace's orchestrator on the
//! runner's login with no member leading it
//! (`engine_cutover::startup::owners_orchestrator`), and whether the grant is
//! still under a day old and spent on this attempt alone
//! (`engine_cutover::startup::grant_holds`).
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
    /// The owner's answer that granted it, spent by this Prepare.
    pub grant: Uuid,
}

impl Delegate {
    /// Who asked, for the journal: `caller` is the process's own note.
    pub fn note(&self, caller: &str) -> String {
        format!("the owner's orchestrator (terminal {}, workspace {}), acting for the owner on their grant (answer {}): {caller}", self.terminal, self.workspace, self.grant)
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
/// the attempt in hand, when `pid` runs inside its pane and the database
/// still says that pane is the owner's orchestrator.
pub fn admits(root: &Path, pid: i32) -> Option<Delegate> {
    admits_at(root, pid, crate::step_up_rpc::now_ms())
}

/// [`admits`] at `now_ms`.
pub(crate) fn admits_at(root: &Path, pid: i32, now_ms: i64) -> Option<Delegate> {
    let delegate = pinned_for(root, pid)?;
    let db = crate::cutover_admin::database(root);
    grant_holds(&db, &delegate, now_ms).then_some(delegate)
}

/// Whether `pid` is the owner's orchestrator pinned for the attempt in hand,
/// still so by the database, and only the owner's grant no longer holds: a
/// day has passed since the answer (M1 of the second review). Nothing the
/// orchestrator does can renew it, since only a live Prepare spends a grant
/// and the live daemon is down; the owner runs Recover or Finish by hand.
pub fn lapsed(root: &Path, pid: i32) -> bool {
    lapsed_at(root, pid, crate::step_up_rpc::now_ms())
}

/// [`lapsed`] at `now_ms`.
pub(crate) fn lapsed_at(root: &Path, pid: i32, now_ms: i64) -> bool {
    pinned_for(root, pid).is_some_and(|delegate| !grant_holds(&crate::cutover_admin::database(root), &delegate, now_ms))
}

/// The delegate named for the attempt in hand, when `pid` runs inside its pane
/// and the database still says that pane is the owner's orchestrator.
fn pinned_for(root: &Path, pid: i32) -> Option<Delegate> {
    let delegate = read(root)?;
    let table = crate::proc_table::snapshot()?;
    let procs = crate::caller_seat::procs(&table);
    let servers = crate::caller_seat::tmux_servers(&procs);
    if !within(pid, &procs, &servers, &delegate, &crate::claude_registry::process_incarnation) { return None; }
    still_the_owners(&crate::cutover_admin::database(root), &delegate).then_some(delegate)
}

/// What a recovery daemon says to the owner's orchestrator whose grant lapsed.
pub const LAPSED: &str = "The owner's “Allow cutover once” is more than a day old, so their orchestrator can't go on with this cutover. Only the owner can finish it now: they run Recover or Finish themselves, from a terminal of their own.";

/// Whether the owner's grant that admitted the delegate still does at
/// `now_ms`: under `cutover_grant::MAX_AGE_MS` old, and spent on this attempt
/// alone. Asked at every recovery step, as at Prepare. One that can't be read
/// says no.
pub(crate) fn grant_holds(db: &Path, delegate: &Delegate, now_ms: i64) -> bool {
    farcooler_store::engine_cutover::startup::grant_holds(db, delegate.grant, &delegate.attempt, now_ms, crate::cutover_grant::MAX_AGE_MS).unwrap_or_else(|error| {
        tracing::warn!(%error, "the owner's cutover grant could not be checked");
        false
    })
}

/// Whether the database says now that the delegate's pane is its workspace's
/// orchestrator, on the runner's login, with no member leading the workspace.
/// One that can't be read says no.
pub(crate) fn still_the_owners(db: &Path, delegate: &Delegate) -> bool {
    farcooler_store::engine_cutover::startup::owners_orchestrator(db, delegate.terminal, delegate.workspace).unwrap_or_else(|error| {
        tracing::warn!(%error, "the cutover's orchestrator could not be checked");
        false
    })
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

/// The owner's orchestrator as the live Prepare admitted it: the pane to pin
/// for recovery, and the owner's grant to spend just before the hold.
pub(crate) struct Pinned {
    pub delegate: Delegate,
    pub grant: crate::cutover_grant::Grant,
}

/// The live daemon's Prepare admission: a person, or the owner's
/// orchestrator, with the pane to pin and the grant to spend when it is the
/// latter. Writes nothing: the Prepare spends the grant last, after the drain
/// and the recheck, and gives it back if the hold then fails (M1).
pub(crate) async fn admit(
    svc: &Arc<crate::service::Service>, peer: Peer, session: Arc<crate::sessions::Session>, caller_pid: Option<i32>,
    caller_pane: Arc<OnceLock<Uuid>>, attempt: &str,
) -> Result<(Admission, Option<Pinned>)> {
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
    // Last, once everything else admits it: the owner's word.
    let grant = crate::cutover_grant::required(&svc.store, workspace, crate::step_up_rpc::now_ms())?;
    let delegate = Delegate { attempt: attempt.into(), terminal: term.id, workspace, pane_pid, pane_start, grant: grant.answer };
    Ok((admission, Some(Pinned { delegate, grant })))
}

/// The live Prepare once admitted: pin the pane (or forget any), then drain
/// and hold. The owner's grant is spent last, after the drain and the
/// caller's recheck, just before the hold mark; a hold that fails gives it
/// back, so only a crash between the two costs the owner an ask (M1).
pub(crate) async fn pin_and_hold(
    svc: &crate::service::Service, admission: &Admission, pinned: Option<&Pinned>, attempt: &str,
) -> Result<std::result::Result<serde_json::Value, crate::cutover_admin::Refused>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = svc.root_dir();
    match pinned { Some(p) => write(root, &p.delegate), None => forget(root) }
        .map_err(|error| { tracing::warn!(%error, "cutover delegate not recorded"); DomainError::OperationFailed })?;
    let spent = AtomicBool::new(false);
    let held = crate::cutover_admin::hold(svc, attempt, || {
        admission.recheck_now().map_err(|_| crate::cutover_admin::Refused("the caller lost access during the drain".into()))?;
        if let Some(p) = pinned {
            crate::cutover_grant::spend(&svc.store, &p.grant, attempt, crate::step_up_rpc::now_ms())
                .map_err(|error| crate::cutover_admin::Refused(error.to_string()))?;
            spent.store(true, Ordering::SeqCst);
        }
        Ok(())
    }).await;
    if held.is_err() {
        let _ = forget(root);
        if let (Some(p), true) = (pinned, spent.load(Ordering::SeqCst)) {
            if let Err(error) = crate::cutover_grant::give_back(&svc.store, &p.grant, attempt) {
                tracing::warn!(%error, "the owner's cutover grant could not be given back");
            }
        }
    }
    Ok(held)
}

#[cfg(test)]
#[path = "cutover_delegate_tests.rs"]
mod tests;

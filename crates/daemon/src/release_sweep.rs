//! The released devices' sweep (ov-783): a device taken off a member's list,
//! or left when a member was removed, has no line in `authorized_keys` and no
//! open session, even when the revoke that should have done it failed.
//!
//! `member.set` revokes the devices it takes off a list after the list is
//! saved, so a revoke that fails there (the file couldn't be written) would
//! leave a key that still logs in, with nothing to retry it: the device is
//! off the list, so the same `member.set` again releases nothing new. The
//! sweep is that retry. It runs at the end of every `member.set` and
//! `member.remove`, and once when the daemon starts, before its tunnel, and
//! needs no record of its own: what it must cut off is what
//! `Store::released_to_sweep` names. Every device it revokes goes in one
//! write, with one tunnel rebuild at most.
//!
//! It never touches a device that isn't released, and it leaves alone one a
//! member's device enrolled that's waiting on its first binding (ov-719):
//! that one is released and keeps its line until a runner admin binds it.

use std::sync::Arc;
use std::time::Duration;

use farcooler_core::{DomainError, Result};

use crate::service::Service;

/// What one sweep did.
#[derive(Debug, Default)]
pub struct Swept {
    /// Devices whose line it took out, closing their sessions with it.
    pub revoked: Vec<String>,
    /// Devices with no line whose open sessions it closed.
    pub closed: Vec<String>,
    /// Devices whose line is still there, each with why.
    pub failed: Vec<(String, DomainError)>,
}

/// Revoke every released device that still has a line, all in one write
/// with one tunnel rebuild at most (`enrollment::revoke_many`), and close the
/// open sessions of every one that has none. `Err` only when the store or
/// `authorized_keys` can't be read, so nothing was tried.
pub async fn sweep(svc: &Service) -> Result<Swept> {
    sweep_with(svc, true).await
}

async fn sweep_with(svc: &Service, rebuild_tunnel: bool) -> Result<Swept> {
    let mut swept = Swept::default();
    let candidates = svc.store.released_to_sweep()?;
    if candidates.is_empty() {
        return Ok(swept);
    }
    let lines = crate::enrollment::list(svc).await?;
    let (with_line, without): (Vec<String>, Vec<String>) =
        candidates.into_iter().partition(|d| lines.items.iter().any(|e| e.client_id == *d));
    for device in without {
        // Marked first, as a revoke does: `close` marks nothing (ov-832).
        svc.sessions().mark_revoked(&device);
        if svc.sessions().close(&device) > 0 {
            swept.closed.push(device);
        }
    }
    if with_line.is_empty() {
        return Ok(swept);
    }
    // Asked again inside the fence lock: one bound to a member since the
    // list above was read is someone's, and kept.
    let store = svc.store.clone();
    let still = move |device: &str| store.device_released(device).unwrap_or(false) && store.member_of_device(device).is_ok_and(|m| m.is_none());
    match crate::enrollment::revoke_many(svc, with_line.clone(), still, rebuild_tunnel).await {
        Ok(revoked) => swept.revoked = revoked,
        Err(err) => {
            tracing::error!(?err, count = with_line.len(), "revoking released devices failed; the next sweep tries again");
            swept.failed = with_line.into_iter().map(|d| (d, err.clone())).collect();
        }
    }
    Ok(swept)
}

/// The sweep when the daemon starts, before its tunnel does (`main.rs`):
/// what a failed revoke left before a restart is taken out of the file the
/// first tunnel is built from, so that tunnel never admits a swept device's
/// node key. Nothing to rebuild, so it's one store read and one file write at
/// most. Each revocation goes to the people log as the runner's own.
///
/// Waited for [`START_LIMIT`] at most: the write takes the fence's lock,
/// which has no timeout, and something else holding it must not keep the
/// daemon from starting. Past that the sweep goes on in the background, and
/// rebuilds the tunnel once it lands, since the tunnel may have started from
/// the file before it.
pub async fn sweep_at_start(svc: &Arc<Service>) {
    sweep_at_start_within(svc, START_LIMIT).await;
}

/// How long the daemon's start waits for its sweep.
pub const START_LIMIT: Duration = Duration::from_secs(10);

/// [`sweep_at_start`], waiting `limit` at most.
pub async fn sweep_at_start_within(svc: &Arc<Service>, limit: Duration) {
    let sweeping = svc.clone();
    let mut sweep = Box::pin(async move { sweep_with(&sweeping, false).await });
    match tokio::time::timeout(limit, &mut sweep).await {
        Ok(swept) => record_start(svc, swept, false).await,
        Err(_) => {
            tracing::warn!(?limit, "the released devices' sweep is waiting on the fence's lock; starting without it");
            let svc = svc.clone();
            tokio::spawn(async move {
                let swept = sweep.await;
                record_start(&svc, swept, true).await;
            });
        }
    }
}

/// Log what a start sweep did, and rebuild the tunnel when it revoked
/// something after the tunnel may have started (`late`).
async fn record_start(svc: &Service, swept: Result<Swept>, late: bool) {
    let swept = match swept {
        Ok(swept) => swept,
        Err(err) => {
            tracing::error!(?err, "the released devices' sweep couldn't read what to sweep");
            return;
        }
    };
    for device in &swept.revoked {
        tracing::warn!(device = %device, "a released device still had a line; revoked at start");
        record(svc, RUNNER, device, "revoked");
    }
    for (device, _) in &swept.failed {
        record(svc, RUNNER, device, "revoke_failed");
    }
    if late && !swept.revoked.is_empty() {
        let outcome = crate::allowlist::start_tunnel(svc).await;
        tracing::info!(?outcome, "made this runner's tunnel match the file a late start sweep wrote");
    }
}

/// The people log's actor for a change the runner made itself, with no
/// caller: the sweep at start.
pub const RUNNER: &str = "runner";

/// One line in the people log about `device`, whose member isn't known here.
/// A line that can't be written is logged and isn't the sweep failing.
fn record(svc: &Service, actor: &str, device: &str, kind: &str) {
    let detail = serde_json::json!({ "device": device });
    let event = farcooler_store::member_roles::NewPeopleEvent { actor, handle: "", workspace: None, kind, detail: &detail };
    if let Err(e) = svc.store.record_people_event(event) {
        tracing::warn!(kind, error = %e.what(), "a sweep's revocation wasn't logged");
    }
}

#[cfg(test)]
#[path = "release_sweep_tests.rs"]
mod tests;

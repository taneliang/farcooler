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
pub async fn sweep_at_start(svc: &Service) {
    let swept = match sweep_with(svc, false).await {
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

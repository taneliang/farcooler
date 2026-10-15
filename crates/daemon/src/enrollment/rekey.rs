//! `client.rekey` (ov-569): a device moves its own line to a new key it holds.
//!
//! This is the runner half of a phone's "Make a New Key". The phone made a new
//! Key A, and it reaches this runner with the old one. It asks here for its
//! line to carry the new key instead. Nothing else about the line changes:
//! scope, label, node key, the restricted shape and the client id all come
//! back as they were.
//!
//! **The client id stays** (fix round 1, H1). This runner knows who a device
//! is by that id: a member's (`member_devices`), a removed member's
//! (`released_devices`), or, for an id in neither, the owner's
//! (`people::person_of`). A new id would make a member's phone, or a removed
//! member's, the owner. So the request's id is ignored, and a released device
//! is refused outright: it's nobody's, and gets no new key here. A later
//! ceremony with the new key finds it by fingerprint ("already enrolled").
//!
//! ## Who may be lying, and what this doesn't depend on
//!
//! - **The request.** The line rewritten is the one the CONNECTION
//!   authenticated as (`peer.client_id`), never one the request names, the
//!   same rule `set_node_key` follows. So a device can't touch another
//!   device's line, widen its own scope, or give itself a shell.
//! - **Whoever holds the old key.** They could already use the line, and moving
//!   it to a key they hold grants nothing more. The new key must sign
//!   `fence::rekey_statement` over the old line's fingerprint, as this runner
//!   reads it from the file, and the new key's. That proves the caller holds
//!   the new key, and it stops a proof made for one line from moving another.
//! - **The phone's own bookkeeping.** A reply lost on the way back leaves the
//!   phone unsure whether the line moved. It then tries the new key, and the
//!   file is the answer.
//!
//! A device that also holds a plain (shell) line under its id is a Mac, and is
//! refused: its Key B would be left naming an id no Key A line carries.
//!
//! **Sessions stay open.** The caller is using one to ask, and a re-key isn't a
//! revocation. A device whose key was stolen is removed with `client.revoke`,
//! which closes sessions.

use farcooler_core::{DomainError, Result};
use farcooler_fence::{self as fence, Entry};
use farcooler_protocol::v1::{ClientRekey, ClientRekeyResult, Scope};
use farcooler_transport::Peer;

use super::{attributed, blocking, now_millis, read_back, refused, Refusal};
use crate::service::Service;
use crate::wire;

/// Move the caller's own Key A line to the key in `request`.
pub async fn rekey(svc: &Service, peer: &Peer, request: &ClientRekey) -> Result<ClientRekeyResult> {
    // A local socket client has no line here, and picking one for it is the
    // thing this call must never do.
    let Some(old_id) = peer.client_id.clone() else {
        return Err(DomainError::InvalidArgument { what: "client_id" });
    };
    let new_key = ssh_key::PublicKey::from_openssh(request.public_key.trim())
        .map_err(|_| DomainError::InvalidArgument { what: "public_key" })?;
    if new_key.algorithm() != ssh_key::Algorithm::Ed25519 {
        return Err(DomainError::InvalidArgument { what: "public_key algorithm" });
    }
    let new_fingerprint = new_key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
    // Parsed here, before the lock. Its error names no input.
    let proof = ssh_key::SshSig::from_pem(request.proof.trim())
        .map_err(|_| DomainError::InvalidArgument { what: "proof" })?;
    // Nobody's device gets no new key. Its id stays released either way, but
    // a removed member's phone has no business on this runner at all.
    if svc.store.device_released(&old_id)? {
        return Err(DomainError::InvalidArgument { what: "client_id" });
    }
    let store = svc.store.clone();
    let request = request.clone();
    let path = svc.authorized_keys().to_path_buf();
    let now = now_millis();

    let line = blocking(move || {
        // Under one lock hold, as every write here: the block is rebuilt from
        // what was read, so a snapshot from before a concurrent enrollment
        // would put the file back without that device.
        fence::update(&path, fence::AUTHORIZED_KEYS, fence::Placement::Last, |entries| {
            let entries = attributed(entries);
            // Whose a device is, for the case below where another line takes
            // this one's place: the same person's only. An unreadable store
            // answers no one's, which refuses.
            let whose = |id: &str| -> Option<Option<String>> {
                if store.device_released(id).ok()? { return None; }
                store.member_of_device(id).ok()
            };
            rewrite(&entries, &old_id, &new_key, &new_fingerprint, &proof, &request, &whose)
        })
    })
    .await?;
    tracing::info!(client = %peer.client_id.as_deref().unwrap_or("-"), "moved a device to a new key");
    Ok(ClientRekeyResult { client: Some(wire::enrolled_client(&line, now)) })
}

/// The block as it reads once the caller's line carries the new key, and that
/// line as the parser reads it back.
///
/// Pure, so the rules are tested without a file: every refusal leaves the
/// block exactly as it was.
pub(super) fn rewrite(
    entries: &[Entry],
    old_id: &str,
    new_key: &ssh_key::PublicKey,
    new_fingerprint: &str,
    proof: &ssh_key::SshSig,
    request: &ClientRekey,
    // Whose device an id is: `Some(None)` the owner's, `Some(Some(handle))` a
    // member's, `None` released or unknowable.
    whose: &dyn Fn(&str) -> Option<Option<String>>,
) -> std::result::Result<(fence::Change, Entry), Refusal> {
    let mine = |e: &&Entry| !e.client_id.is_empty() && e.client_id == old_id;
    let old = entries
        .iter()
        .find(|e| mine(e) && !e.shell_access)
        .ok_or(Refusal(DomainError::NotFound))?;
    if entries.iter().filter(mine).any(|e| e.shell_access) {
        return Err(Refusal(DomainError::InvalidArgument { what: "client_id" }));
    }
    // A scope this build can't read would render as host_admin, so a rewrite
    // would promote it. Leave such a line alone, as `with_node_key` does.
    if matches!(old.scope, Scope::Unspecified) {
        return Err(Refusal(DomainError::InvalidArgument { what: "scope" }));
    }
    let statement = fence::rekey_statement(&old.fingerprint, new_fingerprint)
        .ok_or(Refusal(DomainError::InvalidArgument { what: "public_key" }))?;
    if new_key.verify(fence::REKEY_NAMESPACE, statement.as_bytes(), proof).is_err() {
        return Err(Refusal(DomainError::InvalidArgument { what: "proof" }));
    }

    // The new key can already log in here: someone enrolled it while the
    // phone was making the switch, by scanning its new code. Then the only
    // thing left is to take the old line off, which is the caller giving up
    // its own access and needs nobody's leave. The new key's line stays as
    // whoever wrote it chose.
    //
    // Only when that line is the same person's device as this one (H1): taking
    // a member's line off in favor of one bound to nobody would make their
    // phone the owner.
    if let Some(existing) = entries.iter().find(|e| e.fingerprint == new_fingerprint) {
        let mine = whose(old_id);
        if mine.is_none() || whose(&existing.client_id) != mine {
            return Err(Refusal(DomainError::InvalidArgument { what: "client_id" }));
        }
        let existing = existing.clone();
        return Ok((rebuilt(entries, old_id, None), existing));
    }

    // Rendered from scratch through the one function that writes Key A lines,
    // so nothing is spliced. The name is the one the old line's comment
    // carries, unwrapped, so Settings shows the same device.
    let name = fence::device_name(&old.label).unwrap_or("device");
    let node = (!old.node_key.is_empty()).then_some(old.node_key.as_str());
    // The caller's own id, never the request's (H1).
    let line = fence::render(&request.public_key, name, old_id, old.scope, fence::Grant::FarCooler, node)
        .map_err(|r| Refusal(refused(r)))?;
    let entry = read_back(&line).ok_or(Refusal(DomainError::OperationFailed))?;
    Ok((rebuilt(entries, old_id, Some(line)), Entry { account: old.account.clone(), ..entry }))
}

/// The block with the caller's Key A line replaced by `line`, or dropped when
/// there is none. Every other line goes back byte for byte, foreign ones
/// included.
fn rebuilt(entries: &[Entry], old_id: &str, line: Option<String>) -> fence::Change {
    let mut ours = Vec::new();
    let mut foreign = Vec::new();
    let mut line = line;
    for entry in entries {
        if entry.client_id.is_empty() {
            foreign.push(entry.line.clone());
        } else if entry.client_id == old_id && !entry.shell_access {
            ours.extend(line.take());
        } else {
            ours.push(entry.line.clone());
        }
    }
    fence::Change::Write { entries: ours, foreign }
}

#[cfg(test)]
#[path = "rekey_tests.rs"]
mod tests;

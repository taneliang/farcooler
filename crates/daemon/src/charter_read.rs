//! `workspace.charter` (ov-755): a workspace's charter text, for anyone on
//! the runner. The charter is the lead's to write and everyone's to read, so
//! a member knows the rules the team's work is done by. Who may ask is the
//! gate's (`Ruling::Reads`: the owner, a member, a member's rep; not a
//! device whose member was removed); what is read here is the file, whatever
//! the asker's scope, since its text carries no path.

use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, result};

use crate::service::Service;
use crate::wire;
use crate::workspace_home;

/// The most of a charter that is sent. A charter is a page or two; a file a
/// hundred times that is not one, and is cut rather than put on the wire.
pub(crate) const MAX_BYTES: u64 = 256 * 1024;

/// `workspace.charter`, targeting a workspace: its charter's text, or none
/// when it has none yet, with the hash `workspace.set_charter` is given back
/// (ov-757).
pub fn handle(svc: &Service, req: &pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(wire::parse_id).ok_or(DomainError::NotFound)?;
    svc.store.get_workspace(id)?;
    Ok(result::Value::WorkspaceCharter(answer(current(svc, id)?)))
}

/// The charter as `bytes` say, or none.
pub(crate) fn answer(bytes: Option<Vec<u8>>) -> pb::WorkspaceCharter {
    match bytes {
        None => pb::WorkspaceCharter { text: String::new(), exists: false, sha256: String::new() },
        Some(bytes) => {
            let sha256 = sha256_hex(&bytes);
            pb::WorkspaceCharter { text: String::from_utf8_lossy(&bytes).into_owned(), exists: true, sha256 }
        }
    }
}

/// The SHA-256 of `bytes`, lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Workspace `id`'s charter's bytes, up to `MAX_BYTES`, or `None` when it
/// has none: no file, or a link or anything else that isn't a regular file.
pub(crate) fn current(svc: &Service, id: uuid::Uuid) -> Result<Option<Vec<u8>>> {
    let path = workspace_home::charter_path(svc.root_dir(), id);
    // The orchestrator's pane writes this file, so it may be a link to any
    // file the runner can read, or a pipe that never answers: neither is a
    // charter. Opened without following or waiting, and only a regular file
    // is read (as `survey_read::read_named` does).
    let file = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(libc::ELOOP) => return Ok(None),
        Err(e) => {
            tracing::warn!(workspace = %id, error = %e, "could not open the charter");
            return Err(DomainError::OperationFailed);
        }
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not read the charter");
        DomainError::OperationFailed
    })?;
    Ok(Some(bytes))
}

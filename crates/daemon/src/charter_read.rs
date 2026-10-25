//! `workspace.charter` (ov-755): a workspace's charter text, for anyone on
//! the runner. The charter is the lead's to write and everyone's to read, so
//! a member knows the rules the team's work is done by. Who may ask is the
//! gate's (`Ruling::Members`: the owner, a member, a member's rep; not a
//! device whose member was removed); what is read here is the file, whatever
//! the asker's scope, since its text carries no path.

use std::io::Read as _;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, result};

use crate::service::Service;
use crate::wire;
use crate::workspace_home;

/// The most of a charter that is sent. A charter is a page or two; a file a
/// hundred times that is not one, and is cut rather than put on the wire.
const MAX_BYTES: u64 = 256 * 1024;

/// `workspace.charter`, targeting a workspace: its charter's text, or none
/// when it has none yet.
pub fn handle(svc: &Service, req: &pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(wire::parse_id).ok_or(DomainError::NotFound)?;
    svc.store.get_workspace(id)?;
    let path = workspace_home::charter_path(svc.root_dir(), id);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(result::Value::WorkspaceCharter(pb::WorkspaceCharter { text: String::new(), exists: false }));
        }
        Err(e) => {
            tracing::warn!(workspace = %id, error = %e, "could not open the charter");
            return Err(DomainError::OperationFailed);
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut bytes).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not read the charter");
        DomainError::OperationFailed
    })?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(result::Value::WorkspaceCharter(pb::WorkspaceCharter { text, exists: true }))
}

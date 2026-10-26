//! `workspace.set_charter` (ov-757, slice 1 of `reports/ov-753/design.md`):
//! a workspace's charter replaced, whole, through the runner.
//!
//! **Who** is the gate's (`Ruling::Charter`, `people::may_edit_charter`):
//! the owner, the workspace's lead, or a Maintainer. A member's rep never
//! edits it (`rep_may`); it asks the orchestrator, which edits only on the
//! word of one of those three (the manager skill).
//!
//! **Never over an edit made meanwhile.** The caller sends back the hash
//! `workspace.charter` gave it (`expected_sha256`, empty for no charter);
//! one that no longer matches what's on disk is refused `charter_changed`,
//! and nothing is written. Edits through the runner are taken one at a time
//! (`EDITS`), so two of them can't both match. A pane writing the file
//! directly between the check and the write isn't stopped: the runner's
//! panes are its Unix user, and this is a guardrail, as the charter file
//! always was.
//!
//! **Each edit goes to the people log** (`member.events`), with who made it
//! and the new text's hash.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::Peer;

use crate::charter_read::{self, MAX_BYTES};
use crate::people::{Person, person_of};
use crate::service::Service;
use crate::workspace_home;

/// One edit through the runner at a time, so a hash checked is the hash
/// written over.
static EDITS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// `workspace.set_charter`, targeting a workspace.
pub async fn handle(svc: &Service, peer: &Peer, req: pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(crate::wire::parse_id).ok_or(DomainError::NotFound)?;
    let Some(request::Payload::WorkspaceSetCharter(p)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    if p.text.len() as u64 > MAX_BYTES {
        return Err(DomainError::InvalidArgument { what: "charter_too_long" });
    }
    if p.text.trim().is_empty() {
        // An empty charter reads as one someone chose to leave blank
        // (`workspace_home`): the orchestrator would stop asking for one.
        return Err(DomainError::InvalidArgument { what: "charter_empty" });
    }
    let workspace = svc.store.get_workspace(id)?;
    let _one = EDITS.lock().await;
    // Main adopts a repository's charter here first, and a new workspace
    // takes Main's, so the hash is checked against what the orchestrator
    // would read.
    svc.ensure_workspace_home(&workspace).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not make the workspace's home");
        DomainError::OperationFailed
    })?;
    let path = workspace_home::charter_path(svc.root_dir(), id);
    // A link, a pipe or a folder at the charter's name is not written
    // through or over.
    if path.symlink_metadata().is_ok_and(|m| !m.is_file()) {
        return Err(DomainError::Conflict { what: "charter_not_a_file" });
    }
    let now = charter_read::current(svc, id)?.map(|bytes| charter_read::sha256_hex(&bytes)).unwrap_or_default();
    if now != p.expected_sha256.trim().to_ascii_lowercase() {
        return Err(DomainError::Conflict { what: "charter_changed" });
    }
    crate::skill_install::write_atomically(&path, &p.text).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not write the charter");
        DomainError::OperationFailed
    })?;
    let written = charter_read::answer(Some(p.text.into_bytes()));
    let actor = match person_of(&svc.store, peer)? {
        Person::Member(handle) => format!("member:{handle}"),
        _ => "user".to_string(),
    };
    let detail = serde_json::json!({ "sha256": written.sha256 });
    let event = farcooler_store::member_roles::NewPeopleEvent { actor: &actor, handle: "", workspace: Some(id), kind: "charter", detail: &detail };
    if let Err(e) = svc.store.record_people_event(event) {
        // Written already: the log missing a line isn't the edit failing.
        tracing::warn!(workspace = %id, error = %e.what(), "a charter edit wasn't logged");
    }
    tracing::info!(workspace = %id, actor = %actor, "the charter was edited");
    Ok(result::Value::WorkspaceCharter(written))
}

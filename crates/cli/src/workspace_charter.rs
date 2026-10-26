//! `workspace charter <workspace>` (ov-755): a workspace's charter text, for
//! the owner, a member or a member's rep. The Mac's Show Charter sheet reads
//! it with `--json` when the runner is on another machine.
//!
//! `workspace set-charter` (ov-757) replaces it, for the owner, the
//! workspace's lead or a Maintainer, given back the hash it was read at.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::json;
use uuid::Uuid;

use crate::tasks::{DispatchLink, Refused};
use crate::{Fallible, req_for, uuid_of};

const NEEDS_UPDATE: &str = "This runner needs an update to show a charter.";

/// `workspace.charter` for `workspace`, naming the capability so a runner
/// from before refuses rather than failing on an unknown method.
pub(crate) fn request(workspace: Uuid) -> pb::Request {
    let mut r = req_for("workspace.charter", workspace);
    r.required_capabilities.push(capability::WORKSPACE_CHARTER.to_string());
    r
}

/// The charter as the runner answers it: `exists` false for a workspace
/// with no charter yet, and `sha256` empty from a runner before ov-757.
pub(crate) async fn read<L: DispatchLink>(link: &mut L, workspace: Uuid) -> Result<pb::WorkspaceCharter, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::WORKSPACE_CHARTER) {
        return Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let answer = link.call(request(workspace)).await.map_err(|e| crate::tasks::refusal(e, "the charter could not be read"))?;
    let Some(result::Value::WorkspaceCharter(c)) = answer.value else { return Err(crate::daemon_link::UNREADABLE.into()) };
    Ok(c)
}

/// `workspace charter`, once the workspace is named. `--json` carries the
/// `sha256` that `set-charter --expect` takes.
pub(crate) async fn show<L: DispatchLink>(link: &mut L, ws: &pb::Workspace, json: bool) -> Fallible {
    let c = read(link, uuid_of(&ws.id)).await?;
    if json {
        println!("{}", json!({ "workspace": uuid_of(&ws.id).to_string(), "exists": c.exists, "text": c.text, "sha256": c.sha256 }));
    } else if c.exists {
        print!("{}", c.text);
    } else {
        println!("{} has no charter yet", ws.name);
    }
    Ok(())
}

const NO_EDIT: &str = "this runner's Far Cooler can't edit a charter yet. update it and try again";

/// `workspace.set_charter` for `workspace`.
pub(crate) fn set_request(workspace: Uuid, text: String, expected: &str) -> pb::Request {
    let mut r = req_for("workspace.set_charter", workspace);
    r.payload = Some(request::Payload::WorkspaceSetCharter(pb::WorkspaceSetCharter { text, expected_sha256: expected.trim().to_string() }));
    r.required_capabilities.push(capability::MEMBER_ROLES.to_string());
    r
}

/// This CLI's line for a refused charter edit.
fn set_said(what: &str) -> Option<&'static str> {
    Some(match what {
        "not_yours" => "only the runner's owner, this workspace's lead or a Maintainer can edit its charter",
        "rep_messenger" => "a rep doesn't edit the charter. send the change to the orchestrator, which asks the lead",
        "charter_changed" => "the charter changed since that hash was read. read it again with `workspace charter --json` and redo the edit",
        "charter_not_a_file" => "the charter's name on the runner isn't a plain file, so it wasn't written over",
        "charter_too_long" => "a charter is at most 256 KB",
        "charter_empty" => "a charter needs something written in it",
        _ => return None,
    })
}

/// `workspace set-charter`, once the workspace is named and the text read.
pub(crate) async fn set<L: DispatchLink>(link: &mut L, ws: &pb::Workspace, text: String, expected: &str, json: bool) -> Fallible {
    if !link.capabilities().iter().any(|c| c == capability::MEMBER_ROLES) {
        return Err(Box::new(Refused::new(NO_EDIT.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let answer = link.call(set_request(uuid_of(&ws.id), text, expected)).await.map_err(|e| match &e {
        farcooler_transport::ClientError::Daemon { code, what, .. } if set_said(what).is_some() => {
            Refused::naming(set_said(what).unwrap_or_default().to_string(), *code, what.clone())
        }
        _ => crate::tasks::refusal(e, "the charter could not be written"),
    })?;
    let Some(result::Value::WorkspaceCharter(c)) = answer.value else { return Err(crate::daemon_link::UNREADABLE.into()) };
    if json {
        println!("{}", json!({ "workspace": uuid_of(&ws.id).to_string(), "sha256": c.sha256 }));
    } else {
        println!("{}'s charter is saved", ws.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request names its method, its workspace and the capability, so a
    /// runner from before refuses it rather than failing on the method.
    #[test]
    fn the_request_names_the_capability() {
        let r = request(Uuid::from_u128(7));
        assert_eq!(r.method, "workspace.charter");
        assert_eq!(r.required_capabilities, [capability::WORKSPACE_CHARTER]);
        assert!(farcooler_daemon::people::rep_may(&r), "a rep's pane may ask for it");
    }

    /// The edit carries its text, the hash it was made to, and the
    /// capability; a rep's pane may not send it.
    #[test]
    fn the_edit_names_the_capability_and_its_hash() {
        let r = set_request(Uuid::from_u128(7), "## Who decides\n".into(), " ab12 ");
        assert_eq!(r.method, "workspace.set_charter");
        assert_eq!(r.required_capabilities, [capability::MEMBER_ROLES]);
        let Some(request::Payload::WorkspaceSetCharter(p)) = &r.payload else { panic!("no payload") };
        assert_eq!((p.text.as_str(), p.expected_sha256.as_str()), ("## Who decides\n", "ab12"));
        assert!(!farcooler_daemon::people::rep_may(&r), "a rep's pane asks the orchestrator instead");
    }
}

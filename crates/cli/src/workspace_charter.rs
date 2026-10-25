//! `workspace charter <workspace>` (ov-755): a workspace's charter text, for
//! the owner, a member or a member's rep. The Mac's Show Charter sheet reads
//! it with `--json` when the runner is on another machine.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
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

/// The charter's text, or `None` for a workspace with no charter yet.
pub(crate) async fn read<L: DispatchLink>(link: &mut L, workspace: Uuid) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::WORKSPACE_CHARTER) {
        return Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let answer = link.call(request(workspace)).await.map_err(|e| crate::tasks::refusal(e, "the charter could not be read"))?;
    let Some(result::Value::WorkspaceCharter(c)) = answer.value else { return Err(crate::daemon_link::UNREADABLE.into()) };
    Ok(c.exists.then_some(c.text))
}

/// `workspace charter`, once the workspace is named.
pub(crate) async fn show<L: DispatchLink>(link: &mut L, ws: &pb::Workspace, json: bool) -> Fallible {
    let text = read(link, uuid_of(&ws.id)).await?;
    if json {
        println!("{}", json!({ "workspace": uuid_of(&ws.id).to_string(), "exists": text.is_some(), "text": text.unwrap_or_default() }));
    } else {
        match text {
            Some(text) => print!("{text}"),
            None => println!("{} has no charter yet", ws.name),
        }
    }
    Ok(())
}

//! `farcooler member connectors` (ov-586): which of a member's claude.ai
//! connectors their rep may use. The rules are the runner's
//! (`farcooler_daemon::rpc_people`, `rep_gate`); this only asks and says.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{expect_value, req, tasks, with};

const NO_CONNECTORS: &str =
    "this runner's Far Cooler can't give a rep connectors yet, so its reps use none. update it and try again";

/// `member connectors <handle>` on `link`: read the list with `set` absent,
/// or replace it with `set` (empty turns them all off), answering what to
/// print.
pub(crate) async fn connectors_on<L: tasks::DispatchLink>(
    link: &mut L,
    handle: &str,
    set: Option<&[String]>,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::REP_CONNECTORS) {
        return Err(NO_CONNECTORS.into());
    }
    let handle = handle.trim();
    let payload = pb::MemberConnectors {
        handle: handle.to_string(),
        servers: set.unwrap_or_default().iter().map(|s| s.trim().to_string()).collect(),
        set: set.is_some(),
    };
    let mut r = with(req("member.connectors"), request::Payload::MemberConnectors(payload));
    r.required_capabilities.push(capability::REP_CONNECTORS.to_string());
    let answer = link.call(r).await.map_err(crate::people::refused)?;
    let result::Value::MemberConnectorList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        return Ok(serde_json::json!({ "handle": list.handle, "connectors": list.servers }).to_string());
    }
    Ok(said(&list, set.is_some()))
}

/// The list as people read it, and when a change reaches a running rep.
pub(crate) fn said(list: &pb::MemberConnectorList, changed: bool) -> String {
    let who = &list.handle;
    if list.servers.is_empty() {
        let tail = if changed { ". a running rep is refused them from its next call" } else { "" };
        return format!("{who}'s rep uses no claude.ai connector{tail}");
    }
    let names = list.servers.join(", ");
    let tail = if changed {
        ". one turned off is refused from its next call; one turned on reaches a running rep when it next starts"
    } else {
        ""
    };
    let said = format!("{who}'s rep may use these claude.ai connectors: {names}{tail}");
    if !changed {
        return said;
    }
    // The runner can't see the member's claude.ai, so it can't say whether
    // "slack" is their "Slack" (review L2): say what each name will match.
    let keys: Vec<String> = list
        .servers
        .iter()
        .map(|s| format!("{s} matches mcp__{}__*", farcooler_store::member_connectors::connector_key(s)))
        .collect();
    format!(
        "{said}\nnames match exactly, capitals included, so type each as claude.ai shows it. {}",
        keys.join(", ")
    )
}

#[cfg(test)]
#[path = "member_connectors_tests.rs"]
mod tests;

//! `farcooler member role` and `farcooler member log` (ov-757): a member's
//! role on the runner, which only its owner changes, and the people log of
//! who changed what.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::tasks::{self, DispatchLink};
use crate::{expect_value, req, with};

const NO_ROLES: &str = "this runner's Far Cooler doesn't know members' roles yet. update it and try again";

/// The request `member role` sends.
pub(crate) fn role_request(handle: &str, role: &str) -> pb::Request {
    let mut r = with(
        req("member.set_role"),
        request::Payload::MemberSetRole(pb::MemberSetRole { handle: handle.trim().to_string(), role: role.to_string() }),
    );
    r.required_capabilities.push(capability::MEMBER_ROLES.to_string());
    r
}

/// What `member role` did, as people read it.
pub(crate) fn role_said(handle: &str, role: &str) -> String {
    match role {
        "maintainer" => format!("{handle} is a Maintainer: they can edit every workspace's charter, rename a workspace, change its prefix, and set its PR size, cost line, nudge hours and wake on answer"),
        _ => format!("{handle} is a member: they edit a charter only where they lead"),
    }
}

/// `member role <handle> <role>` on `link`, answering what to print.
pub(crate) async fn role_on<L: DispatchLink>(link: &mut L, handle: &str, role: &str, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::MEMBER_ROLES) {
        return Err(NO_ROLES.into());
    }
    let answer = link.call(role_request(handle, role)).await.map_err(refused)?;
    let result::Value::MemberList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        return Ok(farcooler_client::members_json::members_json(&list).to_string());
    }
    Ok(crate::people::list_lines(&list, Some(&role_said(handle.trim(), role))))
}

/// `member log` on `link`: the people log, newest first.
pub(crate) async fn log_on<L: DispatchLink>(link: &mut L, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::MEMBER_ROLES) {
        return Err(NO_ROLES.into());
    }
    let mut r = req("member.events");
    r.required_capabilities.push(capability::MEMBER_ROLES.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::PeopleEventList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        let events: Vec<serde_json::Value> = list
            .events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "at_ms": e.at_ms, "actor": e.actor, "handle": e.handle, "kind": e.kind,
                    "workspace": uuid::Uuid::from_slice(&e.workspace_id).map(|u| u.to_string()).unwrap_or_default(),
                    "detail": serde_json::from_str::<serde_json::Value>(&e.detail).unwrap_or_default(),
                })
            })
            .collect();
        return Ok(serde_json::json!({ "events": events }).to_string());
    }
    if list.events.is_empty() {
        return Ok("nobody's role, devices or charter has changed through this runner yet".to_string());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    Ok(list.events.iter().map(|e| event_line(e, now)).collect::<Vec<_>>().join("\n"))
}

/// "just now", "5m ago", "3h ago", "2d ago".
fn ago(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes}m ago"),
        60..=1439 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

/// One line of the log, as people read it, at `now`.
pub(crate) fn event_line(e: &pb::PeopleEvent, now: i64) -> String {
    let when = format!("{:>9}", ago(now - e.at_ms));
    let who = match e.actor.strip_prefix("member:") {
        Some(handle) => handle.to_string(),
        None => "the owner".to_string(),
    };
    let detail: serde_json::Value = serde_json::from_str(&e.detail).unwrap_or_default();
    let what = match e.kind.as_str() {
        "role" => format!("made {} a {}", e.handle, detail["to"].as_str().unwrap_or("member")),
        "added" => format!("added {}", e.handle),
        "devices" => format!("changed {}'s devices", e.handle),
        "removed" => format!("removed {}", e.handle),
        "charter" => {
            let short: String = detail["sha256"].as_str().unwrap_or_default().chars().take(8).collect();
            format!("edited a charter ({short})")
        }
        "rename" => format!("renamed {} to {}", detail["from"].as_str().unwrap_or("a workspace"), detail["to"].as_str().unwrap_or("?")),
        "prefix" => format!("changed a workspace's prefix from {} to {}", detail["from"].as_str().unwrap_or("?"), detail["to"].as_str().unwrap_or("?")),
        "lead" => match e.handle.as_str() {
            "" => "took back a workspace's lead".to_string(),
            lead => format!("made {lead} a workspace's lead"),
        },
        other => format!("{other} {}", e.handle),
    };
    format!("{when}  {who} {what}")
}

/// A refused role change or log read, in this CLI's words.
fn refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e {
        let said = match (farcooler_core::error::word_for(*code), what.as_str()) {
            (_, "not_yours") | ("scope-denied", _) => Some("only the runner's owner can change a member's role"),
            (_, "role") => Some("a role is member or maintainer"),
            _ => None,
        };
        if let Some(said) = said {
            return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
        }
    }
    crate::people::refused(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request names its method and the capability, so a runner from
    /// before refuses it rather than failing on the method.
    #[test]
    fn the_role_request_names_the_capability() {
        let r = role_request(" bob ", "maintainer");
        assert_eq!(r.method, "member.set_role");
        assert_eq!(r.required_capabilities, [capability::MEMBER_ROLES]);
        let Some(request::Payload::MemberSetRole(p)) = r.payload else { panic!("no payload") };
        assert_eq!((p.handle.as_str(), p.role.as_str()), ("bob", "maintainer"));
    }

    /// Each kind of change reads as a sentence naming who made it.
    #[test]
    fn the_log_reads_as_sentences() {
        let e = |actor: &str, handle: &str, kind: &str, detail: &str| pb::PeopleEvent {
            at_ms: 0,
            actor: actor.into(),
            handle: handle.into(),
            kind: kind.into(),
            detail: detail.into(),
            ..Default::default()
        };
        let line = |ev| event_line(&ev, 0).split_once("  ").map(|(_, s)| s.to_string()).unwrap();
        assert_eq!(line(e("user", "bob", "role", r#"{"from":"member","to":"maintainer"}"#)), "the owner made bob a maintainer");
        assert_eq!(line(e("member:ann", "bob", "added", "{}")), "ann added bob");
        assert_eq!(line(e("user", "", "charter", r#"{"sha256":"abcdef0123"}"#)), "the owner edited a charter (abcdef01)");
        assert_eq!(event_line(&e("user", "bob", "removed", "{}"), 3 * 3_600_000), "   3h ago  the owner removed bob");
        assert_eq!(line(e("member:carol", "", "rename", r#"{"from":"Main","to":"Platform"}"#)), "carol renamed Main to Platform");
        assert_eq!(line(e("member:carol", "", "prefix", r#"{"from":"OV","to":"PL"}"#)), "carol changed a workspace's prefix from OV to PL");
        assert_eq!(line(e("user", "bob", "lead", r#"{"from":"owner","to":"bob"}"#)), "the owner made bob a workspace's lead");
        assert_eq!(line(e("user", "", "lead", r#"{"from":"bob","to":"owner"}"#)), "the owner took back a workspace's lead");
    }
}

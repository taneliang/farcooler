//! `farcooler people` (ov-856, slice 1): who has what access on this runner,
//! and which devices are held below it. Read only. The runner doesn't act on
//! any of it yet; this is what its store has recorded, for the owner to
//! check before access starts following the person.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
use serde_json::{Value, json};

use crate::tasks::DispatchLink;
use crate::{Fallible, connect_to, expect_value, req};

const NO_ACCESS: &str = "this runner hasn't recorded its people's access yet. update it, or ask its owner";

/// `farcooler people`, on a connected runner.
pub(crate) async fn people(runner: Option<&str>, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    println!("{}", people_on(&mut link, json).await?);
    Ok(())
}

/// `farcooler people` on `link`, answering what to print.
pub(crate) async fn people_on<L: DispatchLink>(link: &mut L, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::REPRESENTATIVES) {
        return Err(NO_ACCESS.into());
    }
    let answer = link.call(req("member.list")).await.map_err(crate::people::refused)?;
    let result::Value::MemberList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(if json { access_json(&list).to_string() } else { access_lines(&list) })
}

/// The answer as JSON: each person with their access, then every device.
pub(crate) fn access_json(list: &pb::MemberList) -> Value {
    let devices: Vec<Value> = list
        .device_access
        .iter()
        .map(|d| {
            json!({
                "client_id": d.client_id,
                "person": (!d.person.is_empty()).then_some(&d.person),
                "access": d.access,
                "cap": (!d.cap.is_empty()).then_some(&d.cap),
                "check": d.check,
            })
        })
        .collect();
    let people: Vec<Value> = std::iter::once(json!({ "handle": null, "name": "owner", "access": "owner" }))
        .chain(list.members.iter().map(|m| json!({ "handle": m.handle, "name": m.name, "access": m.access })))
        .collect();
    json!({ "people": people, "devices": devices })
}

/// The answer as people read it: one line per device, then what needs a look.
pub(crate) fn access_lines(list: &pb::MemberList) -> String {
    if list.device_access.is_empty() {
        return NO_ACCESS.to_string();
    }
    let mut lines = Vec::new();
    for d in &list.device_access {
        let who = if d.person.is_empty() { "owner" } else { d.person.as_str() };
        let held = if d.cap.is_empty() { String::new() } else { format!("  held at {}", d.cap) };
        let check = if d.check { "  check: added by a member's device and bound to no one" } else { "" };
        lines.push(format!("{who:16} {:24} {:7}{held}{check}", crate::truncate(&d.client_id, 24), d.access));
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "people_access_tests.rs"]
mod tests;

//! `farcooler people` (ov-856): who has what access on this runner, and which
//! devices are held below it, and (slice 4) the three changes the runner's
//! own socket makes: `people access` (a person's access), `people limit` (a
//! device's cap) and `people approve` (a device's step-up key, by the six
//! digits it shows). The Mac's People screen runs them; they are refused from
//! a device, and from a pane.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, result};
use serde_json::{Value, json};

use crate::tasks::DispatchLink;
use crate::{Fallible, connect_to, expect_value, req, with};

/// What `farcooler people` can do besides list.
#[derive(clap::Subcommand)]
pub enum PeopleCmd {
    /// Give a person Admin, Member or Viewer access.
    Access {
        handle: String,
        #[arg(value_parser = ["admin", "member", "viewer"])]
        access: String,
    },
    /// Hold one device below its person's access, at Viewer or Member, or
    /// `none` to lift the limit.
    Limit {
        client_id: String,
        #[arg(value_parser = ["viewer", "member", "none"])]
        cap: String,
    },
    /// Trust the step-up key a device proposed, given the six digits it shows.
    Approve { client_id: String, code: String },
}

const NO_ACCESS: &str = "this runner hasn't recorded its people's access yet. update it, or ask its owner";

/// `farcooler people [access|limit|approve]`, on a connected runner.
pub(crate) async fn people(runner: Option<&str>, command: Option<PeopleCmd>, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    let said = match command {
        None => people_on(&mut link, json).await?,
        Some(change) => change_on(&mut link, change, json).await?,
    };
    println!("{said}");
    Ok(())
}

/// The request `people access`, `people limit` or `people approve` sends.
pub(crate) fn change_request(change: &PeopleCmd) -> pb::Request {
    use pb::request::Payload;
    let (method, payload, needs) = match change {
        PeopleCmd::Access { handle, access } => (
            "member.set_access",
            Payload::MemberSetAccess(pb::MemberSetAccess { handle: handle.trim().to_string(), access: access.clone() }),
            capability::PERSON_ACCESS,
        ),
        PeopleCmd::Limit { client_id, cap } => (
            "client.set_cap",
            Payload::ClientSetCap(pb::ClientSetCap { client_id: client_id.trim().to_string(), cap: if cap == "none" { String::new() } else { cap.clone() } }),
            capability::PERSON_ACCESS,
        ),
        PeopleCmd::Approve { client_id, code } => (
            "client.approve_step_up",
            Payload::ClientApproveStepUp(pb::ClientApproveStepUp { client_id: client_id.trim().to_string(), code: code.trim().to_string() }),
            capability::STEP_UP,
        ),
    };
    let mut r = with(req(method), payload);
    r.required_capabilities.push(needs.to_string());
    r
}

/// A change on `link`, answering what to print: the people as they are now
/// for an access or a limit, the key's state for an approval.
pub(crate) async fn change_on<L: DispatchLink>(link: &mut L, change: PeopleCmd, json: bool) -> Result<String, Box<dyn std::error::Error>> {
    let needs = if matches!(change, PeopleCmd::Approve { .. }) { capability::STEP_UP } else { capability::PERSON_ACCESS };
    if !link.capabilities().iter().any(|c| c == needs) {
        return Err(NO_ACCESS.into());
    }
    let answer = link.call(change_request(&change)).await.map_err(crate::people::refused)?;
    match expect_value(answer.value)? {
        result::Value::MemberList(list) => Ok(if json { access_json(&list).to_string() } else { access_lines(&list) }),
        result::Value::StepUpKeyState(state) => Ok(if json {
            json!({ "client_id": state.client_id, "state": state.state }).to_string()
        } else {
            format!("{}'s key is {}", state.client_id, state.state)
        }),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
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
                "step_up": d.step_up,
                "step_up_code": d.step_up_code,
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

//! People's routes (ov-494): `member.list`, `member.set`, `member.remove` and
//! `rep.start`, behind `representatives`. The rules are in `people`, the
//! records in `farcooler_store::people`, and a rep's start in `reps`.
//!
//! Every answer about members carries the Claude account the runner uses,
//! because joining a shared runner is joining that account (ov-479 D2), and
//! a person adding someone, or listing who's here, is where to say so.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};
use farcooler_transport::Peer;

use crate::people::{Person, person_of};
use crate::service::Service;
use crate::task_ops::required_id;
use crate::watch::Watcher;
use crate::wire::id_bytes;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

/// One people route, as `Rpc::dispatch` hands it over, from `peer`.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, peer: &Peer, req: Request) -> Result<result::Value> {
    match req.method.as_str() {
        "member.list" => Ok(result::Value::MemberList(list(svc, peer)?)),
        "member.set" => {
            let Some(request::Payload::MemberSet(p)) = req.payload else { return Err(payload_missing()) };
            // Only devices this runner enrolled: a typo bound to a member would
            // leave their real phone the owner's.
            let enrolled = crate::enrollment::list(svc).await?;
            let known = |d: &String| enrolled.items.iter().any(|e| !e.client_id.is_empty() && e.client_id == d.trim());
            if !p.client_ids.iter().filter(|d| !d.trim().is_empty()).all(known) {
                return Err(DomainError::InvalidArgument { what: "device" });
            }
            // Who they are in the team directory, when the Mac that added
            // them by QR says (ov-525). Stored as told: a runner admin's
            // word, like the name, and what offboarding will look them up by.
            let who = farcooler_store::people::Directory { account: &p.account, email: &p.email };
            svc.store.set_member_in(p.handle.trim(), &p.name, &p.client_ids, who)?;
            watcher.announce_fleet_changed();
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "member.remove" => {
            let Some(request::Payload::MemberRemove(p)) = req.payload else { return Err(payload_missing()) };
            let handle = p.handle.trim();
            let member = svc.store.member(handle)?;
            svc.stop_reps_of(handle).await?;
            // Their devices lose their access here, not only their say: a
            // released device keeps its enrolled scope, and Control is a shell.
            // A member with no devices has nothing to revoke, so their removal
            // never waits on reading `authorized_keys` (ov-506). One with
            // devices is refused when that file can't be read: "removed" must
            // not leave a device logging in.
            if !member.devices.is_empty() {
                let enrolled = crate::enrollment::list(svc).await?;
                for device in member.devices.iter().filter(|d| enrolled.items.iter().any(|e| &e.client_id == *d)) {
                    crate::enrollment::revoke(svc, &pb::ClientRevoke { client_id: device.clone() }).await?;
                }
            }
            // A workspace they led is the owner's now: its orchestrator starts
            // afresh on the owner's login, not left on theirs (ov-585).
            let led = svc.led_by(handle)?;
            svc.store.remove_member(handle)?;
            let started = svc.follow_leads(&led).await;
            watcher.announce_fleet_changed();
            crate::lead_change::publish_layouts(svc, watcher, &started).await;
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "rep.start" => {
            let Some(request::Payload::RepStart(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let handle = p.handle.trim();
            // A runner admin or the owner starts anyone's; a member, their own.
            if !crate::people::may_start_rep(&person_of(&svc.store, peer)?, peer.scope, handle) {
                return Err(DomainError::ScopeDenied { needed: "host_admin" });
            }
            let started = svc.start_rep(workspace, handle, &p.harness).await?;
            if let Ok(groups) = svc.layout(started.terminal.worktree_id).await {
                watcher.publish_layout(started.terminal.worktree_id, &groups);
            }
            watcher.announce_fleet_changed();
            Ok(result::Value::RepStarted(pb::RepStarted {
                terminal_id: id_bytes(started.terminal.id),
                desk_task_key: started.desk.key,
                claude_account: svc.claude_account().unwrap_or_default(),
            }))
        }
        _ => Err(DomainError::CapabilityUnsupported { needed: "a newer Far Cooler" }),
    }
}

/// `MemberList.you` for a device that was a member's and is nobody's now: no
/// handle can hold parentheses, so it's never mistaken for one.
const RELEASED: &str = "(released)";

/// Everyone on the runner, the account they share, and who's asking.
fn list(svc: &Service, peer: &Peer) -> Result<pb::MemberList> {
    let members = svc
        .store
        .members()?
        .into_iter()
        .map(|m| {
            let reps = svc.store.reps_of(&m.handle)?;
            Ok(pb::Member {
                rep_terminal_ids: reps.iter().map(|r| id_bytes(r.terminal)).collect(),
                rep_workspace_ids: reps.iter().map(|r| id_bytes(r.workspace)).collect(),
                handle: m.handle,
                name: m.name,
                client_ids: m.devices,
                account: m.account.unwrap_or_default(),
                email: m.email.unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let you = match person_of(&svc.store, peer)? {
        Person::Owner => String::new(),
        Person::Member(handle) => handle,
        Person::Released => RELEASED.to_string(),
    };
    // The account's email is the owner's own: a device enrolled only to look
    // is told the account is shared, not whose it is.
    let claude_account = match peer.scope {
        Scope::Control | Scope::HostAdmin => svc.claude_account().unwrap_or_default(),
        _ => String::new(),
    };
    Ok(pb::MemberList { members, claude_account, you })
}

#[cfg(test)]
#[path = "rpc_people_tests.rs"]
mod tests;

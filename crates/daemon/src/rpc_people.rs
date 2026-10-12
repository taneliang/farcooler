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
            svc.store.set_member(p.handle.trim(), &p.name, &p.client_ids)?;
            watcher.announce_fleet_changed();
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "member.remove" => {
            let Some(request::Payload::MemberRemove(p)) = req.payload else { return Err(payload_missing()) };
            let handle = p.handle.trim();
            svc.store.member(handle)?;
            svc.stop_reps_of(handle).await?;
            svc.store.remove_member(handle)?;
            watcher.announce_fleet_changed();
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "rep.start" => {
            let Some(request::Payload::RepStart(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let handle = p.handle.trim();
            // A runner admin or the owner starts anyone's; a member, their own.
            let who = person_of(&svc.store, peer)?;
            let own = who == Person::Member(handle.to_string());
            if peer.scope != Scope::HostAdmin && who != Person::Owner && !own {
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
                claude_account: crate::people::runner_claude_account().unwrap_or_default(),
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
                handle: m.handle,
                name: m.name,
                client_ids: m.devices,
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
        Scope::Control | Scope::HostAdmin => crate::people::runner_claude_account().unwrap_or_default(),
        _ => String::new(),
    };
    Ok(pb::MemberList { members, claude_account, you })
}

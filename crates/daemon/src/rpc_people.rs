//! People's routes (ov-494): `member.list`, `member.set`, `member.remove` and
//! `rep.start`, behind `representatives`, `member.connectors` (ov-586),
//! behind `rep_connectors`, `member.set_own_login` (ov-652), behind
//! `member_own_logins`, and `member.set_role` and `member.events` (ov-757),
//! behind `member_roles`; Claude sign-in's (ov-584) go on to
//! `rpc_claude_sign_in`. The rules are in `people`, the
//! records in `farcooler_store::people`, and a rep's start in `reps`.
//!
//! Each change to who's here goes to the people log (`member.events`): a
//! member added, their devices changed, a member removed, a role given or
//! taken, with who did it.
//!
//! Every answer about members carries the Claude account the runner uses,
//! because joining a shared runner is joining that account (ov-479 D2), and
//! a person adding someone, or listing who's here, is where to say so.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, Scope, request, result};
use farcooler_transport::Peer;

use farcooler_store::member_roles::{NewPeopleEvent, Role};

use crate::people::{Person, person_of};
use crate::service::Service;
use crate::task_ops::required_id;
use crate::watch::Watcher;
use crate::wire::id_bytes;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

/// One people route, as `Rpc::dispatch` hands it over, from `peer`.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, peer: &Peer, caller_pid: Option<i32>, req: Request) -> Result<result::Value> {
    match req.method.as_str() {
        // A member's own Claude login (ov-584).
        "claude.sign_in" | "claude.sign_in_finish" | "claude.sign_in_cancel" | "claude.sign_out" => {
            crate::rpc_claude_sign_in::dispatch(svc, watcher, peer, caller_pid, req).await
        }
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
            let before = match svc.store.member(p.handle.trim()) {
                Err(DomainError::NotFound) => None,
                other => Some(other?),
            };
            let after = svc.store.set_member_in(p.handle.trim(), &p.name, &p.client_ids, who)?;
            match before {
                None => log(svc, peer, caller_pid, &after.handle, "added", serde_json::json!({ "name": after.name, "devices": after.devices })),
                Some(b) if b.devices != after.devices => {
                    log(svc, peer, caller_pid, &after.handle, "devices", serde_json::json!({ "from": b.devices, "to": after.devices }))
                }
                Some(_) => {}
            }
            svc.refresh_claude_key_exports();
            watcher.announce_fleet_changed();
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "member.remove" => {
            let Some(request::Payload::MemberRemove(p)) = req.payload else { return Err(payload_missing()) };
            let handle = p.handle.trim();
            let member = match svc.store.member(handle) {
                Err(DomainError::NotFound) => {
                    // Removed already: run again, it wipes what's left of
                    // their login (`member_removal::rewipe_gone`).
                    let removed = svc.rewipe_gone(handle).await?.ok_or(DomainError::NotFound)?;
                    watcher.announce_fleet_changed();
                    return Ok(result::Value::MemberList(pb::MemberList { removed: Some(removed), ..list(svc, peer)? }));
                }
                other => other?,
            };
            // Their Claude login, read before the row that names it goes,
            // and what runs on it now (ov-588).
            let login = svc.store.member_login(handle)?;
            let running = svc.running_on_login(handle, login.as_deref())?;
            svc.stop_reps_of(handle).await?;
            // Their devices lose their access here, not only their say: a
            // released device keeps its enrolled scope, and Control is a shell.
            // A member with no devices has nothing to revoke, so their removal
            // never waits on reading `authorized_keys` (ov-506). One with
            // devices is refused when that file can't be read: "removed" must
            // not leave a device logging in.
            // And the devices they enrolled that nobody was bound to (ov-719):
            // released already, but a line in `authorized_keys` all the same.
            let mut devices = member.devices.clone();
            devices.extend(svc.store.unbound_enrolled_by(handle)?);
            // Every id, not only those with a line now: an enroll holding the
            // fence lock may have committed its row and not yet written its
            // line, and `revoke` waits for that lock (ov-720). A failure is
            // refused after the rest were tried, while they're still a member.
            if let Some(err) = revoke_all(svc, &devices).await {
                return Err(err);
            }
            // A workspace they led is the owner's now: its orchestrator starts
            // afresh on the owner's login, not left on theirs (ov-585).
            let led = svc.led_by(handle)?;
            // Read with the removal, in one transaction: a device their Mac
            // enrolled after the revokes above is revoked here, and one after
            // this is refused (`record_member_enrolled`, review F3).
            let late = svc.store.remove_member_with_enrolled(handle)?;
            // They're gone now, so a failure is logged and the rest still go.
            if let Some(err) = revoke_all(svc, &late).await {
                tracing::error!(?err, "a device a removed member enrolled may still have a line");
            }
            svc.refresh_claude_key_exports();
            // A sign-in under way ends with them, and a finish under way is
            // waited for, once nothing new can start (ov-584, review L2).
            svc.claude_logins.sign_ins.end_and_wait(handle).await;
            let started = svc.follow_leads(&led).await;
            // Then the login itself: every pane still on it stopped, its
            // Keychain entry and its directory deleted. Gone from the store
            // first, so nothing can launch on it in between (`member_removal`).
            let removed = svc.wipe_login(login.as_deref(), &running).await;
            log(svc, peer, caller_pid, handle, "removed", serde_json::json!({ "devices": member.devices }));
            watcher.announce_fleet_changed();
            crate::lead_change::publish_layouts(svc, watcher, &started).await;
            Ok(result::Value::MemberList(pb::MemberList { removed: Some(removed), ..list(svc, peer)? }))
        }
        "member.set_own_login" => {
            let Some(request::Payload::MemberOwnLoginSet(p)) = req.payload else { return Err(payload_missing()) };
            // `host_admin`, by the method's scope: the runner's admins (ov-652).
            // And a person, never an agent in a pane, whose local socket is
            // `host_admin` too: a member's agent must not lend itself the
            // owner's logins, nor take them from everyone.
            // Nor from inside any pane, a person's own shell included, as a
            // sign-in is refused (`may_sign_in`, review 1, item 4).
            if caller_pid.and_then(|pid| crate::people::pane_of(svc, pid)).is_some() {
                tracing::info!("refused: a change to members' own logins from inside a pane");
                return Err(DomainError::Conflict { what: "in_pane" });
            }
            if crate::caller_seat::caller_seat(svc, caller_pid).await != crate::caller_seat::Seat::Person {
                tracing::info!("refused: a change to members' own logins from a caller who isn't a person");
                return Err(DomainError::Conflict { what: "person_only" });
            }
            if svc.set_own_login(&p, peer.client_id.as_deref().unwrap_or("-"))? {
                watcher.announce_needs_you();
                watcher.announce_fleet_changed();
            }
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "member.set_role" => {
            let Some(request::Payload::MemberSetRole(p)) = req.payload else { return Err(payload_missing()) };
            let role = Role::parse(p.role.trim()).ok_or(DomainError::InvalidArgument { what: "role" })?;
            // The owner's alone (`Why::Grants`, by the gate), and a person,
            // never an agent in a pane, as lending a login is: the local
            // socket is the owner's to the gate, and a role widens what a
            // member's devices may do.
            if caller_pid.and_then(|pid| crate::people::pane_of(svc, pid)).is_some() {
                tracing::info!("refused: a role change from inside a pane");
                return Err(DomainError::Conflict { what: "in_pane" });
            }
            if crate::caller_seat::caller_seat(svc, caller_pid).await != crate::caller_seat::Seat::Person {
                tracing::info!("refused: a role change from a caller who isn't a person");
                return Err(DomainError::Conflict { what: "person_only" });
            }
            let handle = p.handle.trim();
            if svc.store.set_member_role(handle, role, &crate::caller_person::log_actor(svc, peer, caller_pid)?)? != role {
                tracing::info!(handle, role = role.as_str(), "a member's role changed");
                watcher.announce_fleet_changed();
            }
            Ok(result::Value::MemberList(list(svc, peer)?))
        }
        "member.events" => {
            let events = svc.store.people_events(EVENTS_SHOWN)?;
            Ok(result::Value::PeopleEventList(pb::PeopleEventList {
                events: events
                    .into_iter()
                    .map(|e| pb::PeopleEvent {
                        at_ms: e.at,
                        actor: e.actor,
                        handle: e.handle,
                        workspace_id: e.workspace.map(id_bytes).unwrap_or_default(),
                        kind: e.kind,
                        detail: e.detail,
                    })
                    .collect(),
            }))
        }
        "rep.start" => {
            let Some(request::Payload::RepStart(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let handle = p.handle.trim();
            // The owner starts anyone's; a member, their own (ov-716 rule 3).
            if !crate::people::may_start_rep(&person_of(&svc.store, peer)?, handle) {
                return Err(DomainError::NotYours { what: "not_yours" });
            }
            // A member with no Claude login is asked to sign in, never given
            // a rep on the owner's (ov-587).
            if let Err(e) = svc.refuse_rep_until_signed_in(handle) {
                watcher.announce_needs_you();
                return Err(e);
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
        "member.connectors" => {
            let Some(request::Payload::MemberConnectors(p)) = req.payload else { return Err(payload_missing()) };
            let handle = p.handle.trim();
            // Their own, from their own device, or the owner's say for anyone
            // (ov-586). Not a runner admin's for someone else: it's the
            // member's accounts their rep would act in.
            if !crate::people::may_set_connectors(&person_of(&svc.store, peer)?, handle) {
                return Err(DomainError::NotYours { what: "not_yours" });
            }
            svc.store.member(handle)?;
            let servers = if p.set {
                let kept = svc.store.set_member_connectors(handle, &p.servers)?;
                // One turned off is refused from its rep's next call.
                svc.refresh_rep_connectors(handle)?;
                tracing::info!(handle, count = kept.len(), "a member's rep connectors changed");
                kept
            } else {
                svc.store.member_connectors(handle)?
            };
            Ok(result::Value::MemberConnectorList(pb::MemberConnectorList { handle: handle.to_string(), servers }))
        }
        _ => Err(DomainError::CapabilityUnsupported { needed: "a newer Far Cooler" }),
    }
}

/// Revoke each of `devices`, all of them whatever any one does. A device with
/// no line is done (`NotFound`); `revoke` takes the fence lock, so an enroll
/// mid-write is waited for rather than missed. The first other failure, if any.
async fn revoke_all(svc: &Service, devices: &[String]) -> Option<DomainError> {
    let mut failed = None;
    for device in devices {
        match crate::enrollment::revoke(svc, &pb::ClientRevoke { client_id: device.clone() }).await {
            Ok(_) | Err(DomainError::NotFound) => {}
            Err(err) => {
                tracing::error!(device = %device, ?err, "revoking a removed member's device failed");
                failed.get_or_insert(err);
            }
        }
    }
    failed
}

/// The most of the people log `member.events` answers, newest first.
const EVENTS_SHOWN: u32 = 200;

/// Add a change already made to the people log, as the caller's person's
/// (`caller_person::log_actor`). A line that can't be written is logged and
/// isn't the change failing.
fn log(svc: &Service, peer: &Peer, caller_pid: Option<i32>, handle: &str, kind: &str, detail: serde_json::Value) {
    let written = crate::caller_person::log_actor(svc, peer, caller_pid).and_then(|actor| {
        svc.store.record_people_event(NewPeopleEvent { actor: &actor, handle, workspace: None, kind, detail: &detail })
    });
    if let Err(e) = written {
        tracing::warn!(handle, kind, error = %e.what(), "a change to the people here wasn't logged");
    }
}

/// `MemberList.you` for a device that was a member's and is nobody's now: no
/// handle can hold parentheses, so it's never mistaken for one.
const RELEASED: &str = "(released)";

/// Everyone on the runner, the account they share, and who's asking.
fn list(svc: &Service, peer: &Peer) -> Result<pb::MemberList> {
    let viewer = person_of(&svc.store, peer)?;
    // A released device learns that it was, and nothing else (ov-758): no
    // member, no account, no logins.
    if viewer == Person::Released {
        return Ok(pb::MemberList { you: RELEASED.to_string(), ..Default::default() });
    }
    let roles = svc.store.member_roles()?;
    let members = svc
        .store
        .members()?
        .into_iter()
        .map(|m| {
            let reps = svc.store.reps_of(&m.handle)?;
            // Their Claude account's email is theirs and the owner's to read
            // (ov-584); everyone else is told only whether they're signed in.
            let own = matches!(&viewer, Person::Owner) || matches!(&viewer, Person::Member(me) if *me == m.handle);
            let claude_login = Some(svc.member_claude_login(&m.handle, own));
            Ok(pb::Member {
                role: roles.get(&m.handle).copied().unwrap_or_default().as_str().to_string(),
                claude_login,
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
    let you = match viewer {
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
    let own_logins = Some(svc.own_logins_wire(peer.scope));
    Ok(pb::MemberList { members, claude_account, you, removed: None, own_logins })
}

#[cfg(test)]
#[path = "rpc_people_tests.rs"]
mod tests;

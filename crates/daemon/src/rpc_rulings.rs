//! Decided for you (ov-304): `ruling.add` and `ruling.set`, and (ov-333)
//! `ruling.keep_all`, behind
//! `board_rulings`. Read through `plan.get`, whose `Plan.rulings` this file
//! also converts.
//!
//! Part of the plan layer, and beside the board as it is: nothing here touches
//! `task_ops` or emits `task_changed`. Each write announces `plan_changed`
//! once, so a client with the plan open re-reads it.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use farcooler_store::models::Actor;
use farcooler_store::rulings::{NewRuling, Ruling, RulingState};

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

/// One ruling route, as `rpc_plan::dispatch` hands it over.
pub(crate) fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    let missing = || DomainError::InvalidArgument { what: "payload" };
    match (req.method.as_str(), req.payload) {
        ("ruling.add", Some(request::Payload::RulingAdd(p))) => {
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let tasks = p.task_ids.iter().map(|id| required_id(id)).collect::<Result<Vec<_>>>()?;
            let new = NewRuling {
                decision: p.decision,
                why: p.why,
                reversal: p.reversal,
                theme_id: p.theme_id.as_deref().map(required_id).transpose()?,
            };
            let ruling = svc.store.add_ruling(workspace, &new, &tasks, actor)?;
            watcher.announce_plan_changed(workspace, actor);
            Ok(result::Value::BoardRuling(pb_ruling(&ruling)))
        }
        ("ruling.set", Some(request::Payload::RulingSet(p))) => {
            let id = required_id(&p.ruling_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let state = match pb::BoardRulingState::try_from(p.state) {
                Ok(pb::BoardRulingState::Standing) => RulingState::Standing,
                Ok(pb::BoardRulingState::Confirmed) => RulingState::Confirmed,
                Ok(pb::BoardRulingState::Reversed) => RulingState::Reversed,
                _ => return Err(DomainError::InvalidArgument { what: "state" }),
            };
            // A marked reversal is the orchestrator's note that the work is
            // done, with the commit that did it (ov-333).
            let ruling = match (state, p.sha.as_deref()) {
                (RulingState::Reversed, Some(sha)) => svc.store.reverse_ruling(id, sha, p.note.as_deref(), actor)?,
                (_, Some(_)) => return Err(DomainError::InvalidArgument { what: "reversed_sha" }),
                (_, None) => svc.store.set_ruling(id, state, p.note.as_deref(), actor)?,
            };
            watcher.announce_plan_changed(ruling.workspace_id, actor);
            Ok(result::Value::BoardRuling(pb_ruling(&ruling)))
        }
        ("ruling.keep_all", Some(request::Payload::RulingKeepAll(p))) => {
            let workspace = required_id(&p.workspace_id)?;
            // Keep All is the owner's own mark: the orchestrator doesn't
            // settle a ruling on the owner's behalf in bulk.
            let actor = match actor_from_wire(&p.actor)? {
                Actor::User => Actor::User,
                _ => return Err(DomainError::InvalidArgument { what: "actor" }),
            };
            let kept = svc.store.keep_all_rulings(workspace, actor)?;
            if !kept.is_empty() {
                watcher.announce_plan_changed(workspace, actor);
            }
            Ok(result::Value::RulingsKept(pb::RulingsKept { rulings: kept.iter().map(pb_ruling).collect() }))
        }
        ("ruling.add" | "ruling.set" | "ruling.keep_all", _) => Err(missing()),
        (other, _) => {
            tracing::error!(method = %other, "a ruling route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

/// A ruling as the wire carries it.
pub(crate) fn pb_ruling(r: &Ruling) -> pb::BoardRuling {
    pb::BoardRuling {
        id: id_bytes(r.id),
        workspace_id: id_bytes(r.workspace_id),
        number: r.number,
        decision: r.decision.clone(),
        why: r.why.clone(),
        reversal: r.reversal.clone(),
        task_ids: r.tasks.iter().map(|id| id_bytes(*id)).collect(),
        task_keys: r.task_keys.clone(),
        theme_id: r.theme_id.map(id_bytes),
        state: (match r.state {
            RulingState::Standing => pb::BoardRulingState::Standing,
            RulingState::Confirmed => pb::BoardRulingState::Confirmed,
            RulingState::Reversed => pb::BoardRulingState::Reversed,
        }) as i32,
        note: r.note.clone(),
        actor: r.actor.clone(),
        created_at: r.created_at,
        settled_by: r.settled_by.clone(),
        settled_at: r.settled_at,
        reversed_sha: r.reversed_sha.clone(),
        resource_version: r.resource_version,
    }
}

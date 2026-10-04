//! Lines `farcooler events` prints for the events a client reads by shape.

use farcooler_protocol::v1 as pb;

use crate::{short_bytes, uuid_of};

/// A task notice, as the Mac's `NoticeEvent` reads it (ov-94).
pub(crate) fn notice_json(n: &pb::Notice) -> serde_json::Value {
    serde_json::json!({
        "kind": "notice",
        "notice_id": n.notice_id,
        "event": n.event,
        "level": n.level,
        "title": n.title,
        "body": n.body,
        "task": n.task_key,
        "runner": n.runner_id,
        "workspace": n.workspace,
        "options": n.options,
        // The task's repository, which a key is unique within (ov-106).
        // Null from a runner too old to say.
        "repository": (!n.repository_id.is_empty()).then(|| uuid_of(&n.repository_id).to_string()),
    })
}

/// A board's read state moved on another device (ov-113): the whole state,
/// which a client merges as it is. Not `task`: no task moved, and that line
/// makes every board re-read.
pub(crate) fn reads_event_json(r: &pb::BoardReads) -> serde_json::Value {
    let mut line = farcooler_client::session::reads_json(r);
    line["kind"] = serde_json::json!("reads");
    line
}

/// A repository's board moved.
///
/// A named function rather than an object built inline in `events`, for the
/// reason `terminal_event_json` above is one: a hand-built object in a match
/// arm is a thing with no test, and the field most likely to be dropped from
/// one is the field a client cannot work without. Here that is `actor` —
/// without it the line still says "the board moved" and every client keeps
/// working, and the one thing that quietly stops being possible is telling
/// your own write apart from somebody else's.
///
/// The repository and not just the task, because `task list` answers a whole
/// board in one call and a board is what is on screen: a client told only
/// which task moved would still have to read the board to know where the row
/// goes now. See `FleetEvent::Task` in crates/client/src/session.rs, which is
/// the same news over the other transport.
pub(crate) fn task_event_json(t: &farcooler_protocol::v1::TaskChanged) -> serde_json::Value {
    serde_json::json!({
        "kind": "task",
        "task": uuid_of(&t.task_id).to_string(),
        "short": short_bytes(&t.task_id),
        "repository": uuid_of(&t.repository_id).to_string(),
        // `user`, `manager`, or `agent:<uuid>` — the daemon's `Actor` display,
        // verbatim. Never parsed into parts here: two fields are two things
        // that can disagree, which is the argument the proto's own comment on
        // this field makes.
        "actor": t.actor,
        // Which board moved, and on a move the board it left too, so a
        // client showing either one reads it again and a client showing
        // neither doesn't. Null from a runner without workspaces, where the
        // repository is the one board. The FFI's event line spells them the
        // same way.
        "workspace": crate::workspaces_json::workspace_of(t.workspace_id.as_deref()),
        "from_workspace": crate::workspaces_json::workspace_of(t.from_workspace_id.as_deref()),
    })
}

/// The plan layer on a board moved (ov-268): a theme, a lane or the plan was
/// written. The Mac's Plan view re-reads `plan --json` on it. Not `task`: no
/// task moved, and that line makes every board re-read.
pub(crate) fn plan_event_json(p: &pb::PlanChanged) -> serde_json::Value {
    serde_json::json!({
        "kind": "plan",
        "workspace": uuid_of(&p.workspace_id).to_string(),
        "actor": p.actor,
    })
}

/// A page on a board was written or removed (ov-269). A client with that page
/// open re-reads the slot. Not `task`: no task moved.
pub(crate) fn pages_event_json(p: &pb::PagesChanged) -> serde_json::Value {
    serde_json::json!({
        "kind": "pages",
        "workspace": uuid_of(&p.workspace_id).to_string(),
        "slot": p.slot,
        "revision": p.revision,
        "actor": p.actor,
        "removed": p.removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pages line names the board and the slot, and says when the page went.
    #[test]
    fn a_pages_line_names_its_board_slot_and_whether_it_went() {
        let ws = uuid::Uuid::now_v7();
        let line = pages_event_json(&pb::PagesChanged {
            workspace_id: bytes::Bytes::copy_from_slice(ws.as_bytes()),
            slot: "train".into(),
            revision: 3,
            actor: "manager".into(),
            removed: true,
        });
        assert_eq!(line["kind"], "pages");
        assert_eq!(line["workspace"], ws.to_string());
        assert_eq!((line["slot"].as_str(), line["revision"].as_u64(), line["removed"].as_bool()), (Some("train"), Some(3), Some(true)));
    }

    /// **A plan line names the board it's about** (ov-273): the Mac's
    /// `EventStream` reads `workspace` to re-read that board's plan and no
    /// other, so a key renamed here is a Plan view that never moves.
    #[test]
    fn a_plan_line_names_its_board_and_actor() {
        let ws = uuid::Uuid::now_v7();
        let line = plan_event_json(&pb::PlanChanged {
            workspace_id: bytes::Bytes::copy_from_slice(ws.as_bytes()),
            actor: "manager".into(),
        });
        assert_eq!(line["kind"], "plan");
        assert_eq!(line["workspace"], ws.to_string());
        assert_eq!(line["actor"], "manager");
    }
}

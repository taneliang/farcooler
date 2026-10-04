//! Lines `farcooler events` prints for the events a client reads by shape.

use farcooler_protocol::v1 as pb;

use crate::uuid_of;

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

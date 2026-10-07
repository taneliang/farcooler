//! Small readings of a `Terminal` for the fleet's JSON rows, out of
//! `session.rs` for its size ceiling.

use farcooler_protocol::v1::Terminal;
use uuid::Uuid;

/// A fleet row with the draft held on its terminal behind a dialog (ov-385),
/// as `draftHold`, so the phone that sent it can say whether it went in.
/// Absent when there's none. Set here, not in the row's `json!`, which is at
/// the macro's recursion limit.
pub(super) fn with_draft_hold(t: &Terminal, mut row: serde_json::Value) -> serde_json::Value {
    if let (Some(hold), Some(row)) = (&t.draft_hold, row.as_object_mut()) {
        row.insert("draftHold".into(), super::draft_prompt::draft_hold_json(hold));
    }
    row
}

/// The board task a terminal was opened for, as a uuid string, or nothing.
///
/// Nothing for a pane nobody dispatched, for a runner too old to record one
/// (`capability::TERMINAL_TASK`), and for bytes that are not a uuid at all —
/// never the nil uuid, which `uuid_of` would hand back and which a client
/// would then match against nothing and draw as a link to nowhere. The same
/// rule as the CLI's `task_of`, which projects the same field for the Mac.
pub(super) fn task_of(t: &Terminal) -> Option<String> {
    t.task_id.as_deref().and_then(|b| Uuid::from_slice(b).ok()).map(|u| u.to_string())
}

/// The terminal `t` was split from, as a uuid string, or nothing: a pane in
/// a window of its own, one from before splits were recorded, and an older
/// runner all say nothing. Never the nil uuid, for `task_of`'s reason.
pub(super) fn split_of(t: &Terminal) -> Option<String> {
    t.split_of.as_deref().and_then(|b| Uuid::from_slice(b).ok()).map(|u| u.to_string())
}

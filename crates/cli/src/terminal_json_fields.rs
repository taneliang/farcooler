//! Fields of a terminal's JSON that need a little more than a copy: the task
//! and split a pane came from, and whose Claude login it runs on (ov-597).

use uuid::Uuid;

/// The board task a terminal was opened for, as the uuid a board row carries.
///
/// `None` for a terminal nobody dispatched, for a runner too old to record one
/// (`capability::TERMINAL_TASK`), and for bytes that are not a uuid at all —
/// never the nil uuid, which `uuid_of` would hand back and which a client
/// would then match against nothing and draw as a link to nowhere.
pub(crate) fn task_of(t: &farcooler_protocol::v1::Terminal) -> Option<String> {
    t.task_id.as_deref().and_then(|b| Uuid::from_slice(b).ok()).map(|u| u.to_string())
}

/// The terminal `t` was split from, as a uuid string, or nothing: for a pane
/// opened in a window of its own, one recorded before splits were, and a
/// runner too old to say. Never the nil uuid, for `task_of`'s reason.
pub(crate) fn split_of(t: &farcooler_protocol::v1::Terminal) -> Option<String> {
    t.split_of.as_deref().and_then(|b| Uuid::from_slice(b).ok()).map(|u| u.to_string())
}

/// Whether the pane `t` was split from was the orchestrator at the time, or
/// nothing: not a split, a split recorded before this was, or a runner too
/// old to say. Only beside a `split_of` that reads, so the two keys never
/// disagree about whether this is a split.
pub(crate) fn split_of_orchestrator(t: &farcooler_protocol::v1::Terminal) -> Option<bool> {
    split_of(t).and(t.split_of_orchestrator)
}

/// `{handle, name, owner}` for the person whose Claude login the pane's agent
/// runs on, or null where the runner says nothing (a runner with only the
/// owner's login, or one from before the field).
pub(crate) fn claude_login_json(t: &farcooler_protocol::v1::Terminal) -> serde_json::Value {
    match &t.claude_login {
        Some(login) => serde_json::json!({ "handle": login.handle, "name": login.name, "owner": login.owner }),
        None => serde_json::Value::Null,
    }
}

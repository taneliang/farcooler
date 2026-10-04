//! `workspace.mark_read`'s arguments, as an app passes them (ov-113).

use serde_json::Value;
use uuid::Uuid;

use crate::session::{MarkRead, SessionError};

/// `{workspace, floor_ms?, opened: [{task_id, opened_ms}]}`.
///
/// Refused here, before the round trip, for a missing or unreadable id or
/// time: a mark with no time is not "now", because a device's clock is the one
/// thing a shared mark must never carry.
pub(super) fn mark_read_of(args: &Value) -> Result<(Uuid, MarkRead), SessionError> {
    const METHOD: &str = "workspace.mark_read";
    let needs = |what: &str| SessionError::Protocol(format!("{METHOD} needs {what}"));
    let id = |v: Option<&Value>, what: &str| {
        v.and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok()).ok_or_else(|| needs(what))
    };
    let workspace = id(args.get("workspace"), "a workspace")?;
    let floor_ms = match args.get("floor_ms") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or_else(|| needs("a floor_ms that is a number"))?),
    };
    let mut opened = Vec::new();
    for mark in args.get("opened").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let task = id(mark.get("task_id"), "a task_id in each opened mark")?;
        let ms = mark.get("opened_ms").and_then(Value::as_i64).ok_or_else(|| needs("an opened_ms in each opened mark"))?;
        opened.push((task, ms));
    }
    Ok((workspace, MarkRead { floor_ms, opened }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_arguments_an_app_passes_become_the_write() {
        let (ws, task) = (Uuid::now_v7(), Uuid::now_v7());
        let args = json!({
            "workspace": ws.to_string(),
            "floor_ms": 5,
            "opened": [{ "task_id": task.to_string(), "opened_ms": 9 }],
        });
        assert_eq!(
            mark_read_of(&args).unwrap(),
            (ws, MarkRead { floor_ms: Some(5), opened: vec![(task, 9)] })
        );
        let open = json!({ "workspace": ws.to_string(), "opened": [{ "task_id": task.to_string(), "opened_ms": 9 }] });
        assert_eq!(mark_read_of(&open).unwrap().1.floor_ms, None, "an open raises no floor");
    }

    #[test]
    fn a_mark_with_no_time_is_refused_and_never_means_now() {
        let ws = Uuid::now_v7().to_string();
        let task = Uuid::now_v7().to_string();
        for args in [
            json!({}),
            json!({ "workspace": ws, "opened": [{ "task_id": task }] }),
            json!({ "workspace": ws, "opened": [{ "opened_ms": 3 }] }),
            json!({ "workspace": ws, "floor_ms": "soon" }),
        ] {
            assert!(matches!(mark_read_of(&args), Err(SessionError::Protocol(_))), "{args}");
        }
    }

    /// The line an app reads for a board's read state moving elsewhere.
    #[test]
    fn the_event_line_is_the_state() {
        let (ws, task) = (Uuid::now_v7(), Uuid::now_v7());
        let line = super::super::event_line(&crate::session::FleetEvent::Reads {
            workspace: ws,
            floor_ms: 4,
            opened: vec![(task, 8)],
        });
        let line: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            line,
            json!({
                "event": "reads",
                "workspace_id": ws.to_string(),
                "floor_ms": 4,
                "opened": [{ "task_id": task.to_string(), "opened_ms": 8 }],
            })
        );
    }
}

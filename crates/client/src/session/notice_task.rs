//! `noticeTaskId`, the task a terminal's own notifications fold into (ov-112).

use farcooler_protocol::v1::Terminal;
use uuid::Uuid;

/// The board task `t`'s own notifications fold into, as a uuid string, or
/// nothing: the runner's `task_link::notice_task`, which no client keeps a copy
/// of. Nothing for a pane that notifies as itself, for a runner without
/// `capability::NOTICE_TASK`, and for bytes that are not a uuid or are the nil one.
pub(super) fn notice_task_of(t: &Terminal) -> Option<String> {
    t.notice_task_id
        .as_deref()
        .and_then(|b| Uuid::from_slice(b).ok())
        .filter(|u| !u.is_nil())
        .map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    /// The task a pane's own notifications fold into is a uuid string or
    /// nothing, never the nil uuid: the phones fold a banner into a task only
    /// on this key (ov-112).
    #[test]
    fn a_pane_names_the_task_its_notices_fold_into_or_nothing() {
        let task = uuid::Uuid::now_v7();
        let pane = |notice_task_id| farcooler_protocol::v1::Terminal { notice_task_id, ..Default::default() };
        let terminals = [
            pane(Some(bytes::Bytes::copy_from_slice(task.as_bytes()))),
            pane(None),
            pane(Some(bytes::Bytes::from_static(&[0; 16]))),
            pane(Some(bytes::Bytes::from_static(b"short"))),
        ];
        let w = farcooler_protocol::v1::Worktree::default();
        let row = serde_json::json!({ "terminals": [{}, {}, {}, {}] });
        let row = super::super::with_workspaces(row, &w, &terminals, &[]);
        assert_eq!(row["terminals"][0]["noticeTaskId"], task.to_string(), "{row}");
        for n in 1..4 {
            assert_eq!(row["terminals"][n]["noticeTaskId"], serde_json::json!(null), "{n}: {row}");
        }
    }
}

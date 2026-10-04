//! `noticeTaskId`, the task a terminal's own notifications fold into (ov-112).

use uuid::Uuid;

/// The board task `t`'s own notifications fold into, as a uuid string, or
/// nothing: the runner's `task_link::notice_task`, which no client keeps a copy
/// of. Nothing for a pane that notifies as itself, for a runner without
/// `capability::NOTICE_TASK`, and for bytes that are not a uuid or are the nil one.
pub(crate) fn notice_task_of(t: &farcooler_protocol::v1::Terminal) -> Option<String> {
    t.notice_task_id
        .as_deref()
        .and_then(|b| Uuid::from_slice(b).ok())
        .filter(|u| !u.is_nil())
        .map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{terminal_event_json, worktree_list_terminal_json};

    /// Both projections carry the key, so the apps' two readers (`worktree list`
    /// and the event stream) see one shape; `the_two_terminal_projections_agree_on_every_field`
    /// cannot say so for a key it does not list.
    #[test]
    fn both_projections_carry_the_key() {
        let t = farcooler_protocol::v1::Terminal::default();
        assert!(worktree_list_terminal_json(&t).as_object().unwrap().contains_key("noticeTaskId"));
        assert!(terminal_event_json(&t).as_object().unwrap().contains_key("noticeTaskId"));
    }

    /// The task a pane's own notifications fold into crosses both projections
    /// as the runner decided it, and a runner that decided nothing sends null:
    /// the Mac folds a banner into a task only on this key (ov-112).
    #[test]
    fn a_terminal_names_the_task_its_notices_fold_into_in_both_projections() {
        let task = uuid::Uuid::now_v7();
        let t = farcooler_protocol::v1::Terminal {
            notice_task_id: Some(bytes::Bytes::copy_from_slice(task.as_bytes())),
            ..Default::default()
        };
        assert_eq!(worktree_list_terminal_json(&t)["noticeTaskId"], task.to_string());
        assert_eq!(terminal_event_json(&t)["noticeTaskId"], task.to_string());
        for bytes in [None, Some(bytes::Bytes::new()), Some(bytes::Bytes::from_static(&[0; 16]))] {
            let t = farcooler_protocol::v1::Terminal { notice_task_id: bytes, ..Default::default() };
            assert_eq!(worktree_list_terminal_json(&t)["noticeTaskId"], serde_json::json!(null));
            assert_eq!(terminal_event_json(&t)["noticeTaskId"], serde_json::json!(null));
        }
    }
}

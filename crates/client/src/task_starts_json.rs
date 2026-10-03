//! When a task starts (ov-212) and who works it (ov-213), as `task_json`
//! carries them: `"wait"`, `"waiting_on"` and `"workers"`.
//!
//! In `tasks_json`'s shape and for its reason: the CLI prints these and the
//! phones read them through the FFI, and AgentKit decodes both with one
//! decoder. Every word is a stable one the apps map to their own copy; no
//! sentence a person reads is made here.

use farcooler_protocol::v1 as pb;
use serde_json::json;

use crate::session::uuid_of;

/// A wait's kind, as the proto names it, lowercased.
pub fn wait_kind_word(raw: i32) -> &'static str {
    match pb::TaskWaitKind::try_from(raw) {
        Ok(pb::TaskWaitKind::InLine) => "in_line",
        Ok(pb::TaskWaitKind::Until) => "until",
        Ok(pb::TaskWaitKind::After) => "after",
        Ok(pb::TaskWaitKind::Parked) => "parked",
        Ok(pb::TaskWaitKind::Unspecified) | Err(_) => "unknown",
    }
}

pub fn line_word(raw: i32) -> &'static str {
    match pb::TaskLine::try_from(raw) {
        Ok(pb::TaskLine::Agent) => "agent",
        Ok(pb::TaskLine::Build) => "build",
        Ok(pb::TaskLine::Unspecified) | Err(_) => "unknown",
    }
}

pub fn event_word(raw: i32) -> &'static str {
    match pb::TaskWaitEvent::try_from(raw) {
        Ok(pb::TaskWaitEvent::Release) => "release",
        Ok(pb::TaskWaitEvent::Recurrence) => "recurrence",
        Ok(pb::TaskWaitEvent::ClearBoard) => "clear_board",
        Ok(pb::TaskWaitEvent::Unspecified) | Err(_) => "unknown",
    }
}

pub fn worker_state_word(raw: i32) -> &'static str {
    match pb::TaskWorkerState::try_from(raw) {
        Ok(pb::TaskWorkerState::Running) => "running",
        Ok(pb::TaskWorkerState::Finished) => "finished",
        Ok(pb::TaskWorkerState::Stopped) => "stopped",
        Ok(pb::TaskWorkerState::Unobserved) => "unobserved",
        Ok(pb::TaskWorkerState::Unspecified) | Err(_) => "unknown",
    }
}

/// `{"kind": "in_line", "line": "build", "position": 2, "ahead": ["ov-177"],
/// "since": …}`, with only the keys the kind has.
pub fn wait_json(wait: &pb::TaskWait) -> serde_json::Value {
    let mut out = json!({ "kind": wait_kind_word(wait.kind), "since": crate::tasks_json::said(wait.since) });
    match pb::TaskWaitKind::try_from(wait.kind) {
        Ok(pb::TaskWaitKind::InLine) => {
            out["line"] = json!(line_word(wait.line));
            out["position"] = json!(wait.position);
            out["ahead"] = json!(wait.ahead);
        }
        Ok(pb::TaskWaitKind::Until) => out["until"] = json!(wait.until),
        Ok(pb::TaskWaitKind::After) => out["event"] = json!(event_word(wait.event)),
        _ => {}
    }
    out
}

/// One subagent. Clocks it doesn't have are null, never 1970.
pub fn worker_json(worker: &pb::TaskWorker) -> serde_json::Value {
    json!({
        "id": uuid_of(&worker.id).to_string(),
        "harness": worker.harness,
        "agent_id": worker.agent_id,
        "label": worker.label,
        "model": worker.model,
        "state": worker_state_word(worker.state),
        "started_at": crate::tasks_json::said(worker.started_at),
        "ended_at": crate::tasks_json::said(worker.ended_at),
        "last_activity_at": crate::tasks_json::said(worker.last_activity_at),
        "doing": worker.doing,
        "linked_by_description": worker.linked_by_description,
        "orchestrator_terminal": worker.orchestrator_terminal.as_ref().map(|b| uuid_of(b).to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> bytes::Bytes {
        bytes::Bytes::copy_from_slice(&[n; 16])
    }

    /// Every shape a row's start and workers can take, pinned whole against
    /// `testdata/task_starts.json`: the CLI's `--json` and the phones' FFI
    /// both print this, and AgentKit decodes it.
    #[test]
    fn rows_match_the_golden() {
        let wait = |kind: pb::TaskWaitKind| pb::TaskWait { kind: kind as i32, since: 1_000, ..Default::default() };
        let rows = [
            pb::Task {
                key: "ov-192".into(),
                status: pb::TaskStatus::InProgress as i32,
                wait: Some(pb::TaskWait {
                    line: pb::TaskLine::Build as i32,
                    position: 2,
                    ahead: vec!["ov-177".into()],
                    ..wait(pb::TaskWaitKind::InLine)
                }),
                workers: vec![pb::TaskWorker {
                    id: id(7),
                    harness: "claude".into(),
                    agent_id: "a3fd8fceef581c787".into(),
                    label: "ov-192 Mac polish".into(),
                    model: "opus".into(),
                    started_at: 2_000,
                    ended_at: 9_000,
                    state: pb::TaskWorkerState::Finished as i32,
                    orchestrator_terminal: Some(id(8)),
                    ..Default::default()
                }],
                ..Default::default()
            },
            pb::Task {
                key: "ov-12".into(),
                wait: Some(pb::TaskWait { until: 1_759_654_800_000, ..wait(pb::TaskWaitKind::Until) }),
                waiting_on: vec!["ov-191".into(), "ov-192".into()],
                ..Default::default()
            },
            pb::Task {
                key: "ov-122".into(),
                wait: Some(pb::TaskWait { event: pb::TaskWaitEvent::Release as i32, ..wait(pb::TaskWaitKind::After) }),
                ..Default::default()
            },
            pb::Task { key: "ov-6".into(), wait: Some(wait(pb::TaskWaitKind::Parked)), ..Default::default() },
            pb::Task {
                key: "ov-213".into(),
                workers: vec![pb::TaskWorker {
                    id: id(9),
                    harness: "codex".into(),
                    agent_id: "/root/lane".into(),
                    started_at: 3_000,
                    state: pb::TaskWorkerState::Unobserved as i32,
                    linked_by_description: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            pb::Task { key: "old-runner".into(), ..Default::default() },
        ];
        let got: Vec<serde_json::Value> = rows
            .iter()
            .map(|t| {
                let row = crate::tasks_json::task_json(t, 0);
                json!({ "key": row["key"], "wait": row["wait"], "waiting_on": row["waiting_on"], "workers": row["workers"] })
            })
            .collect();
        let golden: serde_json::Value = serde_json::from_str(include_str!("testdata/task_starts.json")).unwrap();
        assert_eq!(
            json!(got),
            golden,
            "\n{}",
            serde_json::to_string_pretty(&json!(got)).unwrap()
        );
    }

    #[test]
    fn a_word_this_build_does_not_define_is_unknown() {
        assert_eq!(wait_kind_word(99), "unknown");
        assert_eq!(line_word(99), "unknown");
        assert_eq!(event_word(99), "unknown");
        assert_eq!(worker_state_word(99), "unknown");
    }
}

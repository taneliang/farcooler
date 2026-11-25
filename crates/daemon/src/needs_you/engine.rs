//! A stuck engine build as a Needs You item (ov-1077). Its own file to keep `needs_you.rs` inside its size
//! budget. `engine_stuck` says which builds are stuck and why; this says it as a Block.
use farcooler_protocol::v1::NeedsYouKind;

use super::{Signal, Subject};
use crate::engine_stuck::Stuck;

pub(super) fn signal(index: usize, stuck: &Stuck, tasks: &[farcooler_store::models::Task]) -> Signal {
    let workspace = uuid::Uuid::parse_str(&stuck.workspace).ok();
    let subject = tasks.iter().find(|task| Some(task.workspace_id) == workspace && task.key == stuck.item)
        .map_or(Subject::Engine(index), |task| Subject::Task(task.id));
    Signal {
        kind: NeedsYouKind::Blocked,
        id: format!("engine-stuck-{}-{}", stuck.workspace, stuck.item),
        since_ms: stuck.since_ms,
        subject,
        terminal: None,
        question: stuck.question.clone(),
        detail: Some(stuck.item.clone()),
        ask_id: None,
        actions: Vec::new(),
        link: None,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

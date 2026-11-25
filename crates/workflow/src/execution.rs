//! Read-time execution truth. Scheduling intent alone can never yield Running.
use crate::Phase;
use serde::{Deserialize, Serialize};

/// Only the host observes presence; the pure view requires a claim before considering it.
pub enum Dispatch<'a> {
    Pending,
    Claimed { alive: bool, lost: Option<&'a str> },
    Stalled(&'a str),
    Uncertain,
    UncertainBecause(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionView {
    pub phase: Phase,
    pub position: Option<usize>,
    pub holders: Vec<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heavy_lock_wait: Option<String>,
}

pub fn execution_view(dispatch: Dispatch<'_>, position: Option<usize>, holders: Vec<String>) -> ExecutionView {
    let (phase, position, text) = match (dispatch, position) {
        (Dispatch::Pending, None) => (Phase::WaitingSlot, None,
            "Waiting for the runner to observe the external condition. No execution slot is needed.".into()),
        (Dispatch::Pending, Some(position)) => (Phase::WaitingSlot, Some(position),
            format!("Waiting for a slot. Place in line: {position}. Slots held by: {}.",
                if holders.is_empty() { "none".into() } else { holders.join(", ") })),
        (Dispatch::Claimed { lost: Some(reason), .. }, _) => (Phase::Uncertain, None, reason.into()),
        (Dispatch::Claimed { alive: true, lost: None }, _) => (Phase::Running, None, "The runner is executing this step or verifying its result.".into()),
        (Dispatch::Claimed { alive: false, lost: None }, _) => (Phase::Starting, None, "The effect is claimed and its session is starting.".into()),
        (Dispatch::Stalled(reason), _) => (Phase::Stalled, None, reason.into()),
        (Dispatch::UncertainBecause(reason), _) => (Phase::Uncertain, None, reason.into()),
        (Dispatch::Uncertain, _) => (Phase::Uncertain, None, "The effect lost its runtime session without a receipt. Check it before it continues.".into()),
    };
    ExecutionView { phase, position, holders, text, heavy_lock_wait: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unclaimed_work_cannot_be_running() {
        let view = execution_view(Dispatch::Pending, Some(3), vec!["w/one".into()]);
        assert_eq!(view.phase, Phase::WaitingSlot);
        assert_eq!(view.position, Some(3));
        assert!(view.text.contains("w/one"));
        let passive = execution_view(Dispatch::Pending, None, vec![]);
        assert_eq!(passive.position, None);
        assert!(!passive.text.contains("Place in line"));
        assert_ne!(passive.phase, Phase::Running);
        assert_eq!(execution_view(Dispatch::Claimed { alive: false, lost: None }, None, vec![]).phase, Phase::Starting);
        assert_eq!(execution_view(Dispatch::Claimed { alive: true, lost: None }, None, vec![]).phase, Phase::Running);
        assert_eq!(execution_view(Dispatch::Claimed { alive: true, lost: Some("The session was lost without a receipt.") }, None, vec![]).phase, Phase::Uncertain);
    }
}

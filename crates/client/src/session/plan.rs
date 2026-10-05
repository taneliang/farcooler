//! The plan layer's two reads on a session (ov-274), and the news that a plan moved.

use super::*;

impl Session {
    /// One board's plan: its themes, lanes and the queued order (`plan.get`,
    /// ov-274). EXPERIMENTAL. Refused without a round trip on a runner that
    /// doesn't advertise `board_plan`, so a phone never waits on a runner that
    /// can't answer.
    pub async fn plan(&self, workspace: Uuid) -> Result<farcooler_protocol::v1::Plan, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_PLAN, "plan.get")?;
        let payload = request::Payload::PlanGet(farcooler_protocol::v1::PlanGetRequest {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            include_closed: false,
        });
        match self.value("plan.get", None, Some(payload)).await? {
            result::Value::Plan(plan) => Ok(plan),
            other => Err(wrong("plan", &other)),
        }
    }

    /// The owner keeps one ruling (`ruling.set` to confirmed, as `user`,
    /// ov-333). Refused without a round trip on a runner without
    /// `board_ruling_actions`. A phone offers no other move: reversing is a
    /// request to the orchestrator.
    pub async fn keep_ruling(&self, ruling: Uuid) -> Result<(), SessionError> {
        use farcooler_protocol::v1::{BoardRulingState, RulingSet};
        require(self.capabilities(), farcooler_protocol::capability::BOARD_RULING_ACTIONS, "ruling.set")?;
        let payload = request::Payload::RulingSet(RulingSet {
            ruling_id: bytes::Bytes::copy_from_slice(ruling.as_bytes()),
            state: BoardRulingState::Confirmed as i32,
            note: None,
            actor: "user".into(),
            sha: None,
        });
        match self.value("ruling.set", None, Some(payload)).await? {
            result::Value::BoardRuling(_) => Ok(()),
            other => Err(wrong("board_ruling", &other)),
        }
    }

    /// The owner keeps every open ruling on a board (`ruling.keep_all`,
    /// ov-333). Answers how many it kept.
    pub async fn keep_all_rulings(&self, workspace: Uuid) -> Result<usize, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_RULING_ACTIONS, "ruling.keep_all")?;
        let payload = request::Payload::RulingKeepAll(farcooler_protocol::v1::RulingKeepAll {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            actor: "user".into(),
        });
        match self.value("ruling.keep_all", None, Some(payload)).await? {
            result::Value::RulingsKept(k) => Ok(k.rulings.len()),
            other => Err(wrong("rulings_kept", &other)),
        }
    }

    /// One theme's or lane's record, oldest first (`plan.events`, ov-274).
    pub async fn plan_events(
        &self,
        subject: farcooler_protocol::v1::plan_events_request::Subject,
    ) -> Result<farcooler_protocol::v1::PlanEventList, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_PLAN, "plan.events")?;
        let payload = request::Payload::PlanEvents(farcooler_protocol::v1::PlanEventsRequest {
            subject: Some(subject),
            since_ms: 0,
        });
        match self.value("plan.events", None, Some(payload)).await? {
            result::Value::PlanEventList(list) => Ok(list),
            other => Err(wrong("plan_event_list", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    /// A plan written on a board is news of that board (ov-274): not nothing,
    /// which is what it was while no phone read the plan, and not `Fleet`.
    #[test]
    fn a_plan_event_is_news_of_its_board() {
        use farcooler_protocol::v1::event::Payload;
        let board = uuid::Uuid::from_u128(9);
        let news = super::FleetEvent::of(Payload::PlanChanged(farcooler_protocol::v1::PlanChanged {
            workspace_id: bytes::Bytes::copy_from_slice(board.as_bytes()),
            actor: "manager".into(),
        }));
        assert_eq!(news, Some(super::FleetEvent::Plan { workspace: board }));
    }
}

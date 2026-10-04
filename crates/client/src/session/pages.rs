//! Orchestrator pages' two reads on a session (ov-282), and the news that a
//! page moved.

use super::*;

impl Session {
    /// One board's pages, with their documents (`page.list`). EXPERIMENTAL.
    /// Refused without a round trip on a runner that doesn't advertise
    /// `board_pages`, so a phone never waits on a runner that can't answer.
    pub async fn pages(&self, workspace: Uuid) -> Result<farcooler_protocol::v1::BoardPageList, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_PAGES, "page.list")?;
        let payload = request::Payload::PageList(farcooler_protocol::v1::PageListRequest {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            with_docs: true,
        });
        match self.value("page.list", None, Some(payload)).await? {
            result::Value::BoardPageList(list) => Ok(list),
            other => Err(wrong("board_page_list", &other)),
        }
    }

    /// One page by slot (`page.get`).
    pub async fn page(&self, workspace: Uuid, slot: &str) -> Result<farcooler_protocol::v1::BoardPage, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::BOARD_PAGES, "page.get")?;
        let payload = request::Payload::PageGet(farcooler_protocol::v1::PageGetRequest {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            slot: slot.to_string(),
        });
        match self.value("page.get", None, Some(payload)).await? {
            result::Value::BoardPage(page) => Ok(page),
            other => Err(wrong("board_page", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    /// A page written on a board is news of that board and slot.
    #[test]
    fn a_page_event_is_news_of_its_board_and_slot() {
        use farcooler_protocol::v1::event::Payload;
        let board = uuid::Uuid::from_u128(9);
        let news = super::FleetEvent::of(Payload::PagesChanged(farcooler_protocol::v1::PagesChanged {
            workspace_id: bytes::Bytes::copy_from_slice(board.as_bytes()),
            slot: "train".into(),
            revision: 2,
            actor: "manager".into(),
            removed: true,
        }));
        assert_eq!(news, Some(super::FleetEvent::Pages { workspace: board, slot: "train".into(), removed: true }));
    }
}

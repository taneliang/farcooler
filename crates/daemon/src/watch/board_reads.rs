//! The announce for a board's read state (ov-113).

use farcooler_protocol::v1::{Event, event};
use uuid::Uuid;

use super::Watcher;

impl Watcher {
    /// A board's read state moved.
    ///
    /// Carries the whole state, like `announce_stack_changed`: it is a few
    /// hundred bytes, and every part of it only rises, so a client merges it
    /// as it is and a re-read would cost a phone a round trip for nothing.
    pub fn announce_board_reads(&self, reads: farcooler_protocol::v1::BoardReads) {
        let _ = self.events.send(Event {
            event_id: bytes::Bytes::copy_from_slice(Uuid::now_v7().as_bytes()),
            sequence: 0,
            payload: Some(event::Payload::BoardReadsChanged(reads)),
        });
    }
}

//! `terminal agent-subscribe --follow`: one process for the life of a chat
//! view, instead of one per poll.
//!
//! The Mac's chat view used to start a `farcooler terminal agent-subscribe`
//! process every 200 ms for each chat pane on screen, which is five processes a
//! second, and five ssh sessions a second on a remote runner, almost all of
//! them answering "nothing new". Each run is a fork, an exec, a daemon
//! handshake and a JSON decode in the app, so a quiet chat cost about as much as
//! a busy one (ov-229).
//!
//! This keeps one link open and asks over it. A batch is printed as one JSON
//! line only when it holds something: events, or a new epoch. Its shape is the
//! one-shot command's, so the reader parses both the same way. A quiet chat
//! prints nothing, and the app reading it does nothing.
//!
//! The asking still happens on a clock, because the daemon has no push for
//! the agent channel. It's an RPC on a link that's already open, though,
//! which costs microseconds rather than a process.

use std::io::Write;
use std::time::Duration;

use farcooler_protocol::v1::{AgentEventBatch, request, result};
use uuid::Uuid;

use crate::daemon_link::Link;
use crate::{expect_value, id_bytes, req, with};

/// How often the open link asks. The same cadence the app's one-shot poll had.
const ASK_EVERY: Duration = Duration::from_millis(200);

/// Where the next ask starts, and whether this batch is worth a line.
///
/// Pure, so the cursor rules are tested without a daemon. A batch from a new
/// epoch is the whole transcript of a different conversation: it's always
/// printed, even empty, because the reader has to drop what it holds. Within
/// one epoch, an empty batch is news to nobody.
pub(crate) fn advance(from_seq: u64, epoch: u64, first: bool, batch: &AgentEventBatch) -> (u64, u64, bool) {
    let next = batch.events.iter().map(|e| e.seq + 1).max().unwrap_or(from_seq);
    let moved = batch.epoch != epoch;
    let next = if moved { batch.events.iter().map(|e| e.seq + 1).max().unwrap_or(0) } else { next };
    (next, batch.epoch, first || moved || !batch.events.is_empty())
}

/// The JSON line for a batch, for `--follow` and the one-shot command alike.
///
/// `AgentStream.swift`'s `Batch`/`EventFrame` decode with the stock
/// `JSONDecoder`, with no snake_case conversion configured, so these keys
/// must be exactly `events`/`seq`/`payloadJson`/`epoch`. Renaming any of them
/// here does not fail to compile; it makes the Mac app silently drop every
/// batch it receives.
pub(crate) fn line(batch: &AgentEventBatch) -> String {
    serde_json::json!({
        "epoch": batch.epoch,
        "events": batch.events.iter().map(|e| serde_json::json!({
            "seq": e.seq,
            "payloadJson": e.payload_json,
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

/// Ask until the link fails or the reader goes away. Returns the error that
/// ended it; a reader that closed the pipe ends it cleanly.
pub(crate) async fn follow(
    mut link: Link,
    id: Uuid,
    mut from_seq: u64,
    mut epoch: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut first = true;
    let mut out = std::io::stdout().lock();
    let parent = std::os::unix::process::parent_id();
    loop {
        let r = link
            .call(with(
                req("terminal.agent_subscribe"),
                request::Payload::AgentSubscribe(farcooler_protocol::v1::AgentSubscribe {
                    epoch,
                    terminal_id: id_bytes(id),
                    from_seq,
                }),
            ))
            .await?;
        let result::Value::AgentEventBatch(batch) = expect_value(r.value)? else {
            return Err(crate::daemon_link::UNREADABLE.into());
        };
        let (next, now, print) = advance(from_seq, epoch, first, &batch);
        if print && (writeln!(out, "{}", line(&batch)).is_err() || out.flush().is_err()) {
            // Nobody is reading any more: the view went away.
            return Ok(());
        }
        (from_seq, epoch, first) = (next, now, false);
        // A reader that died without closing the pipe, or a quiet chat whose
        // reader died, is only noticed by writing; on a quiet chat nothing is
        // written. Reparented to launchd or init means the reader is gone.
        if orphaned(parent) {
            return Ok(());
        }
        tokio::time::sleep(ASK_EVERY).await;
    }
}

/// Whether the process that started this one has gone.
fn orphaned(parent: u32) -> bool {
    std::os::unix::process::parent_id() != parent
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_protocol::v1::AgentEventFrame;

    fn batch(epoch: u64, seqs: &[u64]) -> AgentEventBatch {
        AgentEventBatch {
            epoch,
            events: seqs.iter().map(|&seq| AgentEventFrame { seq, payload_json: "{}".into(), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_quiet_chat_prints_nothing_after_the_first_line() {
        assert_eq!(advance(0, 0, true, &batch(0, &[])), (0, 0, true));
        assert_eq!(advance(0, 0, false, &batch(0, &[])), (0, 0, false));
        assert_eq!(advance(7, 3, false, &batch(3, &[])), (7, 3, false));
    }

    #[test]
    fn new_events_print_and_move_the_cursor_past_the_last() {
        assert_eq!(advance(5, 1, false, &batch(1, &[5, 6, 7])), (8, 1, true));
    }

    #[test]
    fn a_new_epoch_prints_and_counts_from_its_own_events() {
        assert_eq!(advance(40, 1, false, &batch(2, &[0, 1])), (2, 2, true));
        assert_eq!(advance(40, 1, false, &batch(2, &[])), (0, 2, true));
    }

    #[test]
    fn the_line_has_the_keys_the_mac_decodes() {
        let v: serde_json::Value = serde_json::from_str(&line(&batch(4, &[9]))).unwrap();
        assert_eq!(v["epoch"], 4);
        assert_eq!(v["events"][0]["seq"], 9);
        assert_eq!(v["events"][0]["payloadJson"], "{}");
    }
}

//! A replay that fits in one envelope (ov-381).
//!
//! A reader attaching from nothing is handed the whole window as one reply,
//! and a control envelope is capped at `MAX_CONTROL_ENVELOPE_BYTES` (1 MiB). A
//! window past that failed on every attempt, and a new view always starts from
//! nothing, so a chat that had grown that big never loaded again. This hands
//! back the newest events that fit, behind a `Gap` saying older history was
//! left out, and the reader fills from there and follows.

use farcooler_agent::event::{AgentEvent, AgentGapReason, Sequenced};
use farcooler_agent::link::encode_line;

/// What one replay may weigh, serialized: three quarters of the 1 MiB
/// envelope, which leaves room for the frames' own fields and the rest of the
/// reply.
pub(super) const REPLAY_BUDGET_BYTES: usize = 768 * 1024;

/// What a frame adds to its payload: the sequence number, two field tags, a
/// length. Generous, so the sum errs toward fitting.
const FRAME_OVERHEAD_BYTES: usize = 24;

/// The newest of `events` whose serialized size fits the replay budget, opened by a
/// `Gap` when anything older was left out.
///
/// Measured from the newest backwards and stopped at the first that does not
/// fit, so a follower asking for a handful of events pays for a handful. The
/// gap takes the number just below the first event kept: a reader whose cursor
/// is at or below it reads the gap first, and one past it has nothing missing.
pub(super) fn newest_that_fit(events: Vec<Sequenced>) -> Vec<Sequenced> {
    fit(events, REPLAY_BUDGET_BYTES)
}

/// `newest_that_fit` against a budget of the caller's choosing.
fn fit(mut events: Vec<Sequenced>, budget: usize) -> Vec<Sequenced> {
    // Room for the gap itself.
    let mut left = budget.saturating_sub(128);
    let mut keep = events.len();
    for (index, item) in events.iter().enumerate().rev() {
        let weight = encode_line(&item.event).map_or(0, |line| line.len()) + FRAME_OVERHEAD_BYTES;
        if weight > left {
            break;
        }
        left -= weight;
        keep = index;
    }
    if keep == 0 {
        return events;
    }
    let mut fitted = events.split_off(keep);
    let already_marked = matches!(
        fitted.first().map(|first| &first.event),
        Some(AgentEvent::Gap { reason: AgentGapReason::RingTrimmed })
    );
    if !already_marked {
        let seq = fitted.first().map_or(0, |first| first.seq.saturating_sub(1));
        fitted.insert(0, Sequenced { seq, event: AgentEvent::Gap { reason: AgentGapReason::RingTrimmed } });
    }
    fitted
}

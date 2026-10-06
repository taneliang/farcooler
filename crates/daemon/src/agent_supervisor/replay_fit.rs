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

/// What stands in for the part of an event that was cut to make it fit.
const CUT_MARKER: &str = "\n\n[This part was too long to load here.]";

/// `newest_that_fit` against a budget of the caller's choosing.
fn fit(mut events: Vec<Sequenced>, budget: usize) -> Vec<Sequenced> {
    // Room for the gap itself.
    let room = budget.saturating_sub(128);
    let newest = events.last().map(|item| item.seq);
    // The newest event is the one a reader most needs. When it alone is past
    // the budget, cut its text rather than hand back nothing at its number.
    if let Some(last) = events.last_mut()
        && weight(&last.event) > room
        && let Some(cut) = shortened(&last.event, room)
    {
        last.event = cut;
    }
    let mut left = room;
    let mut keep = events.len();
    for (index, item) in events.iter().enumerate().rev() {
        let weight = weight(&item.event);
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
        // With nothing kept, the gap takes the newest number, so a cursor
        // moves past it rather than sitting where it was (or rewinding).
        let seq = fitted.first().map_or(newest.unwrap_or(0), |first| first.seq.saturating_sub(1));
        fitted.insert(0, Sequenced { seq, event: AgentEvent::Gap { reason: AgentGapReason::RingTrimmed } });
    }
    fitted
}

/// What one event adds to a reply.
fn weight(event: &AgentEvent) -> usize {
    encode_line(event).map_or(0, |line| line.len()) + FRAME_OVERHEAD_BYTES
}

/// The text of an event that can carry a long one.
fn text_of(event: &mut AgentEvent) -> Option<&mut String> {
    match event {
        AgentEvent::Message { text, .. } => Some(text),
        AgentEvent::ToolUpdate { content: Some(text), .. } => Some(text),
        _ => None,
    }
}

/// `event` with its long text halved until it weighs no more than `room`, or
/// `None` when it has no text to cut or the cut cannot reach it.
///
/// The text is taken out once, so each round builds one string and the event is
/// cloned a single time, however many rounds it takes.
fn shortened(event: &AgentEvent, room: usize) -> Option<AgentEvent> {
    let mut event = event.clone();
    let original = std::mem::take(text_of(&mut event)?);
    let mut keep = original.len();
    while keep > 0 {
        keep /= 2;
        let mut end = keep;
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        *text_of(&mut event)? = format!("{}{CUT_MARKER}", &original[..end]);
        if weight(&event) <= room {
            return Some(event);
        }
    }
    None
}

//! The words a row carries for its apps (ov-1054, ov-1062): said once here, so no app asks for them.

use super::rows::{RowKind, TurnOrigin};

/// A subagent's type as words: `general-purpose` reads "General purpose".
pub fn agent_type(raw: &str) -> String {
    let words = raw.replace(['-', '_'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(|| "Agent".into(), |first| first.to_uppercase().chain(chars).collect())
}

/// "0:04", "1:12", "2:03:09": the format a running timer counts in, so a
/// finished time reads like the running one did.
pub fn short(ms: i64) -> String {
    let seconds = (ms / 1000).max(0);
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// A turn nobody typed: a background task finishing, or claude waking itself.
/// Drawn as a notice, never as the person's message.
pub fn is_notice(origin: TurnOrigin) -> bool {
    matches!(origin, TurnOrigin::Notification | TurnOrigin::System)
}

fn span(started: Option<i64>, ended: Option<i64>) -> Option<String> {
    Some(short(ended? - started?))
}

/// Fills the words a row's kind carries, from the fields they say.
pub(super) fn fill(kind: &mut RowKind) {
    match kind {
        RowKind::Turn(t) => {
            t.notice = is_notice(t.origin);
            t.took = t.duration_ms.map(short);
        }
        RowKind::Subagent(s) => {
            s.type_words = agent_type(&s.agent_type);
            s.ran = span(s.started_ms, s.ended_ms);
        }
        RowKind::Tool(t) => t.ran = span(t.started_ms, t.ended_ms),
        RowKind::Thinking(t) => t.ran = span(t.started_ms, t.ended_ms),
        _ => {}
    }
}

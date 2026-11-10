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

/// Writes the words an older daemon's row doesn't carry, from the fields it does, so a client reads one shape
/// whatever the daemon's age (ov-1062). A field the row already has stays as it is.
pub fn backfill(row: &mut serde_json::Value) {
    let Some(kind) = row.get_mut("kind").and_then(|k| k.as_object_mut()) else { return };
    if let Some(turn) = kind.get_mut("Turn").and_then(|t| t.as_object_mut()) {
        let notice = matches!(turn.get("origin").and_then(|o| o.as_str()), Some("Notification" | "System"));
        if notice && !turn.contains_key("notice") {
            turn.insert("notice".into(), true.into());
        }
        let took = turn.get("duration_ms").and_then(|d| d.as_i64()).map(short);
        if let (Some(took), false) = (took, turn.contains_key("took")) {
            turn.insert("took".into(), took.into());
        }
    }
    if let Some(sub) = kind.get_mut("Subagent").and_then(|s| s.as_object_mut()) {
        let words = sub.get("agent_type").and_then(|t| t.as_str()).map(agent_type);
        if let (Some(words), false) = (words, sub.contains_key("type_words")) {
            sub.insert("type_words".into(), words.into());
        }
    }
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
        }
        _ => {}
    }
}

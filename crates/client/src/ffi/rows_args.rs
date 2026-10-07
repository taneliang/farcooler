//! A terminal's agent rows, as an app passes and reads them (ov-366):
//! `agent.rows` and `agent.rows_follow`. EXPERIMENTAL until a client reads
//! them (ov-372).
//!
//! Each row arrives as the object `projector::Row` serializes to, not as a
//! string of JSON, so an app decodes rows in the same pass as the page.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{Session, SessionError};

/// One of the two row methods. `method` is one `dispatch` matched.
///
/// - `agent.rows {terminal, before?, limit?}` answers
///   `{epoch, rev, moreBefore, rows: [row]}`.
/// - `agent.rows_follow {terminal, epoch, afterRev, waitMs?}` answers
///   `{epoch, rev, reset, changes: [{kind, id, rev, row?}]}`, `kind` one of
///   `insert`, `update` and `remove` (which carries no row). Without
///   `waitMs` the runner holds it `DEFAULT_WAIT_MS`: an app that forgets it
///   must not poll in a tight loop.
pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    let terminal = args
        .get("terminal")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Uuid>().ok())
        .ok_or_else(|| SessionError::Protocol(format!("{method} needs a terminal")))?;
    let number = |key: &str| args.get(key).and_then(Value::as_u64);
    if method == "agent.rows" {
        let limit = number("limit").unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        let page = session.agent_rows(terminal, number("before"), limit).await?;
        return Ok(page_of(&page));
    }
    let wait = wait_of(args);
    let follow = session.agent_rows_follow(terminal, number("epoch").unwrap_or(0), number("afterRev").unwrap_or(0), wait).await?;
    Ok(changes_of(&follow))
}

/// How long a follow waits when the app doesn't say: inside the 30 s a call
/// has (`deadlines.rs`), with room for the round trip.
pub(super) const DEFAULT_WAIT_MS: u32 = 20_000;

fn wait_of(args: &Value) -> u32 {
    args.get("waitMs").and_then(Value::as_u64).map_or(DEFAULT_WAIT_MS, |ms| ms.min(u64::from(u32::MAX)) as u32)
}

fn row_of(row: &farcooler_protocol::v1::AgentRow) -> Value {
    serde_json::from_str(&row.row_json).unwrap_or_else(|_| json!({ "id": row.id, "ord": row.ord, "rev": row.rev }))
}

/// A page as an app reads it: `{epoch, rev, moreBefore, rows: [row]}`.
pub(super) fn page_of(page: &farcooler_protocol::v1::AgentRowPage) -> Value {
    json!({
        "epoch": page.epoch,
        "rev": page.rev,
        "moreBefore": page.more_before,
        "rows": page.rows.iter().map(row_of).collect::<Vec<_>>(),
    })
}

pub(super) fn changes_of(follow: &farcooler_protocol::v1::AgentRowChanges) -> Value {
    use farcooler_protocol::v1::AgentRowChangeKind as Kind;
    let changes: Vec<Value> = follow
        .changes
        .iter()
        .map(|change| {
            let kind = match Kind::try_from(change.kind) {
                Ok(Kind::Insert) => "insert",
                Ok(Kind::Remove) => "remove",
                _ => "update",
            };
            let mut out = json!({ "kind": kind, "id": change.id, "rev": change.rev });
            if let Some(row) = &change.row {
                out["row"] = row_of(row);
            }
            out
        })
        .collect();
    json!({ "epoch": follow.epoch, "rev": follow.rev, "reset": follow.reset, "changes": changes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_follow_with_no_wait_named_waits_twenty_seconds_and_zero_is_zero() {
        assert_eq!(wait_of(&json!({})), DEFAULT_WAIT_MS);
        assert_eq!(wait_of(&json!({ "waitMs": 0 })), 0);
        assert_eq!(wait_of(&json!({ "waitMs": 5 })), 5);
    }
}

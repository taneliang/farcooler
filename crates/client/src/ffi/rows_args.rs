//! A terminal's agent rows, as an app passes and reads them (ov-366):
//! `agent.rows` and `agent.rows_follow`, and a prompt's image (`agent.image`,
//! ov-454).
//!
//! Each row arrives as the object `projector::Row` serializes to, not as a
//! string of JSON, so an app decodes rows in the same pass as the page.

use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{Session, SessionError};

/// One of the row methods. `method` is one `dispatch` matched.
///
/// - Either takes `agent`, a subagent's `agentId`, for that agent's own rows
///   (ov-453), where the runner offers `subagent_rows`.
/// - `agent.rows {terminal, before?, limit?}` answers
///   `{epoch, rev, moreBefore, rows: [row]}`.
/// - `agent.rows_follow {terminal, epoch, afterRev, waitMs?}` answers
///   `{epoch, rev, reset, changes: [{kind, id, rev, row?}]}`, `kind` one of
///   `insert`, `update` and `remove` (which carries no row). Without
///   `waitMs` the runner holds it `DEFAULT_WAIT_MS`: an app that forgets it
///   must not poll in a tight loop.
/// - `agent.image {terminal, row, index}` answers `{mime, base64}`: one image
///   a turn row's prompt carried, whole (ov-454).
pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    let terminal = args
        .get("terminal")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Uuid>().ok())
        .ok_or_else(|| SessionError::Protocol(format!("{method} needs a terminal")))?;
    let number = |key: &str| args.get(key).and_then(Value::as_u64);
    if method == "agent.image" {
        let row = args.get("row").and_then(Value::as_str).ok_or_else(|| SessionError::Protocol(format!("{method} needs a row")))?;
        let index = number("index").unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        let (mime, bytes) = session.agent_image(terminal, row, index).await?;
        return Ok(json!({ "mime": mime, "base64": farcooler_core::base64::encode(&bytes) }));
    }
    let agent = agent_of(args);
    if method == "agent.rows" {
        let limit = number("limit").unwrap_or(0).min(u64::from(u32::MAX)) as u32;
        let page = session.agent_rows(terminal, number("before"), limit, agent).await?;
        return Ok(page_of(&page));
    }
    let wait = wait_of(args);
    let follow = session.agent_rows_follow(terminal, number("epoch").unwrap_or(0), number("afterRev").unwrap_or(0), wait, agent).await?;
    Ok(changes_of(&follow))
}

/// How long a follow waits when the app doesn't say: inside the 30 s a call
/// has (`deadlines.rs`), with room for the round trip.
pub(super) const DEFAULT_WAIT_MS: u32 = 20_000;

/// A subagent's own rows (ov-453): `agent`, its `agentId`. Absent or empty
/// is the pane's.
fn agent_of(args: &Value) -> Option<&str> {
    args.get("agent").and_then(Value::as_str).filter(|a| !a.is_empty())
}

fn wait_of(args: &Value) -> u32 {
    args.get("waitMs").and_then(Value::as_u64).map_or(DEFAULT_WAIT_MS, |ms| ms.min(u64::from(u32::MAX)) as u32)
}

pub(super) use crate::rows_json::{changes_json as changes_of, rows_page_json as page_of};

#[cfg(test)]
#[path = "subagent_rows_tests.rs"]
mod subagent_rows_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_named_asks_for_its_rows_and_none_or_empty_for_the_panes() {
        assert_eq!(agent_of(&json!({ "agent": "a1" })), Some("a1"));
        assert_eq!(agent_of(&json!({ "agent": "" })), None);
        assert_eq!(agent_of(&json!({})), None);
    }

    #[test]
    fn a_follow_with_no_wait_named_waits_twenty_seconds_and_zero_is_zero() {
        assert_eq!(wait_of(&json!({})), DEFAULT_WAIT_MS);
        assert_eq!(wait_of(&json!({ "waitMs": 0 })), 0);
        assert_eq!(wait_of(&json!({ "waitMs": 5 })), 5);
    }
}

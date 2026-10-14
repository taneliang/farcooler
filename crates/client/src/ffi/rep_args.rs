//! A member's Talk, as an app passes it (ov-526): `rep.start`.

use serde_json::{Value, json};

use crate::session::{Session, SessionError, uuid_of};

/// `{workspace, handle}`: open `handle`'s rep in `workspace`. The app names
/// the handle it read as `member.list`'s `you`; the runner refuses anyone
/// else's, so a wrong handle is a refusal there, not a rep for someone else.
pub(super) async fn start(session: &Session, args: &Value) -> Result<Value, SessionError> {
    const METHOD: &str = "rep.start";
    let workspace = args
        .get("workspace")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<uuid::Uuid>().ok())
        .ok_or_else(|| SessionError::Protocol(format!("{METHOD} needs a workspace")))?;
    let handle = args.get("handle").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if handle.is_empty() {
        return Err(SessionError::Protocol(format!("{METHOD} needs a handle")));
    }
    let started = session.start_rep(workspace, handle).await?;
    Ok(json!({
        "id": uuid_of(&started.terminal_id).to_string(),
        "desk": started.desk_task_key,
        "claude_account": started.claude_account,
    }))
}

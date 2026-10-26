//! A member's Talk, as an app passes it (ov-526): `rep.start`; a runner
//! admin's switch for members' own logins (ov-652): `member.set_own_login`;
//! and the owner's role picker (ov-784): `member.set_role`.

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

/// `{harness, on}`: whether members sign in to `harness` (`claude`, `codex`,
/// `cursor`) with their own account on this runner. Answers `member.list`'s
/// JSON as it is now. The runner refuses anyone but its admins.
pub(super) async fn set_own_login(session: &Session, args: &Value) -> Result<Value, SessionError> {
    const METHOD: &str = "member.set_own_login";
    let harness = args.get("harness").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let on = args.get("on").and_then(Value::as_bool).ok_or_else(|| SessionError::Protocol(format!("{METHOD} needs on")))?;
    Ok(crate::members_json::members_json(&session.set_own_login(harness, on).await?))
}

/// `{handle, role}`: give `handle` `role` (`member` or `maintainer`).
/// Answers `member.list`'s JSON as it is now. The runner refuses anyone but
/// its owner, a runner admin included.
pub(super) async fn set_role(session: &Session, args: &Value) -> Result<Value, SessionError> {
    const METHOD: &str = "member.set_role";
    let field = |name: &str| args.get(name).and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let (handle, role) = (field("handle"), field("role"));
    if handle.is_empty() || role.is_empty() {
        return Err(SessionError::Protocol(format!("{METHOD} needs a handle and a role")));
    }
    Ok(crate::members_json::members_json(&session.set_role(handle, role).await?))
}

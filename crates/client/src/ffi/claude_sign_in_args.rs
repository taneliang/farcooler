//! A member signing in to Claude on a runner, as an app passes it (ov-584):
//! `claude.sign_in {handle}` → `{session, url, expires_at}`, then
//! `claude.sign_in_finish {session, code}`, `claude.sign_in_cancel
//! {session}` or `claude.sign_out {handle}` → the login as `member.list`
//! shows it (`{state, email}`).

use serde_json::{Value, json};

use crate::members_json::claude_login_json;
use crate::session::{Session, SessionError, uuid_of};

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

fn needs(method: &str, key: &str) -> SessionError {
    SessionError::Protocol(format!("{method} needs a {key}"))
}

pub(super) async fn call(session: &Session, method: &str, args: &Value) -> Result<Value, SessionError> {
    let handle = || Some(text(args, "handle")).filter(|h| !h.is_empty()).ok_or_else(|| needs(method, "handle"));
    let sign_in = || text(args, "session").parse::<uuid::Uuid>().map_err(|_| needs(method, "session"));
    match method {
        "claude.sign_in" => {
            let started = session.claude_sign_in(handle()?).await?;
            Ok(json!({
                "session": uuid_of(&started.session_id).to_string(),
                "url": started.url,
                "expires_at": started.expires_at,
            }))
        }
        // The code goes as given: the runner trims it and says when it isn't whole.
        "claude.sign_in_finish" => {
            let code = args.get("code").and_then(Value::as_str).unwrap_or_default();
            Ok(claude_login_json(&session.claude_sign_in_finish(sign_in()?, code).await?))
        }
        "claude.sign_in_cancel" => Ok(claude_login_json(&session.claude_sign_in_cancel(sign_in()?).await?)),
        "claude.sign_out" => Ok(claude_login_json(&session.claude_sign_out(handle()?).await?)),
        _ => Err(SessionError::Protocol(format!("{method} isn't a sign-in call"))),
    }
}

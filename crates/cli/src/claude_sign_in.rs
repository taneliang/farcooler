//! `farcooler member claude-sign-in` and its three companions (ov-584): the
//! Mac app's way to sign a member in to Claude on a runner, since it reaches a
//! runner through this CLI. The rules are the runner's
//! (`farcooler_daemon::rpc_claude_sign_in`); this only asks and says.
//!
//! The answers are the ones the phones' bridge gives (`claude.sign_in`,
//! `claude.sign_in_finish`, `claude.sign_in_cancel`, `claude.sign_out`), so
//! one decoder reads both. The pasted code is read from standard input and
//! never an argument, so it is on no command line.

use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{expect_value, req, tasks, with};

const NO_SIGN_IN: &str = "this runner's Far Cooler can't sign a member in to Claude yet. update it and try again";

type Said = Result<String, Box<dyn std::error::Error>>;

/// One sign-in call, as the commands name them.
pub(crate) enum Call<'a> {
    Start { handle: &'a str },
    Finish { session: &'a str, code: &'a str },
    Cancel { session: &'a str },
    SignOut { handle: &'a str },
}

fn session_id(session: &str) -> Result<bytes::Bytes, Box<dyn std::error::Error>> {
    let id = session.trim().parse::<uuid::Uuid>().map_err(|_| "a sign-in's session is the id `claude-sign-in` printed")?;
    Ok(bytes::Bytes::copy_from_slice(id.as_bytes()))
}

/// A refused call. A finish or a cancel the runner finds no sign-in for is
/// over (it ran out, or was ended), which `people::refused` would call a
/// missing member; a start or a sign-out that finds nobody is that.
fn refused(e: farcooler_transport::ClientError, session_call: bool) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && session_call
        && farcooler_core::error::word_for(*code) == "not-found"
    {
        return Box::new(tasks::Refused::naming("that sign-in is over. start it again".to_string(), *code, what.clone()));
    }
    crate::people::refused(e)
}

/// `call` on `link`, answering what to print: the JSON the bridge answers,
/// or a sentence.
pub(crate) async fn sign_in_on<L: tasks::DispatchLink>(link: &mut L, call: Call<'_>, json: bool) -> Said {
    if !link.capabilities().iter().any(|c| c == capability::CLAUDE_SIGN_IN) {
        return Err(NO_SIGN_IN.into());
    }
    let (method, payload) = match call {
        Call::Start { handle } => {
            ("claude.sign_in", request::Payload::ClaudeSignInStart(pb::ClaudeSignInStart { handle: handle.trim().to_string() }))
        }
        Call::Finish { session, code } => (
            "claude.sign_in_finish",
            request::Payload::ClaudeSignInFinish(pb::ClaudeSignInFinish { session_id: session_id(session)?, code: code.to_string() }),
        ),
        Call::Cancel { session } => {
            ("claude.sign_in_cancel", request::Payload::ClaudeSignInCancel(pb::ClaudeSignInCancel { session_id: session_id(session)? }))
        }
        Call::SignOut { handle } => {
            ("claude.sign_out", request::Payload::ClaudeSignOut(pb::ClaudeSignOut { handle: handle.trim().to_string() }))
        }
    };
    let mut r = with(req(method), payload);
    r.required_capabilities.push(capability::CLAUDE_SIGN_IN.to_string());
    let session_call = matches!(method, "claude.sign_in_finish" | "claude.sign_in_cancel");
    let answer = link.call(r).await.map_err(|e| refused(e, session_call))?;
    match expect_value(answer.value)? {
        result::Value::ClaudeSignIn(started) => {
            let session = uuid::Uuid::from_slice(&started.session_id).map_err(|_| crate::daemon_link::UNREADABLE)?;
            if !json {
                // The page, alone on its line, for a person to open.
                return Ok(started.url);
            }
            Ok(serde_json::json!({ "session": session.to_string(), "url": started.url, "expires_at": started.expires_at }).to_string())
        }
        result::Value::MemberClaudeLogin(login) => {
            if json {
                return Ok(farcooler_client::members_json::claude_login_json(&login).to_string());
            }
            Ok(match pb::ClaudeLoginState::try_from(login.state) {
                Ok(pb::ClaudeLoginState::SignedIn) if login.email.is_empty() => "signed in to Claude".to_string(),
                Ok(pb::ClaudeLoginState::SignedIn) => format!("signed in to Claude as {}", login.email),
                _ => "not signed in to Claude".to_string(),
            })
        }
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

#[cfg(test)]
#[path = "claude_sign_in_tests.rs"]
mod tests;

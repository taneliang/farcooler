//! The people on a runner, on a session (ov-508): `member.list`, for the
//! phones' runner settings.

use super::*;

impl Session {
    /// Everyone on this runner besides its owner, their reps, the Claude
    /// account they all work on (empty below Control), and who's asking.
    /// Refused without a round trip on a runner without `representatives`.
    pub async fn members(&self) -> Result<farcooler_protocol::v1::MemberList, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::REPRESENTATIVES, "member.list")?;
        let required = vec![farcooler_protocol::capability::REPRESENTATIVES.to_string()];
        match self.value_requiring("member.list", None, None, required).await? {
            result::Value::MemberList(list) => Ok(list),
            other => Err(wrong("member_list", &other)),
        }
    }

    /// Open `handle`'s rep in `workspace` (`rep.start`): what a member's Talk
    /// does when the rep isn't running. The runner decides who may: a member
    /// starts their own, and anyone else's is refused `scope_denied`. A rep
    /// that is running is refused `rep_running`. Refused without a round trip
    /// on a runner without `representatives`.
    pub async fn start_rep(
        &self,
        workspace: Uuid,
        handle: &str,
    ) -> Result<farcooler_protocol::v1::RepStarted, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::REPRESENTATIVES, "rep.start")?;
        let required = vec![farcooler_protocol::capability::REPRESENTATIVES.to_string()];
        let payload = request::Payload::RepStart(farcooler_protocol::v1::RepStart {
            workspace_id: bytes::Bytes::copy_from_slice(workspace.as_bytes()),
            handle: handle.to_string(),
            harness: String::new(),
        });
        match self.value_requiring("rep.start", None, Some(payload), required).await? {
            result::Value::RepStarted(started) => Ok(started),
            other => Err(wrong("rep_started", &other)),
        }
    }

    /// Start signing `handle` in to Claude on this runner (`claude.sign_in`,
    /// ov-584): the page to open, for this device alone. Only the member
    /// themself, from their own device. Refused without a round trip on a
    /// runner without `claude_sign_in`.
    pub async fn claude_sign_in(&self, handle: &str) -> Result<farcooler_protocol::v1::ClaudeSignIn, SessionError> {
        let payload = request::Payload::ClaudeSignInStart(farcooler_protocol::v1::ClaudeSignInStart { handle: handle.to_string() });
        match self.sign_in_call("claude.sign_in", payload).await? {
            result::Value::ClaudeSignIn(started) => Ok(started),
            other => Err(wrong("claude_sign_in", &other)),
        }
    }

    /// The code the sign-in page showed, for the sign-in `session`.
    pub async fn claude_sign_in_finish(&self, session: Uuid, code: &str) -> Result<farcooler_protocol::v1::MemberClaudeLogin, SessionError> {
        let payload = request::Payload::ClaudeSignInFinish(farcooler_protocol::v1::ClaudeSignInFinish {
            session_id: bytes::Bytes::copy_from_slice(session.as_bytes()),
            code: code.to_string(),
        });
        self.login_answer("claude.sign_in_finish", payload).await
    }

    /// End the sign-in `session`.
    pub async fn claude_sign_in_cancel(&self, session: Uuid) -> Result<farcooler_protocol::v1::MemberClaudeLogin, SessionError> {
        let payload = request::Payload::ClaudeSignInCancel(farcooler_protocol::v1::ClaudeSignInCancel {
            session_id: bytes::Bytes::copy_from_slice(session.as_bytes()),
        });
        self.login_answer("claude.sign_in_cancel", payload).await
    }

    /// Sign `handle` out of Claude on this runner (`claude.sign_out`).
    pub async fn claude_sign_out(&self, handle: &str) -> Result<farcooler_protocol::v1::MemberClaudeLogin, SessionError> {
        let payload = request::Payload::ClaudeSignOut(farcooler_protocol::v1::ClaudeSignOut { handle: handle.to_string() });
        self.login_answer("claude.sign_out", payload).await
    }

    async fn login_answer(&self, method: &str, payload: request::Payload) -> Result<farcooler_protocol::v1::MemberClaudeLogin, SessionError> {
        match self.sign_in_call(method, payload).await? {
            result::Value::MemberClaudeLogin(login) => Ok(login),
            other => Err(wrong("member_claude_login", &other)),
        }
    }

    async fn sign_in_call(&self, method: &str, payload: request::Payload) -> Result<result::Value, SessionError> {
        require(self.capabilities(), farcooler_protocol::capability::CLAUDE_SIGN_IN, method)?;
        let required = vec![farcooler_protocol::capability::CLAUDE_SIGN_IN.to_string()];
        self.value_requiring(method, None, Some(payload), required).await
    }
}

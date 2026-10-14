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
}

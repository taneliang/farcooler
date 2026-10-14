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
}

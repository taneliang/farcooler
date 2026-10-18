//! `farcooler member claude-sign-in` and its companions (ov-584), against a
//! fake runner that records what it was sent.

use super::*;
use crate::people::{MemberCmd, member_on};

const SESSION: uuid::Uuid = uuid::Uuid::from_u128(0x5e55_1011);

struct Runner {
    sent: Vec<pb::Request>,
    sign_in: bool,
    refuse: Option<(pb::ErrorCode, &'static str)>,
}

impl tasks::DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        let mut have = vec![capability::REPRESENTATIVES.to_string()];
        if self.sign_in {
            have.push(capability::CLAUDE_SIGN_IN.to_string());
        }
        have
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
        if let Some((code, what)) = self.refuse {
            return Err(farcooler_transport::ClientError::Daemon { code: code as i32, message: String::new(), what: what.into(), retryable: false });
        }
        let login = |state: pb::ClaudeLoginState, email: &str| {
            result::Value::MemberClaudeLogin(pb::MemberClaudeLogin { handle: "bob".into(), state: state as i32, email: email.into() })
        };
        let value = match req.method.as_str() {
            "claude.sign_in" => result::Value::ClaudeSignIn(pb::ClaudeSignIn {
                session_id: bytes::Bytes::copy_from_slice(SESSION.as_bytes()),
                url: "https://claude.com/cai/oauth/authorize?code=true".into(),
                expires_at: 1_760_000_000_000,
            }),
            "claude.sign_in_finish" => login(pb::ClaudeLoginState::SignedIn, "bob@claude.example"),
            "claude.sign_in_cancel" | "claude.sign_out" => login(pb::ClaudeLoginState::None, ""),
            other => panic!("sent {other}, which this fake doesn't expect"),
        };
        self.sent.push(req);
        Ok(pb::Result { value: Some(value) })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn runner() -> Runner {
    Runner { sent: Vec::new(), sign_in: true, refuse: None }
}

fn cmd(line: &str) -> MemberCmd {
    use clap::Parser;
    match crate::Cli::try_parse_from(line.split_whitespace()).unwrap().command {
        crate::Command::Member(cmd) => cmd,
        _ => panic!("not a member command"),
    }
}

/// A start asks for the member's own sign-in, names the capability, and
/// answers the bridge's JSON: the session as text, the page, the expiry.
#[tokio::test]
async fn starting_answers_the_page_and_the_session() {
    let mut link = runner();
    let out = member_on(&mut link, cmd("farcooler member claude-sign-in bob"), true).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["session"], SESSION.to_string());
    assert_eq!(json["url"], "https://claude.com/cai/oauth/authorize?code=true");
    assert_eq!(json["expires_at"], 1_760_000_000_000i64);
    let req = &link.sent[0];
    assert_eq!(req.method, "claude.sign_in");
    assert!(req.required_capabilities.contains(&capability::CLAUDE_SIGN_IN.to_string()));
    let Some(request::Payload::ClaudeSignInStart(p)) = &req.payload else { panic!("no payload") };
    assert_eq!(p.handle, "bob");
    // For a person at a terminal: the page, alone.
    let said = member_on(&mut link, cmd("farcooler member claude-sign-in bob"), false).await.unwrap();
    assert_eq!(said, "https://claude.com/cai/oauth/authorize?code=true");
}

/// A finish sends the session's id bytes and the code as given, and answers
/// the login in `member.list`'s shape.
#[tokio::test]
async fn finishing_sends_the_code_to_the_session() {
    let mut link = runner();
    let call = Call::Finish { session: &SESSION.to_string(), code: "abc#def\n" };
    let out = sign_in_on(&mut link, call, true).await.unwrap();
    assert_eq!(out, r#"{"email":"bob@claude.example","state":"signed_in"}"#);
    let Some(request::Payload::ClaudeSignInFinish(p)) = &link.sent[0].payload else { panic!("no payload") };
    assert_eq!(p.session_id.as_ref(), SESSION.as_bytes());
    assert_eq!(p.code, "abc#def\n");
    assert_eq!(link.sent[0].method, "claude.sign_in_finish");
    let said = sign_in_on(&mut link, Call::Finish { session: &SESSION.to_string(), code: "a#b" }, false).await.unwrap();
    assert_eq!(said, "signed in to Claude as bob@claude.example");
}

/// Cancel and sign-out each send their own call, and answer not signed in.
#[tokio::test]
async fn cancelling_and_signing_out() {
    let mut link = runner();
    let out = member_on(&mut link, cmd(&format!("farcooler member claude-sign-in-cancel {SESSION}")), true).await.unwrap();
    assert_eq!(out, r#"{"email":"","state":"none"}"#);
    let said = member_on(&mut link, cmd("farcooler member claude-sign-out bob"), false).await.unwrap();
    assert_eq!(said, "not signed in to Claude");
    let methods: Vec<&str> = link.sent.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["claude.sign_in_cancel", "claude.sign_out"]);
    let Some(request::Payload::ClaudeSignOut(p)) = &link.sent[1].payload else { panic!("no payload") };
    assert_eq!(p.handle, "bob");
}

/// A runner from before sign-in is refused without a round trip, and a
/// session that isn't an id is refused before anything is sent.
#[tokio::test]
async fn an_old_runner_and_a_bad_session_send_nothing() {
    let mut old = Runner { sign_in: false, ..runner() };
    let said = member_on(&mut old, cmd("farcooler member claude-sign-in bob"), false).await.expect_err("refused").to_string();
    assert!(said.contains("can't sign a member in to Claude yet"), "{said}");
    assert!(old.sent.is_empty());
    let mut link = runner();
    let bad = sign_in_on(&mut link, Call::Cancel { session: "not-an-id" }, true).await.expect_err("refused").to_string();
    assert!(bad.contains("session"), "{bad}");
    assert!(link.sent.is_empty());
}

/// A refusal keeps the runner's word and the case it named under `--json`,
/// which is what the Mac switches on, and its sentence is this CLI's.
#[tokio::test]
async fn a_refusal_keeps_its_word_and_case() {
    for (code, what, word, said) in [
        (pb::ErrorCode::ResourceConflict, "code_refused", "resource-conflict", "the sign-in didn't accept that code"),
        (pb::ErrorCode::InvalidArgument, "sign_in_code", "invalid-argument", "that code isn't complete"),
        (pb::ErrorCode::ResourceConflict, "sign_in_too_old", "resource-conflict", "too old to sign in from here"),
        (pb::ErrorCode::NotFound, "", "not-found", "nobody on this runner by that handle"),
    ] {
        let mut link = Runner { refuse: Some((code, what)), ..runner() };
        let err = member_on(&mut link, cmd("farcooler member claude-sign-in bob"), true).await.expect_err("refused");
        let lines = crate::error_code_lines(err.as_ref(), true);
        assert_eq!(lines[0], format!("code: {word}"));
        if !what.is_empty() {
            assert_eq!(lines[1], format!("what: {what}"));
        }
        assert!(err.to_string().contains(said), "{what}: {err}");
    }
}

/// A finish or a cancel the runner has no sign-in for says the sign-in is
/// over; a sign-out or a start that finds no member says so, not that.
#[tokio::test]
async fn a_missing_sign_in_is_over_and_a_missing_member_is_missing() {
    let gone = Some((pb::ErrorCode::NotFound, ""));
    for (line, said) in [
        (format!("farcooler member claude-sign-in-cancel {SESSION}"), "that sign-in is over"),
        ("farcooler member claude-sign-out bob".to_string(), "nobody on this runner"),
    ] {
        let mut link = Runner { refuse: gone, ..runner() };
        let err = member_on(&mut link, cmd(&line), true).await.expect_err("refused");
        assert!(err.to_string().contains(said), "{line}: {err}");
    }
    let mut link = Runner { refuse: gone, ..runner() };
    let err = sign_in_on(&mut link, Call::Finish { session: &SESSION.to_string(), code: "a#b" }, true).await.expect_err("refused");
    assert!(err.to_string().contains("that sign-in is over"), "{err}");
}

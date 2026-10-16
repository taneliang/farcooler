//! `farcooler member connectors` (ov-586), against a fake runner that
//! records what it was sent.

use super::*;
use crate::people::{MemberCmd, member_on};

struct Runner {
    sent: Vec<pb::Request>,
    connectors: bool,
}

impl tasks::DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        let mut have = vec![capability::REPRESENTATIVES.to_string()];
        if self.connectors {
            have.push(capability::REP_CONNECTORS.to_string());
        }
        have
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
        let Some(request::Payload::MemberConnectors(p)) = req.payload.clone() else { panic!("sent {}", req.method) };
        let servers = if p.set { p.servers.clone() } else { vec!["Slack".into()] };
        self.sent.push(req);
        Ok(pb::Result { value: Some(result::Value::MemberConnectorList(pb::MemberConnectorList { handle: p.handle, servers })) })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn cmd(line: &str) -> MemberCmd {
    use clap::Parser;
    match crate::Cli::try_parse_from(line.split('|')).unwrap().command {
        crate::Command::Member(cmd) => cmd,
        _ => panic!("not a member command"),
    }
}

/// No flag reads; `--allow` replaces the list with exactly those; `--none`
/// sends an empty list that turns them all off; and each names the runner's
/// capability, so an older runner refuses rather than ignoring it.
#[tokio::test]
async fn connectors_are_read_replaced_or_turned_off() {
    let mut link = Runner { sent: Vec::new(), connectors: true };
    let said = member_on(&mut link, cmd("farcooler|member|connectors|alice"), false).await.unwrap();
    assert_eq!(said, "alice's rep may use these claude.ai connectors: Slack");
    let said = member_on(&mut link, cmd("farcooler|member|connectors|alice|--allow|Slack|--allow|Google Drive"), false).await.unwrap();
    assert!(said.starts_with("alice's rep may use these claude.ai connectors: Slack, Google Drive."), "{said}");
    // Matching is exact, so the key each name matches is shown (review L2).
    assert!(said.contains("capitals included"), "{said}");
    assert!(said.contains("Slack matches mcp__claude_ai_Slack__*, Google Drive matches mcp__claude_ai_Google_Drive__*"), "{said}");
    let said = member_on(&mut link, cmd("farcooler|member|connectors|alice|--none"), true).await.unwrap();
    assert_eq!(said, r#"{"connectors":[],"handle":"alice"}"#);

    let sent: Vec<pb::MemberConnectors> = link
        .sent
        .iter()
        .map(|r| {
            assert_eq!(r.method, "member.connectors");
            assert!(r.required_capabilities.contains(&capability::REP_CONNECTORS.to_string()));
            let Some(request::Payload::MemberConnectors(p)) = r.payload.clone() else { unreachable!() };
            p
        })
        .collect();
    assert_eq!(sent[0], pb::MemberConnectors { handle: "alice".into(), servers: vec![], set: false });
    assert_eq!(sent[1], pb::MemberConnectors { handle: "alice".into(), servers: vec!["Slack".into(), "Google Drive".into()], set: true });
    assert_eq!(sent[2], pb::MemberConnectors { handle: "alice".into(), servers: vec![], set: true });

    use clap::Parser;
    assert!(crate::Cli::try_parse_from("farcooler|member|connectors|alice|--allow|Slack|--none".split('|')).is_err(), "both at once");
}

/// A runner from before connectors is told to update, and sent nothing.
#[tokio::test]
async fn an_older_runner_is_asked_nothing() {
    let mut link = Runner { sent: Vec::new(), connectors: false };
    let refused = member_on(&mut link, cmd("farcooler|member|connectors|alice|--allow|Slack"), false).await.unwrap_err();
    assert!(refused.to_string().contains("update it"), "{refused}");
    assert!(link.sent.is_empty());
}

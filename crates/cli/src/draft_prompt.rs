//! `terminal draft-prompt`: Ask the Orchestrator's paste (ov-184), held
//! behind a dialog when asked (ov-385), and `terminal draft-withdraw`.

use farcooler_client::session::draft_prompt::draft_hold_json;
use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request, result};

use super::{id_bytes, req, short, terminal_by_record, with};

/// Ask the daemon to paste `text` into a TUI pane's box and never press
/// Enter. It refuses, typing nothing, unless the pane is provably an idle
/// agent with an empty box; the error then reaches the caller as usual.
///
/// With `hold`, a runner with `draft_hold` holds the draft behind a dialog
/// instead of refusing for one, and the answer is a JSON line the Mac reads:
/// `{"pasted":true}`, or `{"held":<hold>}` (`draft_hold_json`). A runner
/// without it isn't asked, and refuses as it always did.
pub(crate) async fn run(
    runner: Option<&str>,
    terminal: &str,
    text: String,
    hold: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let can_hold = hold && link.daemon_capabilities().iter().any(|c| c == farcooler_protocol::capability::DRAFT_HOLD);
    let mut ask = with(
        req("terminal.draft_prompt"),
        request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: id_bytes(id),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text)) }],
            hold_behind_dialog: can_hold,
        }),
    );
    if can_hold {
        ask.required_capabilities = vec![farcooler_protocol::capability::DRAFT_HOLD.into()];
    }
    let answer = link.call(ask).await?;
    println!("{}", drafted(answer.value.as_ref(), hold, &short(id)));
    Ok(())
}

/// What a draft came to, as printed: a sentence, or with `hold` the JSON
/// line the Mac reads.
pub(crate) fn drafted(value: Option<&result::Value>, hold: bool, terminal: &str) -> String {
    match (value, hold) {
        (Some(result::Value::DraftHold(h)), _) => serde_json::json!({ "held": draft_hold_json(h) }).to_string(),
        (_, true) => serde_json::json!({ "pasted": true }).to_string(),
        (_, false) => format!("drafted in {terminal}"),
    }
}

/// Withdraw the draft `hold` held behind a dialog on `terminal`, and print
/// the hold as it now is: `{"hold":<hold>}`.
pub(crate) async fn withdraw(runner: Option<&str>, terminal: &str, hold: &str) -> Result<(), Box<dyn std::error::Error>> {
    let hold = uuid::Uuid::parse_str(hold).map_err(|_| "that isn't a draft's id")?;
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let mut ask = with(
        req("terminal.draft_withdraw"),
        request::Payload::DraftWithdraw(pb::DraftWithdraw {
            terminal_id: id_bytes(id),
            hold_id: bytes::Bytes::copy_from_slice(hold.as_bytes()),
        }),
    );
    ask.required_capabilities = vec![farcooler_protocol::capability::DRAFT_HOLD.into()];
    let answer = link.call(ask).await?;
    match answer.value {
        Some(result::Value::DraftHold(h)) => println!("{}", serde_json::json!({ "hold": draft_hold_json(&h) })),
        _ => return Err(crate::daemon_link::UNREADABLE.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `--hold`, a JSON line either way, which the Mac decodes; without,
    /// the sentence it always printed.
    #[test]
    fn a_held_draft_prints_its_hold_and_a_pasted_one_says_so() {
        let id = uuid::Uuid::now_v7();
        let hold = pb::DraftHold {
            id: bytes::Bytes::copy_from_slice(id.as_bytes()),
            state: pb::DraftHoldState::Waiting as i32,
            held_ms: 1_000,
            expires_ms: 1_801_000,
            ended_ms: 0,
        };
        let held: serde_json::Value =
            serde_json::from_str(&drafted(Some(&result::Value::DraftHold(hold)), true, "ab12")).unwrap();
        assert_eq!(held["held"]["id"], id.to_string());
        assert_eq!(held["held"]["state"], "waiting");
        assert_eq!(held["held"]["expiresMs"], 1_801_000);
        let terminal = result::Value::Terminal(pb::Terminal::default());
        assert_eq!(drafted(Some(&terminal), true, "ab12"), r#"{"pasted":true}"#);
        assert_eq!(drafted(Some(&terminal), false, "ab12"), "drafted in ab12");
    }
}

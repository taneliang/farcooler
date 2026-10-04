//! `terminal tell`: the Mac title bar's message to a terminal orchestrator
//! (ov-214).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request};

use super::{id_bytes, req, short, terminal_by_record, with};

/// Ask the daemon to type `text` into an orchestrator's TUI and press Enter.
/// It refuses, typing nothing, unless the pane is provably an idle agent
/// with an empty box (`Watcher::tell_into`); the refusal's word reaches the
/// caller as `what:`.
pub(crate) async fn run(runner: Option<&str>, terminal: &str, text: String) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    link.call(with(
        req("terminal.tell"),
        request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: id_bytes(id),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text)) }],
        }),
    ))
    .await?;
    println!("told {}", short(id));
    Ok(())
}

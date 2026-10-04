//! `terminal draft-prompt`: Ask the Orchestrator's paste (ov-184).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request};

use super::{id_bytes, req, short, terminal_by_record, with};

/// Ask the daemon to paste `text` into a TUI pane's box and never press
/// Enter. It refuses, typing nothing, unless the pane is provably an idle
/// agent with an empty box; the error then reaches the caller as usual.
pub(crate) async fn run(
    runner: Option<&str>,
    terminal: &str,
    text: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    link.call(with(
        req("terminal.draft_prompt"),
        request::Payload::AgentPrompt(pb::AgentPrompt {
            terminal_id: id_bytes(id),
            blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text)) }],
        }),
    ))
    .await?;
    println!("drafted in {}", short(id));
    Ok(())
}

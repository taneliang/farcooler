//! `terminal compose`: a native composer's message, with its line breaks,
//! images and slash command, typed into claude's box in a terminal pane and
//! submitted (ov-367). For testing, and for the Mac's composer to call.

use std::io::Read;
use std::path::PathBuf;

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request};

use super::{id_bytes, req, short, tasks, tell, terminal_by_record, with};

/// Ask the daemon to type `text` (or stdin, for `-`) and `images` into
/// claude's box in `terminal` and submit it. Prints whether claude took it as
/// its next prompt (sent) or queued it behind its turn; with `json`,
/// `{"queued":false}` or `{"queued":true}`. A refusal types nothing, or says
/// where the text was left, in this CLI's words and, under `--json`, the
/// runner's word (`what: handoff`).
pub(crate) async fn run(
    runner: Option<&str>,
    terminal: &str,
    text: String,
    images: Vec<PathBuf>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = if text == "-" {
        let mut read = String::new();
        std::io::stdin().read_to_string(&mut read)?;
        read
    } else {
        text
    };
    let mut blocks = vec![pb::AgentPromptBlock { content: Some(Content::Text(text)) }];
    blocks.extend(super::images::image_blocks(&images)?);
    let image_bytes: usize = blocks
        .iter()
        .filter_map(|b| match &b.content {
            Some(Content::Image(image)) => Some(image.data.len()),
            _ => None,
        })
        .sum();
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    use farcooler_protocol::capability::{AGENT_COMPOSE, COMPOSE, COMPOSE_UPLOAD};
    let offers = |need: &str| link.daemon_capabilities().iter().any(|c| c == need);
    if !(offers(AGENT_COMPOSE) && offers(COMPOSE)) {
        return Err("this runner can't compose into a terminal yet. update it".into());
    }
    // Uploaded first, in chunks, where the runner takes it (ov-393); else
    // carried in the one request, and refused here as the runner would,
    // before a frame too big to send.
    let upload = image_bytes > 0 && offers(COMPOSE_UPLOAD);
    let cap = if upload { farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES } else { farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES };
    if image_bytes > cap {
        let code = pb::ErrorCode::ResourceConflict as i32;
        let said = said_about("images_too_large").unwrap_or_default().to_string();
        return Err(Box::new(tasks::Refused::naming(said, code, "images_too_large".into())));
    }
    let mut required = vec![AGENT_COMPOSE.to_string(), COMPOSE.to_string()];
    if upload {
        for block in &mut blocks {
            if let Some(Content::Image(image)) = &block.content {
                let staged = farcooler_client::actions::stage_compose_image(link.client_mut(), id, &image.mime_type, &image.data)
                    .await
                    .map_err(refused)?;
                block.content = Some(Content::StagedImage(staged));
            }
        }
        required.push(COMPOSE_UPLOAD.to_string());
    }
    let mut ask = with(
        req("terminal.compose"),
        request::Payload::AgentPrompt(pb::AgentPrompt { terminal_id: id_bytes(id), blocks, hold_behind_dialog: false }),
    );
    ask.required_capabilities = required;
    // Targeted, as the apps' compose is: its order kept against the pane's
    // other input, beside every other pane's calls.
    ask.target_resource_id = Some(id_bytes(id));
    let answer = link.call(ask).await.map_err(refused)?;
    let queued = matches!(answer.value, Some(pb::result::Value::TerminalTold(pb::TerminalTold { queued: true })));
    if json {
        println!("{}", serde_json::json!({ "queued": queued }));
    } else {
        println!("{}", tell::told(answer.value.as_ref(), &short(id)));
    }
    Ok(())
}

/// This CLI's line for a refusal `terminal tell` doesn't have.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "handoff" => "that command opens a panel or acts at once in claude, so it's for the terminal. open the pane and type it there",
        "unsupported" => "only claude can be composed into. use terminal draft-prompt for this agent",
        "images_too_large" => "the images are too large to send together. send fewer, or smaller ones",
        "unconfirmable" => "the agent's session can't be found, so a send couldn't be confirmed. nothing was typed",
        "command" => "a message can't start with !, which claude reads as a shell command, or with a / that isn't a command",
        "too_long" => "that message is over 100,000 characters. shorten it",
        "busy" => {
            "the agent is working, and either can't be typed to safely now or the message is a command, which waits for the turn to end. try again when it's done"
        }
        "unconfirmed" => "the message was submitted, but claude never said it took it. check its pane before sending it again",
        "prompt" => "the agent is showing a question, a menu or a panel. answer it in the terminal first",
        "draft" => "there's a draft in the agent's box. send or clear it first, or bring it here",
        other => return tell::said_about(other),
    })
}

/// A refused compose, in this CLI's words when the runner named why.
fn refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && let Some(said) = said_about(what)
    {
        return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
    }
    Box::new(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every word the runner's `compose_into` can refuse with has a line.
    #[test]
    fn every_refusal_has_a_line() {
        for what in [
            "busy", "prompt", "draft", "typing", "not_an_agent", "unfamiliar", "unproven", "too_long", "command",
            "not_running", "paste_left", "left_at_shell", "dialog", "unconfirmed", "handoff", "unsupported", "unconfirmable",
            "images_too_large",
        ] {
            let said = said_about(what).unwrap_or_else(|| panic!("no line for {what}"));
            assert!(!said.ends_with('.') && said.chars().next().is_some_and(char::is_lowercase), "{said}");
        }
    }
}

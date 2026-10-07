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
    for path in images {
        let data = std::fs::read(&path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
        let image = pb::ImageBlock { mime_type: mime_of(&path).into(), data: data.into() };
        blocks.push(pb::AgentPromptBlock { content: Some(Content::Image(image)) });
    }
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    if !link.daemon_capabilities().iter().any(|c| c == farcooler_protocol::capability::COMPOSE) {
        return Err("this runner can't compose into a terminal yet. update it".into());
    }
    let mut ask = with(
        req("terminal.compose"),
        request::Payload::AgentPrompt(pb::AgentPrompt { terminal_id: id_bytes(id), blocks, hold_behind_dialog: false }),
    );
    ask.required_capabilities = vec![farcooler_protocol::capability::COMPOSE.into()];
    let answer = link.call(ask).await.map_err(refused)?;
    let queued = matches!(answer.value, Some(pb::result::Value::TerminalTold(pb::TerminalTold { queued: true })));
    if json {
        println!("{}", serde_json::json!({ "queued": queued }));
    } else {
        println!("{}", tell::told(answer.value.as_ref(), &short(id)));
    }
    Ok(())
}

/// The MIME type a path's extension claims; the runner sniffs the bytes.
fn mime_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// This CLI's line for a refusal `terminal tell` doesn't have.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "handoff" => "that command opens a panel or acts at once in claude, so it's for the terminal. open the pane and type it there",
        "unsupported" => "only claude can be composed into. use terminal draft-prompt for this agent",
        "no_session" => "Far Cooler can't find the agent's session, so it couldn't confirm a send. nothing was typed",
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
            "not_running", "paste_left", "left_at_shell", "dialog", "unconfirmed", "handoff", "unsupported", "no_session",
        ] {
            let said = said_about(what).unwrap_or_else(|| panic!("no line for {what}"));
            assert!(!said.ends_with('.') && said.chars().next().is_some_and(char::is_lowercase), "{said}");
        }
    }

    #[test]
    fn an_image_is_named_by_its_extension() {
        assert_eq!(mime_of(std::path::Path::new("/a/b.PNG")), "image/png");
        assert_eq!(mime_of(std::path::Path::new("shot.jpeg")), "image/jpeg");
        assert_eq!(mime_of(std::path::Path::new("notes")), "application/octet-stream");
    }
}

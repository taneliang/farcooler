//! `terminal tell`: a message typed into a terminal orchestrator and
//! submitted (ov-214), or queued when it's working (ov-360).

use farcooler_protocol::v1::{self as pb, agent_prompt_block::Content, request, result};

use super::{id_bytes, req, short, tasks, terminal_by_record, with};

/// Ask the daemon to type `text` into an orchestrator's TUI and press Enter.
/// It does when the pane is provably an agent with an empty box, between
/// turns (sent) or mid-turn in one that queues it (queued), and otherwise
/// refuses, typing nothing (`Watcher::tell_into`), in this CLI's words for
/// the runner's (`refused`).
pub(crate) async fn run(runner: Option<&str>, terminal: &str, text: String) -> Result<(), Box<dyn std::error::Error>> {
    let (mut link, id) = terminal_by_record(runner, terminal).await?;
    let answer = link
        .call(with(
            req("terminal.tell"),
            request::Payload::AgentPrompt(pb::AgentPrompt {
                terminal_id: id_bytes(id),
                blocks: vec![pb::AgentPromptBlock { content: Some(Content::Text(text)) }],
            }),
        ))
        .await
        .map_err(refused)?;
    println!("{}", told(answer.value.as_ref(), &short(id)));
    Ok(())
}

/// What a told message came to. A runner from before ov-360 answers with
/// the terminal, and it typed only between turns: sent.
pub(crate) fn told(value: Option<&result::Value>, terminal: &str) -> String {
    match value {
        Some(result::Value::TerminalTold(pb::TerminalTold { queued: true })) => {
            format!("queued for {terminal}: it's working, and takes this when it's ready")
        }
        _ => format!("sent to {terminal}"),
    }
}

/// This CLI's line for a refusal the runner named, in clap's style.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "busy" => {
            "the orchestrator is working, and Far Cooler can't tell whether a question would take the message, \
             so nothing was typed. try again when it's done"
        }
        "prompt" => "the orchestrator is showing a question or a menu. answer it first",
        "draft" => "there's a draft in the orchestrator's box. send or clear it first",
        "typing" => "someone is typing in the orchestrator's pane. try again in a few seconds",
        "not_an_agent" => "no agent is running in the orchestrator's pane",
        "unfamiliar" => "the orchestrator's screen isn't one Far Cooler recognizes, so nothing was typed",
        "unproven" => {
            "this runner's tmux is older than 3.7, so Far Cooler can't tell whether the agent takes a paste. restart the agent in its pane"
        }
        "too_long" => "that message is over 500 characters. shorten it",
        "command" => "a message can't start with / ! # @ & $ ? or \\, which the agent reads as a command",
        "not_running" => "the orchestrator isn't running",
        "paste_left" => "the message never showed in the box as typed, so it was left there, not sent",
        "left_at_shell" => "the agent quit while the message was typed. it's at a shell prompt, not run",
        "dialog" => "the orchestrator raised a question as the message was typed, so it's left in the box, not sent",
        "unconfirmed" => {
            "the message was submitted while the orchestrator worked, but never showed in its queue. check its pane before sending it again"
        }
        _ => return None,
    })
}

/// A refused tell, in this CLI's words when the runner named why.
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

    #[test]
    fn a_queued_message_says_so_and_anything_else_was_sent() {
        let queued = result::Value::TerminalTold(pb::TerminalTold { queued: true });
        assert_eq!(told(Some(&queued), "ab12"), "queued for ab12: it's working, and takes this when it's ready");
        let sent = result::Value::TerminalTold(pb::TerminalTold { queued: false });
        assert_eq!(told(Some(&sent), "ab12"), "sent to ab12");
        let older = result::Value::Terminal(pb::Terminal::default());
        assert_eq!(told(Some(&older), "ab12"), "sent to ab12");
    }

    /// Every word the runner's `tell_into` can refuse with has a line here.
    #[test]
    fn every_refusal_has_a_line() {
        for what in [
            "busy", "prompt", "draft", "typing", "not_an_agent", "unfamiliar", "unproven", "too_long", "command",
            "not_running", "paste_left", "left_at_shell", "dialog", "unconfirmed",
        ] {
            let said = said_about(what).unwrap_or_else(|| panic!("no line for {what}"));
            assert!(!said.ends_with('.') && said.chars().next().is_some_and(char::is_lowercase), "{said}");
        }
        assert_eq!(said_about("not_held"), None);
    }
}

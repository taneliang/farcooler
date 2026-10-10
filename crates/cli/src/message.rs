//! `farcooler message <to> <text>` (ov-455): an agent tells its orchestrator,
//! and the orchestrator tells a lane.
//!
//! The runner queues the message and types it into the recipient's box when
//! it's safe, tagged with who it's from (`[from mac-ux] PR is up`): a claude
//! working takes it into its own queue, a codex between turns, a chat pane on
//! its channel. It is told once, in order, and waits through a restart. See
//! `farcooler_daemon::watch::answer_wake::messages`.
//!
//! The sender is the pane's `FARCOOLER_ACTOR`, its card `FARCOOLER_TASK`, and
//! its board `FARCOOLER_WORKSPACE`, so an agent Far Cooler started needs no
//! flag: `farcooler message orchestrator "PR is up: #123"`.

use clap::Args;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::tasks::{self, actor_for, board_for};
use crate::workspaces::WORKSPACE_ENV;
use crate::{Fallible, connect_to, expect_value, req, with};

/// `farcooler message`'s arguments.
#[derive(Debug, Clone, Args)]
pub(crate) struct MessageArgs {
    /// Who it's for: `orchestrator`, a lane by name, or a card by key (the
    /// agent working it, or its lane's). An agent may only message its
    /// orchestrator.
    to: String,
    /// The message, one line of at most 400 characters. Put `--` before one
    /// that starts with a dash.
    text: String,
    /// Which board the names are on: a workspace by name or task prefix. Read
    /// from FARCOOLER_WORKSPACE in a pane.
    #[arg(long)]
    workspace: Option<String>,
    /// Which repository's board, when two have the workspace's name.
    #[arg(long)]
    repo: Option<String>,
    /// The card this message is about, for a sender with no pane: read from
    /// FARCOOLER_TASK when not given.
    #[arg(long)]
    task: Option<String>,
    /// Who this is from: `user`, `manager`, or `agent:<terminal id>`. Read
    /// from FARCOOLER_ACTOR when not given, and `user` when neither says.
    #[arg(long)]
    actor: Option<String>,
}

/// `farcooler message`, on a connected runner.
pub(crate) async fn message(runner: Option<&str>, args: MessageArgs, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    if !link.daemon_capabilities().iter().any(|c| c == capability::AGENT_MESSAGES) {
        return Err("this runner's Far Cooler can't pass messages yet. update it and try again".into());
    }
    let actor = actor_for(args.actor.as_deref())?;
    let env = std::env::var(WORKSPACE_ENV).ok();
    let board = board_for(&mut link, args.repo.as_deref(), args.workspace.as_deref(), env).await?;
    let task = args.task.or_else(|| std::env::var(farcooler_core::pane_env::TASK).ok()).unwrap_or_default();
    let mut r = with(
        req("message.send"),
        request::Payload::MessageSend(pb::MessageSend {
            to: args.to.trim().to_string(),
            text: args.text,
            actor: actor.to_string(),
            task,
            workspace_id: board.workspace.map(|w| w.id).unwrap_or_default(),
        }),
    );
    r.required_capabilities.push(capability::AGENT_MESSAGES.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::MessageSent(sent) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    println!("{}", sent_line(&sent, json));
    Ok(())
}

/// What's printed once it's queued.
pub(crate) fn sent_line(sent: &pb::MessageSent, json: bool) -> String {
    if json {
        return serde_json::json!({ "task": sent.task_key, "to": sent.recipient }).to_string();
    }
    format!("queued for {}, on {}. it's typed in when it's ready", sent.recipient, sent.task_key)
}

/// This CLI's line for a refusal the runner named, in clap's style.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "hub" => "an agent messages its orchestrator, not another lane: send it to `orchestrator`",
        "self" => "that's you. message a lane by name, or a card by key",
        "nobody" => "no agent is working that lane or card. dispatch one with `farcooler plan lane dispatch`",
        "to" => "there's no lane or card by that name on this board",
        "no_task" => "this pane works no card, so there's nowhere to file the message. name one with --task",
        "too_long" => "that message is over 400 characters. put the detail in a note or a report, and send its path",
        "flood" => "too many messages are waiting for them, or you've sent them 30 in the last hour. wait for an answer",
        "typing_off" => "this board doesn't let Far Cooler type into panes. turn on waking agents in its settings",
        "workspace" => "say which board with --workspace",
        "text" => "the message is empty",
        _ => return None,
    })
}

/// A refused message, in this CLI's words when the runner named why.
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
    fn a_queued_message_says_where_it_went() {
        let sent = pb::MessageSent { note_id: Default::default(), task_key: "ov-4".into(), recipient: "the lane mac-ux".into() };
        assert_eq!(sent_line(&sent, false), "queued for the lane mac-ux, on ov-4. it's typed in when it's ready");
        assert_eq!(sent_line(&sent, true), r#"{"task":"ov-4","to":"the lane mac-ux"}"#);
    }

    /// Every word the runner's `message_send` refuses with has a line here.
    #[test]
    fn every_refusal_has_a_line() {
        for what in ["hub", "self", "nobody", "to", "no_task", "too_long", "flood", "typing_off", "workspace", "text"] {
            let said = said_about(what).unwrap_or_else(|| panic!("no line for {what}"));
            assert!(!said.ends_with('.') && said.chars().next().is_some_and(char::is_lowercase), "{said}");
        }
    }

    /// The agent's own line parses: the destination and the text, in order.
    #[test]
    fn the_agents_line_parses() {
        use clap::Parser;
        let cli = crate::Cli::try_parse_from(["farcooler", "message", "orchestrator", "PR is up"]).unwrap();
        let crate::Command::Message(args) = cli.command else { panic!("not message") };
        assert_eq!((args.to.as_str(), args.text.as_str()), ("orchestrator", "PR is up"));
    }
}

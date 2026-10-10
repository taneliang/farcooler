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
    let var = |name: &str| std::env::var(name).ok();
    let pane = Pane { actor: var(ACTOR_ENV), workspace: var(WORKSPACE_ENV), task: var(farcooler_core::pane_env::TASK) };
    println!("{}", send(&mut link, args, pane, json).await?);
    Ok(())
}

/// What the pane the CLI runs in says about it: `FARCOOLER_ACTOR`,
/// `FARCOOLER_WORKSPACE` and `FARCOOLER_TASK`.
pub(crate) struct Pane {
    pub(crate) actor: Option<String>,
    pub(crate) workspace: Option<String>,
    pub(crate) task: Option<String>,
}

const ACTOR_ENV: &str = farcooler_core::pane_env::ACTOR;

/// `farcooler message` on `link`, from `pane`, answering what to print.
pub(crate) async fn send<L: tasks::DispatchLink>(
    link: &mut L,
    args: MessageArgs,
    pane: Pane,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::AGENT_MESSAGES) {
        return Err("this runner's Far Cooler can't pass messages yet. update it and try again".into());
    }
    let actor = actor_for(sender(args.actor.as_deref(), pane.actor.as_deref())?.or(pane.actor.as_deref()))?;
    let board = board_for(link, args.repo.as_deref(), args.workspace.as_deref(), pane.workspace).await?;
    let task = args.task.or(pane.task).unwrap_or_default();
    let mut r = with(
        req("message.send"),
        request::Payload::MessageSend(pb::MessageSend {
            to: args.to.trim().to_string(),
            text: args.text,
            actor: actor.to_string(),
            task,
            workspace_id: workspace_of(link, &board).await?,
        }),
    );
    r.required_capabilities.push(capability::AGENT_MESSAGES.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::MessageSent(sent) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(sent_line(&sent, json))
}

/// The board's workspace, or with none named, its repository's Main, as
/// `task create` and `plan` take it: an orchestrator outside its pane names
/// only `--repo`.
async fn workspace_of<L: tasks::DispatchLink>(link: &mut L, board: &tasks::Board) -> Result<bytes::Bytes, Box<dyn std::error::Error>> {
    if let Some(ws) = &board.workspace {
        return Ok(ws.id.clone());
    }
    let main = crate::workspaces::workspaces_on(link, Some(board.repository)).await?.into_iter().find(|w| w.is_main);
    Ok(main.map(|w| w.id).unwrap_or_default())
}

/// Who a message is from: `--actor`, unless this is an agent's pane
/// (`FARCOOLER_ACTOR=agent:…`), where only the pane's own name is taken. An
/// agent that names another sender, the orchestrator to get past the hub
/// rule, or a person, is refused rather than believed. A guardrail against
/// a mistake, not a boundary: the variable itself can be set by anything in
/// the pane (`message_send` in the daemon says the same).
pub(crate) fn sender<'a>(given: Option<&'a str>, pane: Option<&'a str>) -> Result<Option<&'a str>, Box<dyn std::error::Error>> {
    match (given, pane) {
        (Some(named), Some(own)) if own.starts_with("agent:") && named != own => {
            Err("an agent's pane messages as itself: leave out --actor".into())
        }
        (Some(named), _) => Ok(Some(named)),
        (None, _) => Ok(None),
    }
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

    /// In an agent's pane `--actor` can't name anyone else: not the
    /// orchestrator, not a person. Its own name, or none, is taken; outside
    /// an agent's pane `--actor` is as given.
    #[test]
    fn an_agent_cannot_message_as_someone_else() {
        let pane = Some("agent:01a0dad0-afd0-7bd1-9d62-3d125ede9ae2");
        for other in ["manager", "user", "agent:01a0dad0-afd0-7bd1-9d62-000000000000"] {
            assert!(sender(Some(other), pane).is_err(), "{other}");
        }
        assert_eq!(sender(pane, pane).unwrap(), pane);
        assert_eq!(sender(None, pane).unwrap(), None);
        assert_eq!(sender(Some("manager"), Some("manager")).unwrap(), Some("manager"));
        assert_eq!(sender(Some("user"), None).unwrap(), Some("user"));
    }

    /// A runner that answers every read and records what it's sent.
    struct Runner(Vec<pb::Request>);

    impl tasks::DispatchLink for Runner {
        fn capabilities(&self) -> Vec<String> {
            ["workstreams", "tasks", capability::AGENT_MESSAGES].map(String::from).to_vec()
        }
        async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
            let value = match req.method.as_str() {
                "workspace.list" => result::Value::WorkspaceList(pb::WorkspaceList {
                    items: vec![pb::Workspace { id: crate::id_bytes(WS), repository_id: crate::id_bytes(WS), is_main: true, ..Default::default() }],
                }),
                "message.send" => result::Value::MessageSent(pb::MessageSent::default()),
                other => panic!("message sent {other}, which this fake doesn't expect"),
            };
            self.0.push(req);
            Ok(pb::Result { value: Some(value) })
        }
        async fn pause(&mut self, _wait: std::time::Duration) {}
    }

    const WS: uuid::Uuid = uuid::Uuid::from_u128(0x0202);

    fn args(line: &str) -> MessageArgs {
        use clap::Parser;
        let cli = crate::Cli::try_parse_from(line.split_whitespace()).unwrap();
        let crate::Command::Message(args) = cli.command else { panic!("not message") };
        args
    }

    /// The real command, in an agent's pane, with `--actor user`: refused,
    /// and nothing is sent to be queued. Its own name goes through.
    #[tokio::test]
    async fn the_command_in_an_agents_pane_refuses_another_actor() {
        let agent = "agent:01a0dad0-afd0-7bd1-9d62-3d125ede9ae2";
        let pane = || Pane { actor: Some(agent.into()), workspace: Some(WS.to_string()), task: Some("ov-1".into()) };
        let mut link = Runner(Vec::new());
        let refused = send(&mut link, args("farcooler message orchestrator hi --actor user"), pane(), false).await;
        assert!(refused.is_err(), "an agent sent as the owner");
        assert!(!link.0.iter().any(|r| r.method == "message.send"), "something was sent to be queued");
        send(&mut link, args("farcooler message orchestrator hi"), pane(), false).await.unwrap();
        let sent: Vec<_> = link.0.iter().filter_map(|r| match &r.payload {
            Some(request::Payload::MessageSend(m)) => Some(m.actor.clone()),
            _ => None,
        }).collect();
        assert_eq!(sent, [agent]);
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

//! `agent dispatch` (ov-457, ov-872), against a fake runner.

use clap::Parser;
use farcooler_transport::ClientError;
use uuid::Uuid;

use super::super::{AgentCmd, Session, run_in};
use super::*;
use crate::id_bytes;

const REPO: Uuid = Uuid::from_u128(0x0101);
const WORKSPACE: Uuid = Uuid::from_u128(0x0202);
const WORKTREE: Uuid = Uuid::from_u128(0x0303);
const PANE: Uuid = Uuid::from_u128(0x0404);

fn task(n: u128, key: &str, status: pb::TaskStatus) -> pb::Task {
    pb::Task {
        id: id_bytes(Uuid::from_u128(0x1000 + n)),
        key: key.into(),
        title: format!("Title of {key}"),
        status: status as i32,
        workspace_id: id_bytes(WORKSPACE),
        repository_id: id_bytes(REPO),
        ..Default::default()
    }
}

/// A runner with three cards, the first done; it answers a dispatch's reads
/// and writes and records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    /// What the opened pane's agent reads as, and asks.
    activity: pb::AgentActivity,
    question: Option<String>,
}

fn runner() -> Runner {
    let caps = ["workstreams", "tasks", capability::BOARD_PLAN, capability::BOARD_LINE, capability::TERMINAL_TASK];
    Runner { capabilities: caps.map(String::from).to_vec(), sent: Vec::new(), activity: pb::AgentActivity::Working, question: None }
}

fn items() -> Vec<pb::Task> {
    vec![task(1, "ov-1", pb::TaskStatus::Done), task(2, "ov-2", pb::TaskStatus::Todo), task(3, "ov-3", pb::TaskStatus::Backlog)]
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        let (method, payload) = (req.method.clone(), req.payload.clone());
        self.sent.push(req);
        let (activity, question) = (self.activity as i32, self.question.clone());
        let pane = || pb::Terminal {
            id: id_bytes(PANE),
            state: pb::TerminalState::Running as i32,
            activity,
            blocked_question: question.clone(),
            ..Default::default()
        };
        let value = match (method.as_str(), payload) {
            ("task.list", _) => result::Value::TaskList(pb::TaskList { items: items(), reads: None }),
            ("worktree.list", _) => result::Value::WorktreeList(pb::WorktreeList {
                items: vec![pb::Worktree { id: id_bytes(WORKTREE), repository_id: id_bytes(REPO), task_name: "lane".into(), ..Default::default() }],
            }),
            ("terminal.list", _) => {
                let opened = self.sent.iter().any(|r| r.method == "terminal.create");
                result::Value::TerminalList(pb::TerminalList { items: if opened { vec![pane()] } else { vec![] }, ..Default::default() })
            }
            ("terminal.create", _) => result::Value::Terminal(pane()),
            ("task.get", _) => result::Value::TaskDetail(pb::TaskDetail { task: Some(items()[1].clone()), ..Default::default() }),
            ("task.update" | "task.set_status", _) => result::Value::Task(items()[1].clone()),
            ("task.get_by_key", _) => result::Value::TaskList(pb::TaskList { items: vec![], reads: None }),
            ("repository.list", _) => result::Value::RepositoryList(pb::RepositoryList {
                items: vec![pb::Repository { id: id_bytes(REPO), display_name: "overnight".into(), ..Default::default() }],
            }),
            (other, _) => panic!("agent dispatch sent {other}, which this fake doesn't expect"),
        };
        Ok(pb::Result { value: Some(value) })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

impl Runner {
    fn methods(&self) -> Vec<&str> {
        self.sent.iter().map(|r| r.method.as_str()).collect()
    }
    fn create(&self) -> (&pb::Request, &pb::TerminalCreate) {
        let req = self.sent.iter().find(|r| r.method == "terminal.create").expect("a pane was opened");
        let Some(farcooler_protocol::v1::request::Payload::TerminalCreate(create)) = &req.payload else { panic!("no payload") };
        (req, create)
    }
}

fn the_board() -> Board {
    Board {
        repository: REPO,
        workspace: Some(pb::Workspace { id: id_bytes(WORKSPACE), repository_id: id_bytes(REPO), ..Default::default() }),
        has_workspaces: true,
    }
}

fn parsed(args: &str) -> AgentCmd {
    let argv = format!("farcooler agent dispatch {args} --workspace main");
    let cmd = crate::Cli::try_parse_from(argv.split_whitespace().map(String::from).collect::<Vec<_>>())
        .map(|cli| match cli.command {
            crate::Command::Agent(a) => a.cmd,
            _ => panic!("not agent"),
        })
        .unwrap_or_else(|e| panic!("{argv}: {e}"));
    assert!(matches!(cmd, AgentCmd::Dispatch(_)));
    cmd
}

async fn say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    run_in(link, &the_board(), parsed(args), "manager", false, &Session::default()).await
}

/// A builder opens its pane on its first ticket, names its role, title and
/// the tickets after the first on the pane with the capability that carries
/// them, puts the card into progress, and says so.
#[tokio::test]
async fn a_builder_opens_on_its_first_ticket() {
    let mut link = runner();
    let said = say(&mut link, "--role builder --ticket ov-2 --ticket ov-3 --ticket ov-2 --worktree lane --harness codex --title Phones")
        .await
        .unwrap();
    let (req, create) = link.create();
    assert_eq!(create.task_key.as_deref(), Some("ov-2"));
    let agent = create.agent.clone().expect("opened as an agent");
    assert_eq!(agent.role, pb::AgentRole::Builder as i32);
    assert_eq!((agent.more_tickets, agent.title.as_str()), (vec!["ov-3".to_string()], "Phones"), "each ticket once");
    assert_eq!(create.command_preset, "codex");
    assert!(req.required_capabilities.contains(&capability::BOARD_LINE.to_string()), "{:?}", req.required_capabilities);
    assert!(said.starts_with("Builder is starting on ov-2, then ov-3.\nov-2 is in progress in lane"), "{said}");
    assert!(link.methods().contains(&"task.update") && link.methods().contains(&"task.set_status"), "{:?}", link.methods());
}

/// A reviewer works a ticket a builder built, so the card stays where it is:
/// no move onto the worktree, none into progress, and no warning that it
/// would.
#[tokio::test]
async fn a_reviewer_leaves_the_card_where_it_is() {
    let mut link = runner();
    let said = say(&mut link, "--role reviewer --ticket ov-2 --worktree lane").await.unwrap();
    let (_, create) = link.create();
    assert_eq!(create.agent.as_ref().map(|a| a.role), Some(pb::AgentRole::Reviewer as i32));
    assert!(!link.methods().contains(&"task.update") && !link.methods().contains(&"task.set_status"), "{:?}", link.methods());
    assert!(said.starts_with("Reviewer is starting on ov-2.\nterminal 00000404 is working ov-2 in lane"), "{said}");
}

/// A ticket on another board, or none at all, is refused before anything is
/// opened, and a ticket is required.
#[tokio::test]
async fn a_ticket_must_be_on_this_board() {
    let mut link = runner();
    let refused = say(&mut link, "--role builder --ticket ov-9 --worktree lane").await.unwrap_err().to_string();
    assert_eq!(refused, "No card here is called \"ov-9\".");
    assert!(!link.methods().contains(&"terminal.create") && !link.methods().contains(&"worktree.list"), "{:?}", link.methods());
    assert!(crate::Cli::try_parse_from(["farcooler", "agent", "dispatch", "--role", "builder", "--worktree", "lane"]).is_err());
    assert!(crate::Cli::try_parse_from(["farcooler", "agent", "dispatch", "--ticket", "ov-1", "--worktree", "lane"]).is_err(), "a role is named");
}

/// A runner from before agents could be named on a pane is told so before
/// anything is sent.
#[tokio::test]
async fn an_old_runner_is_refused_first() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::BOARD_LINE);
    let refused = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap_err().to_string();
    assert_eq!(refused, super::super::super::NEEDS_LINE);
    assert!(link.methods().is_empty(), "{:?}", link.methods());
}

/// Dispatch looks for the agent's first turn and says it began (ov-493).
#[tokio::test]
async fn an_agent_says_it_began() {
    let mut link = runner();
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap();
    assert!(said.ends_with("\n  its agent began its first turn"), "{said}");
}

/// An agent on claude's folder-trust prompt hasn't started, and dispatch
/// says it's the owner's to answer, in words and in `--json`.
#[tokio::test]
async fn an_agent_on_the_trust_prompt_says_so() {
    let mut link = runner();
    link.activity = pb::AgentActivity::Blocked;
    link.question = Some("Trust this folder?".into());
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap();
    assert!(said.contains("its agent hasn't started: Claude Code is asking whether to trust its folder"), "{said}");
    assert!(said.contains("tell them it's waiting in Needs You"), "{said}");
    let mut link = runner();
    link.activity = pb::AgentActivity::Blocked;
    link.question = Some("Trust this folder?".into());
    let json = say_json(&mut link, "--role builder --ticket ov-2 --ticket ov-3 --worktree lane").await;
    assert_eq!((json["first_turn"].as_str(), json["stopped_on"].as_str()), (Some("stopped"), Some("Trust this folder?")), "{json}");
    assert_eq!((json["role"].as_str(), json["tickets"].clone()), (Some("builder"), serde_json::json!(["ov-2", "ov-3"])));
}

/// An agent that hasn't begun by the last look says so, and where to see
/// it; dispatch looks a bounded number of times.
#[tokio::test]
async fn an_agent_that_hasnt_begun_says_where_to_look() {
    let mut link = runner();
    link.activity = pb::AgentActivity::Idle;
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap();
    assert!(said.contains("its agent hasn't begun its first turn yet: see `farcooler terminal screen 00000404`"), "{said}");
    let looks = link.sent.iter().skip_while(|r| r.method != "terminal.create").filter(|r| r.method == "terminal.list").count();
    assert!(looks <= FIRST_TURN_LOOKS + 4, "{looks} looks");
    let json = say_json(&mut runner_idle(), "--role builder --ticket ov-2 --worktree lane").await;
    assert_eq!(json["first_turn"], "not_yet", "{json}");
}

fn runner_idle() -> Runner {
    let mut link = runner();
    link.activity = pb::AgentActivity::Idle;
    link
}

async fn say_json(link: &mut Runner, args: &str) -> serde_json::Value {
    let said = run_in(link, &the_board(), parsed(args), "manager", true, &Session::default()).await.unwrap();
    serde_json::from_str(&said).unwrap()
}

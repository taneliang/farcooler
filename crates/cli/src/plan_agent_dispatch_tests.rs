//! `agent dispatch` (ov-457, ov-872), against a fake runner.

use clap::Parser;
use farcooler_transport::ClientError;
use serde_json::json;
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
    /// What the engine holds, as `workflow get` answers it: `None` is a runner with no engine.
    engine: Option<serde_json::Value>,
    /// The engine's answer to a start: `None` accepts it.
    refuse_start: Option<&'static str>,
    /// `workflow get` fails, as for a seat the engine turns away.
    engine_unanswered: bool,
    /// `workflow.start` fails with this code, as the runner refuses a taken branch or name (ov-967).
    start_error: Option<pb::ErrorCode>,
    /// `terminal.create` fails, as it does on a card the engine holds.
    refuse_pane: bool,
}

fn runner() -> Runner {
    let caps = ["workstreams", "tasks", capability::BOARD_PLAN, capability::BOARD_LINE, capability::TERMINAL_TASK];
    Runner { capabilities: caps.map(String::from).to_vec(), sent: Vec::new(), activity: pb::AgentActivity::Working, question: None, engine: None, refuse_start: None, engine_unanswered: false, start_error: None, refuse_pane: false }
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
        if (method == "workflow.get" && self.engine_unanswered) || (method == "terminal.create" && self.refuse_pane) {
            return Err(ClientError::Closed);
        }
        if let Some(code) = self.start_error.filter(|_| method == "workflow.start") {
            return Err(ClientError::Daemon { code: code as i32, retryable: false, message: String::new(), what: String::new() });
        }
        let value = match (method.as_str(), payload) {
            ("workflow.get", _) => result::Value::WorkflowReply(pb::WorkflowReply {
                schema: 1,
                json: self.engine.clone().expect("the engine was asked about").to_string(),
            }),
            ("workflow.start", _) => result::Value::WorkflowReply(pb::WorkflowReply {
                schema: 1,
                json: match self.refuse_start {
                    Some(why) => json!({"schema":1,"receipt":{"result":{"Err":why}}}),
                    None => json!({"schema":1,"receipt":{"result":{"Ok":null}}}),
                }
                .to_string(),
            }),
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

/// What `workflow get` says of a board whose cards are `held` (key, source, phase).
fn engine(held: &[(&str, &str, &str)]) -> serde_json::Value {
    let items: serde_json::Map<String, serde_json::Value> = held
        .iter()
        .map(|(key, source, phase)| {
            (key.to_string(), json!({"definition": {"source": source}, "phase": phase, "run": format!("run-{key}"), "pin": format!("pin-{key}")}))
        })
        .collect();
    json!({"schema":1,"configuration":{"pin":"wf1:published"},"state":{"revision":7,"epoch":1,"items":items}})
}

fn with_engine(held: &[(&str, &str, &str)]) -> Runner {
    let mut link = runner();
    link.capabilities.push(capability::WORKFLOW_COMMANDS.into());
    link.engine = Some(engine(held));
    link
}

fn starts(link: &Runner) -> Vec<serde_json::Value> {
    link.sent
        .iter()
        .filter(|r| r.method == "workflow.start")
        .map(|r| {
            let Some(farcooler_protocol::v1::request::Payload::WorkflowCommand(c)) = &r.payload else { panic!("no payload") };
            serde_json::from_str(&c.json).unwrap()
        })
        .collect()
}

/// A builder on tracked tickets is one Start, under the published pin and the first item's own run and pin, with
/// the rest following it (ov-970): one agent, one branch, one pull request. No worktree is made and no pane is
/// opened: the engine hands the tickets to a worker.
#[tokio::test]
async fn a_builder_on_tracked_tickets_goes_through_one_start() {
    let held = [("ov-2", "ticket/v1", "WaitingApproval"), ("ov-3", "ticket/v1", "WaitingApproval")];
    let mut link = with_engine(&held);
    let said = say(&mut link, "--role builder --ticket ov-2 --ticket ov-3").await.unwrap();
    let sent = starts(&link);
    assert_eq!(sent.len(), 1, "{:?}", link.methods());
    assert_eq!((sent[0]["item"].as_str(), sent[0]["run"].as_str(), sent[0]["pin"].as_str(), sent[0]["start_pin"].as_str()), (Some("ov-2"), Some("run-ov-2"), Some("pin-ov-2"), Some("wf1:published")));
    assert_eq!((sent[0]["mutation"]["revision"].as_u64(), sent[0]["mutation"]["epoch"].as_u64()), (Some(7), Some(1)));
    assert_eq!(sent[0]["with"], json!(["ov-3"]));
    for method in ["terminal.create", "worktree.create", "task.update", "task.set_status"] {
        assert!(!link.methods().contains(&method), "{method} was sent: {:?}", link.methods());
    }
    assert!(said.starts_with("The engine is starting ov-2 and ov-3 together, as one run."), "{said}");
    // One ticket is a run of its own, and says nothing of followers.
    let mut link = with_engine(&held[..1]);
    say(&mut link, "--role builder --ticket ov-2").await.unwrap();
    assert!(starts(&link)[0].get("with").is_none());
    // A ticket already following a run isn't free to start, and a closed one is pointed at reopen (ov-968).
    let mut link = with_engine(&held);
    link.engine.as_mut().unwrap()["state"]["items"]["ov-3"]["follows"] = json!("ov-9");
    let said = say(&mut link, "--role builder --ticket ov-2 --ticket ov-3").await.unwrap_err().to_string();
    assert!(said.starts_with("ov-3 follows another ticket's run, so it can't be dispatched."), "{said}");
    let mut link = with_engine(&[("ov-2", "ticket/v1", "Done")]);
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.contains("Reopen it with `farcooler workflow reopen "), "{said}");
}

/// A board with no engine, or no tracked ticket, dispatches a pane exactly as before.
#[tokio::test]
async fn untracked_tickets_still_open_a_pane() {
    let mut link = with_engine(&[("ov-9", "ticket/v1", "WaitingApproval")]);
    say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap();
    assert!(starts(&link).is_empty() && link.methods().contains(&"terminal.create"), "{:?}", link.methods());
}

/// Nothing starts when any ticket can't: a mix, a ticket with a run, a closed one, a reviewer, no
/// published workflow, and the engine's own refusal (which names what already started).
#[tokio::test]
async fn nothing_starts_that_the_engine_cannot_take() {
    let mixed = [("ov-2", "ticket/v1", "WaitingApproval")];
    let mut link = with_engine(&mixed);
    let said = say(&mut link, "--role builder --ticket ov-2 --ticket ov-3").await.unwrap_err().to_string();
    assert!(said.starts_with("ov-3 isn't tracked by the engine"), "{said}");
    let mut link = with_engine(&[("ov-2", "configured-coding/v1", "Running")]);
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.starts_with("ov-2 already has a run under the engine"), "{said}");
    let mut link = with_engine(&[("ov-2", "ticket/v1", "Done")]);
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.starts_with("ov-2 has closed under the engine"), "{said}");
    let mut link = with_engine(&mixed);
    let said = say(&mut link, "--role reviewer --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.starts_with("A reviewer can't be dispatched on a tracked ticket"), "{said}");
    let mut link = with_engine(&mixed);
    link.engine.as_mut().unwrap()["configuration"] = serde_json::Value::Null;
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.starts_with("This board has no published workflow"), "{said}");
    let mut link = with_engine(&mixed);
    link.refuse_start = Some("Resource");
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert!(said.starts_with("The engine didn't start ov-2: something else holds what it needs, or a subagent still works it or it has a block."), "{said}");
    // What to do about the second (ov-965): the commands that settle each.
    assert!(said.contains("`farcooler agent end <agent> --verdict stopped`") && said.contains("`farcooler task block ov-2 --on <key> --clear`"), "{said}");
    for m in ["terminal.create", "worktree.create"] {
        assert!(!link.methods().contains(&m), "{:?}", link.methods());
    }
}

/// A flag a start can't honor is named and refused, not dropped with an exit of 0 (ov-962 B3). Nothing starts.
#[tokio::test]
async fn a_flag_the_engine_would_drop_is_named() {
    let held = [("ov-2", "ticket/v1", "WaitingApproval")];
    let mut link = with_engine(&held);
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane --title Phones").await.unwrap_err().to_string();
    assert!(said.contains("--worktree and --title would be ignored"), "{said}");
    assert!(said.ends_with("Leave them out and dispatch again."), "{said}");
    assert!(starts(&link).is_empty(), "{:?}", link.methods());
    let mut link = with_engine(&held);
    let said = say(&mut link, "--role builder --ticket ov-2 --new lane --branch b --base main").await.unwrap_err().to_string();
    assert!(said.contains("so --base would be ignored. Leave it out"), "{said}");
    // Untracked tickets keep every flag, and a pane still needs its worktree.
    let mut link = with_engine(&[("ov-9", "ticket/v1", "WaitingApproval")]);
    assert!(say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string().contains("Name a worktree"));
}

/// A ticket that has closed is in the engine's archive, not its items, from the step after it closes.
#[tokio::test]
async fn a_closed_ticket_in_the_archive_is_not_dispatched_to_a_pane() {
    let mut link = with_engine(&[("ov-9", "ticket/v1", "WaitingApproval")]);
    link.engine.as_mut().unwrap()["state"]["archive"] = json!({"ov-2": {"phase": "Done", "candidate": null}});
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap_err().to_string();
    assert!(said.starts_with("ov-2 has closed under the engine"), "{said}");
    assert!(!link.methods().contains(&"terminal.create"), "{:?}", link.methods());
}

/// A start the board moved under is read and sent once more, and a second miss is worded.
#[tokio::test]
async fn a_start_the_board_moved_under_is_sent_again_once() {
    let mut link = with_engine(&[("ov-2", "ticket/v1", "WaitingApproval")]);
    link.refuse_start = Some("Stale");
    let said = say(&mut link, "--role builder --ticket ov-2").await.unwrap_err().to_string();
    assert_eq!(starts(&link).len(), 2, "one retry");
    assert!(said.starts_with("The engine didn't start ov-2: the board moved while it was starting."), "{said}");
}

/// A seat the engine turns away from `workflow get` can't tell whether a ticket is tracked. It still gets a
/// pane, and when the pane fails it is told the ticket may be tracked.
#[tokio::test]
async fn an_unanswered_engine_is_said_when_the_pane_fails() {
    let mut link = with_engine(&[]);
    link.engine_unanswered = true;
    link.refuse_pane = true;
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap_err().to_string();
    assert!(said.contains("only the workspace owner's orchestrator can start it"), "{said}");
    let mut link = with_engine(&[]);
    link.refuse_pane = true;
    let said = say(&mut link, "--role builder --ticket ov-2 --worktree lane").await.unwrap_err().to_string();
    assert!(!said.contains("workspace owner"), "an answered engine adds nothing: {said}");
}

/// `task dispatch` asks the engine before it makes a worktree, and sends a tracked ticket to `agent dispatch`.
#[tokio::test]
async fn task_dispatch_turns_a_tracked_ticket_away_before_any_worktree() {
    let mut link = with_engine(&[("ov-2", "ticket/v1", "WaitingApproval")]);
    let said = super::super::start::refuse_tracked(&mut link, &items()[1], true).await.unwrap_err().to_string();
    assert!(said.contains("ov-2 is tracked by the engine") && said.contains("agent dispatch --role builder --ticket ov-2"), "{said}");
    let mut link = with_engine(&[("ov-9", "ticket/v1", "WaitingApproval")]);
    super::super::start::refuse_tracked(&mut link, &items()[1], true).await.unwrap();
    assert!(link.methods().iter().all(|m| *m == "workflow.get"), "{:?}", link.methods());
}

/// A member's orchestrator can't be asked: `task dispatch --new` makes no worktree on a guess, `--worktree` goes on.
#[tokio::test]
async fn task_dispatch_makes_no_worktree_when_the_engine_cannot_be_asked() {
    let mut link = with_engine(&[]);
    link.engine_unanswered = true;
    let said = super::super::start::refuse_tracked(&mut link, &items()[1], true).await.unwrap_err().to_string();
    assert!(said.contains("no worktree was made"), "{said}");
    super::super::start::refuse_tracked(&mut link, &items()[1], false).await.unwrap();
}

/// ov-966: the published workflow with a build step that lists the models it may run with.
fn with_models(held: &[(&str, &str, &str)], models: serde_json::Value) -> Runner {
    let mut link = with_engine(held);
    link.engine.as_mut().unwrap()["configuration"]["definition"] = json!({"models": models});
    link
}

/// ov-966: `--harness` on tracked tickets is the orchestrator's model pick for the build step, sent with each
/// Start. Left out, nothing is picked and each run takes the step's default.
#[tokio::test]
async fn harness_on_tracked_tickets_picks_the_build_steps_model() {
    let held = [("ov-2", "ticket/v1", "WaitingApproval"), ("ov-3", "ticket/v1", "WaitingApproval")];
    let mut link = with_models(&held, json!({"build": ["claude:opus", "codex:gpt-5.5"]}));
    say(&mut link, "--role builder --ticket ov-2 --ticket ov-3 --harness codex:gpt-5.5").await.unwrap();
    let sent = starts(&link);
    assert_eq!(sent.len(), 1, "one run for both (ov-970)");
    assert_eq!(sent[0]["models"], json!({"build": "codex:gpt-5.5"}), "{sent:?}");
    let mut link = with_models(&held, json!({"build": ["claude:opus", "codex:gpt-5.5"]}));
    say(&mut link, "--role builder --ticket ov-2").await.unwrap();
    assert!(starts(&link)[0].get("models").is_none(), "no pick, the default: {:?}", starts(&link));
}

/// A pick the workflow can't take is refused before anything starts, with what to do instead.
#[tokio::test]
async fn a_harness_the_workflow_cannot_take_is_refused_with_what_to_do() {
    let held = [("ov-2", "ticket/v1", "WaitingApproval")];
    let cases = [
        (json!({"build": ["claude:opus", "codex:gpt-5.5"]}), "--harness cursor",
            "The published workflow's build step runs claude:opus or codex:gpt-5.5, not cursor. Pick one of those, or leave --harness out for claude:opus."),
        (serde_json::Value::Null, "--harness codex",
            "The published workflow doesn't let the orchestrator pick a model, so --harness would be ignored. Leave it out, or publish a workflow whose build step lists the models it may run with."),
        (json!({"build": ["claude:opus"], "fix": ["codex"]}), "--harness codex",
            "The published workflow lists models for more than one step (build and fix). Start each ticket with `farcooler workflow start --model <step>=<model>`."),
    ];
    for (models, flag, words) in cases {
        let mut link = with_models(&held, models);
        let said = say(&mut link, &format!("--role builder --ticket ov-2 {flag}")).await.unwrap_err().to_string();
        assert_eq!(said, words);
        assert!(starts(&link).is_empty(), "{:?}", link.methods());
    }
}

/// ov-967: `--new` and `--branch` on a tracked ticket name the worktree its run makes, which the app lists.
#[tokio::test]
async fn new_and_branch_on_a_tracked_ticket_name_its_runs_worktree() {
    let mut link = with_engine(&[("ov-2", "ticket/v1", "WaitingApproval")]);
    let said = say(&mut link, "--role builder --ticket ov-2 --new phones --branch e-liang/phones").await.unwrap();
    assert_eq!(starts(&link)[0]["worktree"], json!({"name": "phones", "branch": "e-liang/phones"}));
    assert!(said.contains("in the worktree phones, on the branch e-liang/phones"), "{said}");
    for m in ["worktree.create", "terminal.create"] { assert!(!link.methods().contains(&m), "the run makes it: {:?}", link.methods()); }
}

/// Several tickets are one run, so one name covers them all (ov-970); and a taken branch or name is said as
/// itself, not as an engine that didn't answer.
#[tokio::test]
async fn a_named_worktree_covers_the_run_and_a_taken_one_is_said() {
    let held = [("ov-2", "ticket/v1", "WaitingApproval"), ("ov-3", "ticket/v1", "WaitingApproval")];
    let mut link = with_engine(&held);
    say(&mut link, "--role builder --ticket ov-2 --ticket ov-3 --new phones --branch b").await.unwrap();
    let sent = starts(&link);
    assert_eq!((sent.len(), &sent[0]["worktree"]["name"], &sent[0]["with"]), (1, &json!("phones"), &json!(["ov-3"])));
    for (code, words) in [
        (pb::ErrorCode::BranchExists, "ov-2 wasn't started: the branch b already exists. Name a new one with --branch."),
        (pb::ErrorCode::WorktreeExists, "ov-2 wasn't started: a worktree named phones already exists. Name another with --new."),
    ] {
        let mut link = with_engine(&held[..1]);
        link.start_error = Some(code);
        let said = say(&mut link, "--role builder --ticket ov-2 --new phones --branch b").await.unwrap_err().to_string();
        assert_eq!(said, words);
    }
}

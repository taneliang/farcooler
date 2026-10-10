//! `plan lane dispatch` (ov-457), against a fake runner.

use clap::Parser;
use farcooler_transport::ClientError;
use uuid::Uuid;

use super::super::{PlanCmd, run_on};
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

fn lane(name: &str, cards: &[u128]) -> pb::Lane {
    pb::Lane {
        id: id_bytes(Uuid::from_u128(0x2001)),
        workspace_id: id_bytes(WORKSPACE),
        name: name.into(),
        state: pb::LaneState::Queued as i32,
        cards: cards.iter().map(|c| pb::LaneCard { task_id: id_bytes(Uuid::from_u128(0x1000 + c)), slice: String::new(), stage: None }).collect(),
        ..Default::default()
    }
}

/// A runner with three cards (the first done) and one queued lane,
/// `mac-ux`, holding all three; it answers a dispatch's reads and writes and
/// records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
}

fn runner() -> Runner {
    let caps = ["workstreams", "tasks", capability::BOARD_PLAN, capability::TERMINAL_TASK, capability::TERMINAL_LANE];
    Runner { capabilities: caps.map(String::from).to_vec(), sent: Vec::new() }
}

fn items() -> Vec<pb::Task> {
    vec![task(1, "ov-1", pb::TaskStatus::Done), task(2, "ov-2", pb::TaskStatus::Todo), task(3, "ov-3", pb::TaskStatus::Backlog)]
}

fn the_plan() -> pb::Plan {
    pb::Plan { lanes: vec![lane("mac-ux", &[1, 2, 3])], ..Default::default() }
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        let (method, payload) = (req.method.clone(), req.payload.clone());
        self.sent.push(req);
        let pane = || pb::Terminal { id: id_bytes(PANE), state: pb::TerminalState::Running as i32, ..Default::default() };
        let value = match (method.as_str(), payload) {
            ("plan.get", _) => result::Value::Plan(the_plan()),
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
            ("lane.create", Some(request::Payload::LaneCreate(p))) => {
                let mut made = lane(&p.name, &[]);
                made.cards = p.cards;
                result::Value::Lane(made)
            }
            ("lane.cards", Some(request::Payload::LaneCards(p))) => {
                let mut changed = the_plan().lanes.remove(0);
                changed.cards.extend(p.add);
                result::Value::Lane(changed)
            }
            (other, _) => panic!("lane dispatch sent {other}, which this fake doesn't expect"),
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
        let Some(request::Payload::TerminalCreate(create)) = &req.payload else { panic!("no payload") };
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

async fn say(link: &mut Runner, args: &str) -> Result<String, Failed> {
    let argv = format!("farcooler plan lane dispatch {args} --workspace main");
    let cmd = crate::Cli::try_parse_from(argv.split_whitespace().map(String::from).collect::<Vec<_>>())
        .map(|cli| match cli.command {
            crate::Command::Plan(a) => a.cmd,
            _ => panic!("not plan"),
        })
        .unwrap_or_else(|e| panic!("{argv}: {e}"));
    assert!(matches!(cmd, Some(PlanCmd::Lane(_))));
    run_on(link, &the_board(), cmd, "manager", false, 0).await
}

/// An existing lane opens its pane on its first card that isn't done, names
/// the lane on the pane with the capability that carries it, and says so.
#[tokio::test]
async fn a_lane_opens_on_its_first_open_card() {
    let mut link = runner();
    let said = say(&mut link, "mac-ux --worktree lane --preset codex").await.unwrap();
    let (req, create) = link.create();
    assert_eq!(create.task_key.as_deref(), Some("ov-2"), "ov-1 is done");
    assert_eq!(create.lane.as_deref(), Some("mac-ux"));
    assert_eq!(create.command_preset, "codex");
    assert!(req.required_capabilities.contains(&capability::TERMINAL_LANE.to_string()), "{:?}", req.required_capabilities);
    assert!(said.starts_with("Lane mac-ux is building, starting on ov-2.\nov-2 is in progress in lane"), "{said}");
    assert!(!link.methods().contains(&"lane.create") && !link.methods().contains(&"lane.cards"), "{:?}", link.methods());
}

/// A lane that doesn't exist is made from `--card`, in the order given, and
/// its pane opens on the first.
#[tokio::test]
async fn a_new_lane_is_made_from_its_cards() {
    let mut link = runner();
    say(&mut link, "phones --card ov-3 --card ov-2 --worktree lane").await.unwrap();
    let made = link.sent.iter().find(|r| r.method == "lane.create").expect("the lane was made");
    let Some(request::Payload::LaneCreate(p)) = &made.payload else { panic!("no payload") };
    let keys: Vec<_> = p.cards.iter().map(|c| c.task_id.clone()).collect();
    assert_eq!(keys, [items()[2].id.clone(), items()[1].id.clone()]);
    let (_, create) = link.create();
    assert_eq!((create.task_key.as_deref(), create.lane.as_deref()), (Some("ov-3"), Some("phones")));
}

/// A lane with no such name and no cards to make it from is refused before
/// anything is written.
#[tokio::test]
async fn no_lane_and_no_cards_is_refused() {
    let mut link = runner();
    let refused = say(&mut link, "nowhere --worktree lane").await.unwrap_err().to_string();
    assert_eq!(refused, "No lane here is called \"nowhere\". Name its cards with --card to make it.");
    assert!(link.methods().iter().all(|m| *m == "plan.get" || *m == "task.list"), "{:?}", link.methods());
}

/// A runner from before lanes could be named on a pane is told so before
/// anything is sent past the plan's read.
#[tokio::test]
async fn an_old_runner_is_refused_first() {
    let mut link = runner();
    link.capabilities.retain(|c| c != capability::TERMINAL_LANE);
    let refused = say(&mut link, "mac-ux --worktree lane").await.unwrap_err().to_string();
    assert!(refused.contains("can't open a pane for a lane yet"), "{refused}");
    assert_eq!(link.methods(), ["plan.get"]);
}

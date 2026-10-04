use clap::Parser;

use super::*;

const REPO: Uuid = Uuid::from_u128(0x0101);
const WORKSPACE: Uuid = Uuid::from_u128(0x0202);
const TASK: Uuid = Uuid::from_u128(0x0303);

/// A runner that answers `workspace.mark_read` and `task.list` with a fixed
/// state, and records what it was sent.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
    refuse: Option<&'static str>,
}

fn runner(capabilities: &[&str]) -> Runner {
    Runner { capabilities: capabilities.iter().map(|c| c.to_string()).collect(), sent: vec![], refuse: None }
}

fn state() -> pb::BoardReads {
    pb::BoardReads {
        workspace_id: id_bytes(WORKSPACE),
        floor_ms: 100,
        opened: vec![pb::TaskRead { task_id: id_bytes(TASK), opened_ms: 250 }],
    }
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        let method = req.method.clone();
        self.sent.push(req);
        if let Some(what) = self.refuse {
            return Err(ClientError::Daemon {
                code: pb::ErrorCode::InvalidArgument as i32,
                retryable: false,
                message: String::new(),
                what: what.into(),
            });
        }
        Ok(pb::Result {
            value: Some(match method.as_str() {
                "workspace.mark_read" => result::Value::BoardReads(state()),
                "task.list" => result::Value::TaskList(pb::TaskList { items: vec![], reads: Some(state()) }),
                other => panic!("sent {other}"),
            }),
        })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn the_board() -> Board {
    Board {
        repository: REPO,
        workspace: Some(pb::Workspace { id: id_bytes(WORKSPACE), repository_id: id_bytes(REPO), ..Default::default() }),
        has_workspaces: true,
    }
}

#[tokio::test]
async fn a_mark_is_sent_as_the_runner_clock_time_it_was_given() {
    let mut link = runner(&["workstreams", "board_reads"]);
    let reads = mark_read(&mut link, &the_board(), Some(90), vec![(TASK, 250)], true).await.expect("marked");
    assert_eq!(reads, state());
    let sent = &link.sent[0];
    assert_eq!(sent.method, "workspace.mark_read");
    assert_eq!(sent.required_capabilities, vec!["board_reads".to_string()], "an older runner must refuse, not drop it");
    let Some(request::Payload::WorkspaceMarkRead(p)) = &sent.payload else { panic!("{sent:?}") };
    assert_eq!(p.workspace_id, id_bytes(WORKSPACE));
    assert_eq!((p.floor_ms, p.seeds_floor), (Some(90), true));
    assert_eq!(p.opened, vec![pb::TaskRead { task_id: id_bytes(TASK), opened_ms: 250 }]);
}

#[tokio::test]
async fn a_runner_without_shared_read_state_is_refused_before_the_wire() {
    let mut link = runner(&["workstreams"]);
    let err = mark_read(&mut link, &the_board(), None, vec![(TASK, 5)], false).await.unwrap_err();
    assert_eq!(err.to_string(), NO_READS);
    assert!(link.sent.is_empty(), "nothing was sent");
}

#[tokio::test]
async fn a_ticket_from_another_board_is_said_in_words_about_a_board() {
    let mut link = runner(&["board_reads"]);
    link.refuse = Some("other_board");
    let err = mark_read(&mut link, &the_board(), None, vec![(TASK, 5)], false).await.unwrap_err();
    assert_eq!(err.to_string(), "those tickets aren't all on this board");
}

/// `task list --json` prints `reads` beside `tasks`, and a reader of `tasks`
/// alone is unaffected.
#[tokio::test]
async fn a_boards_list_prints_its_reads_beside_its_tasks() {
    let mut link = runner(&["workstreams", "tasks", "board_reads"]);
    let out = crate::tasks::listing(&mut link, &the_board(), None, None, true).await.expect("list");
    let board: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(board["tasks"], serde_json::json!([]));
    assert_eq!(board["reads"]["floor_ms"], 100);
    assert_eq!(board["reads"]["opened"][0]["opened_ms"], 250);
}

#[test]
fn marks_are_an_id_and_a_time_and_nothing_else() {
    assert_eq!(parse_mark(&format!("{TASK}:250")).unwrap(), (TASK, 250));
    for bad in ["", "250", &format!("{TASK}"), &format!("{TASK}:soon"), &format!("{TASK}:-1"), "not-an-id:5"] {
        assert!(parse_mark(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_mac_s_argv_parses() {
    let argv = format!(
        "farcooler board mark-read --repo overnight --workspace main --task {TASK}:250 --task {WORKSPACE}:9 --floor 100 --seed --json"
    );
    let cli = crate::Cli::try_parse_from(argv.split_whitespace()).expect("parses");
    assert!(cli.json);
    let crate::Command::Board(BoardCmd::MarkRead { tasks, floor, seed, workspace, .. }) = cli.command else { panic!() };
    assert_eq!((tasks.len(), floor, seed, workspace.as_deref()), (2, Some(100), true, Some("main")));
    // A seed without a floor is nothing to seed.
    let alone = "farcooler board mark-read --workspace main --seed";
    assert!(crate::Cli::try_parse_from(alone.split_whitespace()).is_err());
}

/// The events stream says it, as its own kind: not `task`, which makes every
/// board re-read.
#[test]
fn the_events_stream_carries_the_state_as_reads() {
    let line = crate::event_lines::reads_event_json(&state());
    assert_eq!(line["kind"], "reads");
    assert_eq!(line["workspace_id"], WORKSPACE.to_string());
    assert_eq!(line["opened"][0]["task_id"], TASK.to_string());
}

use std::time::Duration;

use clap::Parser;
use farcooler_transport::ClientError;

use super::*;

/// A runner that says what it can do and answers nothing: a test of what
/// is refused before anything is sent.
struct Silent {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
}

impl DispatchLink for Silent {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        self.sent.push(req);
        Err(ClientError::EmptyResult)
    }
    async fn pause(&mut self, _wait: Duration) {}
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: StartCmd,
}

fn parsed(args: &[&str]) -> Result<StartCmd, clap::Error> {
    Cli::try_parse_from(std::iter::once("task").chain(args.iter().copied())).map(|c| c.cmd)
}

/// A runner without the capability drops the request's fields on the floor,
/// so each command refuses before sending anything, with the sentence that
/// says what to do.
#[tokio::test]
async fn an_old_runner_is_told_before_anything_is_sent() {
    let cases = [
        (vec!["wait", "ov-1", "--park"], "this runner needs an update to record when a task starts"),
        (vec!["line", "ov-1", "ov-2"], "this runner needs an update to record when a task starts"),
        (vec!["line", "--build"], "this runner needs an update to record when a task starts"),
        (vec!["worker", "ov-1", "--subagent", "a1"], "this runner needs an update to record subagents"),
    ];
    for (args, sentence) in cases {
        let mut link = Silent { capabilities: vec![capability::TASKS.into(), capability::WORKSTREAMS.into()], sent: Vec::new() };
        let err = run(&mut link, parsed(&args).unwrap(), false, &SessionEnv::default()).await.unwrap_err();
        assert_eq!(err.to_string(), sentence, "{args:?}");
        assert!(link.sent.is_empty(), "{args:?} sent {:?}", link.sent.iter().map(|r| &r.method).collect::<Vec<_>>());
    }
}

/// Each `task wait` flag, read; a time that has passed and a word that
/// isn't an event refused here, before a round trip.
#[test]
fn task_wait_reads_each_flag() {
    let now = 1_759_000_000_000;
    let later = crate::report::local_time("2099-10-05 09:00").unwrap();
    let until = wait_asked(Some("2099-10-05 09:00"), None, false, false, now).unwrap();
    assert_eq!((until.kind, until.until), (pb::TaskWaitKind::Until as i32, later));
    assert_eq!(
        wait_asked(Some("2001-01-01"), None, false, false, now).unwrap_err(),
        "that time has passed. name one still to come"
    );
    assert!(wait_asked(Some("next tuesday"), None, false, false, now).unwrap_err().contains("isn't a time"));
    for (word, event) in [
        ("release", pb::TaskWaitEvent::Release),
        ("recurrence", pb::TaskWaitEvent::Recurrence),
        ("clear-board", pb::TaskWaitEvent::ClearBoard),
        ("clear_board", pb::TaskWaitEvent::ClearBoard),
    ] {
        let after = wait_asked(None, Some(word), false, false, now).unwrap();
        assert_eq!((after.kind, after.event), (pb::TaskWaitKind::After as i32, event as i32), "{word}");
    }
    assert!(wait_asked(None, Some("launch"), false, false, now).unwrap_err().contains("isn't an event"));
    assert_eq!(wait_asked(None, None, true, false, now).unwrap().kind, pb::TaskWaitKind::Parked as i32);
    assert_eq!(wait_asked(None, None, false, true, now).unwrap(), pb::TaskSetWait::default(), "clear sends no kind");
    assert!(wait_asked(None, None, false, false, now).unwrap_err().starts_with("say how it waits"));
}

/// clap holds the flags apart: one way to wait, `--stopped` only with
/// `--done`, and a line either named or cleared.
#[test]
fn the_flags_that_cannot_go_together_are_refused() {
    assert!(parsed(&["wait", "ov-1", "--park", "--clear"]).is_err());
    assert!(parsed(&["wait", "ov-1", "--until", "2099-01-01", "--after", "release"]).is_err());
    assert!(parsed(&["worker", "ov-1", "--subagent", "a1", "--stopped"]).is_err());
    assert!(parsed(&["line", "ov-1", "--clear"]).is_err());
    assert!(parsed(&["worker", "ov-1"]).is_err(), "a subagent has to be named");
    assert!(matches!(parsed(&["line", "--build", "ov-3", "ov-1"]).unwrap(), StartCmd::Line { build: true, ref keys, .. } if keys == &["ov-3", "ov-1"]));
}

fn args(done: bool) -> WorkerArgs {
    WorkerArgs {
        subagent: "a3fd8fceef581c787".into(),
        harness: "claude".into(),
        label: Some("ov-12 Mac polish".into()),
        model: None,
        done,
        stopped: false,
    }
}

/// From the orchestrator's own session, `task worker` fills in the session,
/// folder and pane, and says nothing more.
#[test]
fn a_worker_is_filled_in_from_the_session_it_runs_in() {
    let task = pb::Task { id: id_bytes(Uuid::nil()), key: "ov-12".into(), ..Default::default() };
    let env = SessionEnv {
        claude_session: Some("76e86926".into()),
        cwd: Some("/Users/e-liang/Dev/overnight".into()),
        tmux_pane: Some("%0".into()),
        tmux: Some("/private/tmp/tmux-502/farcooler-01a0,13667,0".into()),
    };
    let (req, said) = worker_request(&task, &args(false), &env, "manager").unwrap();
    assert_eq!(said, None);
    assert_eq!(req.session_id.as_deref(), Some("76e86926"));
    assert_eq!(req.session_cwd.as_deref(), Some("/Users/e-liang/Dev/overnight"));
    assert_eq!((req.tmux_pane.as_deref(), req.tmux_socket.as_deref()), (Some("%0"), env.tmux.as_deref()));
    assert_eq!((req.harness.as_str(), req.agent_id.as_str(), req.actor.as_str()), ("claude", "a3fd8fceef581c787", "manager"));
    assert!(!req.end);
}

/// Without a Claude session to read, it's still recorded, and the command
/// says the runner can't follow it and how to end it by hand.
#[test]
fn a_worker_with_no_session_is_recorded_unobserved_and_says_so() {
    let task = pb::Task { key: "ov-12".into(), ..Default::default() };
    let (req, said) = worker_request(&task, &args(false), &SessionEnv::default(), "manager").unwrap();
    assert_eq!(req.session_id, None);
    assert_eq!(
        said.as_deref(),
        Some(
            "recorded, unobserved: CLAUDE_CODE_SESSION_ID isn't set here, so the runner can't find its session. \
             when it's finished, run `farcooler task worker ov-12 --subagent a3fd8fceef581c787 --harness claude --done`"
        )
    );
    let codex = WorkerArgs { harness: "Codex".into(), ..args(false) };
    let env = SessionEnv { claude_session: Some("not codex's".into()), ..Default::default() };
    let (req, said) = worker_request(&task, &codex, &env, "manager").unwrap();
    assert_eq!((req.harness.as_str(), req.session_id), ("codex", None), "a claude session is no codex thread");
    assert!(said.unwrap().contains("can't see a codex subagent working"));
    let (ended, said) = worker_request(&task, &args(true), &SessionEnv::default(), "manager").unwrap();
    assert_eq!((ended.end, ended.end_reason.as_str(), said), (true, "finished", None), "an end needs no session");
    assert!(worker_request(&task, &WorkerArgs { harness: "cursor".into(), ..args(false) }, &SessionEnv::default(), "m").is_err());
}

fn in_line(line: pb::TaskLine, position: u32, ahead: &[&str]) -> pb::Task {
    pb::Task {
        wait: Some(pb::TaskWait {
            kind: pb::TaskWaitKind::InLine as i32,
            line: line as i32,
            position,
            ahead: ahead.iter().map(|k| k.to_string()).collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Every start a row can say, in this CLI's lower case.
#[test]
fn each_start_reads_as_a_few_words() {
    let held = |kind: pb::TaskWaitKind, event: pb::TaskWaitEvent| pb::Task {
        wait: Some(pb::TaskWait { kind: kind as i32, event: event as i32, ..Default::default() }),
        ..Default::default()
    };
    let cases = [
        (in_line(pb::TaskLine::Agent, 1, &[]), Some("next to start")),
        (in_line(pb::TaskLine::Agent, 3, &["ov-1", "ov-2"]), Some("3rd in line")),
        (in_line(pb::TaskLine::Build, 1, &[]), Some("builds next")),
        (in_line(pb::TaskLine::Build, 2, &["ov-177"]), Some("2nd in line to build")),
        (held(pb::TaskWaitKind::After, pb::TaskWaitEvent::Release), Some("after the next release")),
        (held(pb::TaskWaitKind::After, pb::TaskWaitEvent::Recurrence), Some("if it happens again")),
        (held(pb::TaskWaitKind::After, pb::TaskWaitEvent::ClearBoard), Some("when nothing else is waiting")),
        (held(pb::TaskWaitKind::Parked, pb::TaskWaitEvent::Unspecified), Some("not planned")),
        (pb::Task { status: pb::TaskStatus::Todo as i32, ..Default::default() }, Some("ready to start")),
        (pb::Task { status: pb::TaskStatus::Backlog as i32, ..Default::default() }, None),
        (
            pb::Task { waiting_on: vec!["ov-191".into(), "ov-192".into()], ..in_line(pb::TaskLine::Agent, 1, &[]) },
            Some("waiting on ov-191 and ov-192"),
        ),
    ];
    for (task, said) in cases {
        assert_eq!(starts_word(&task).as_deref(), said, "{task:?}");
    }
    let until = pb::Task {
        wait: Some(pb::TaskWait { kind: pb::TaskWaitKind::Until as i32, until: 1_759_654_800_000, ..Default::default() }),
        ..Default::default()
    };
    assert_eq!(starts_word(&until).unwrap(), format!("starts {}", farcooler_core::local_time::moment(1_759_654_800_000)));
    for (n, said) in [(1, "1st"), (2, "2nd"), (3, "3rd"), (4, "4th"), (11, "11th"), (12, "12th"), (13, "13th"), (21, "21st"), (22, "22nd"), (111, "111th")] {
        assert_eq!(ordinal(n), said);
    }
}

/// `task show`: the start, who's ahead, and the subagents.
#[test]
fn show_says_when_it_starts_and_who_works_it() {
    let task = pb::Task {
        workers: vec![pb::TaskWorker {
            harness: "claude".into(),
            agent_id: "a1".into(),
            label: "ov-192 Mac polish".into(),
            started_at: 0,
            ended_at: 240_000,
            state: pb::TaskWorkerState::Finished as i32,
            ..Default::default()
        }],
        ..in_line(pb::TaskLine::Build, 3, &["ov-177", "ov-192"])
    };
    assert_eq!(starts_section(&task), "starts\n  3rd in line to build\n  after ov-177 and ov-192\n");
    assert_eq!(workers_section(&task, 480_000), "workers\n  claude subagent a1  finished 4m ago  ov-192 Mac polish\n");
    assert_eq!(starts_section(&pb::Task::default()), "", "nothing to say, no section");
}

/// Blocks by key, and a finished blocker said to be finished, but only by a
/// runner that sends `waiting_on`: from an older one every block still waits.
#[test]
fn a_block_names_its_key_and_whether_it_finished() {
    let (done, open) = (Uuid::now_v7(), Uuid::now_v7());
    let keys: HashMap<Uuid, String> = [(done, "ov-36".to_string()), (open, "ov-40".to_string())].into();
    let block = |on: Uuid, reason: &str| pb::TaskBlock { blocked_by: id_bytes(on), reason: reason.into(), ..Default::default() };
    let task = pb::Task { waiting_on: vec!["ov-40".into()], ..Default::default() };
    assert_eq!(block_line(&block(done, "follows Phase B"), &task, &keys, true), "  waited on ov-36, now finished  follows Phase B\n");
    assert_eq!(block_line(&block(open, ""), &task, &keys, true), "  waits on ov-40\n");
    assert_eq!(block_line(&block(done, ""), &task, &keys, false), "  waits on ov-36\n");
}

/// The list grows a column only when some row says when it starts.
#[test]
fn the_list_says_when_each_starts() {
    let row = |key: &str, task: pb::Task| pb::Task { key: key.into(), title: "t".into(), ..task };
    let tasks = [
        row("ov-1", in_line(pb::TaskLine::Agent, 1, &[])),
        row("ov-2", pb::Task { status: pb::TaskStatus::Backlog as i32, ..Default::default() }),
    ];
    let listed = super::super::render_list(&tasks, None, 0);
    let first = listed.lines().next().unwrap();
    assert!(first.contains("next to start  t"), "{listed}");
    let quiet = super::super::render_list(&tasks[1..], None, 0);
    assert!(quiet.lines().next().unwrap().ends_with("  t") && !quiet.contains("start"), "{quiet}");
}

/// Every word the three routes refuse with has a sentence here, in this
/// CLI's style, and a refusal carrying one says it.
#[test]
fn every_refusal_these_routes_send_has_a_sentence() {
    let words = [
        "line", "line_status", "other_board", "task_twice", "wait_status", "until", "event", "task_closed",
        "harness", "agent_id", "end_reason",
    ];
    for what in words {
        let said = said_here(what).unwrap_or_else(|| panic!("no sentence for {what}"));
        assert!(said.starts_with(char::is_lowercase) && !said.ends_with('.'), "{what}: {said}");
    }
    let err = ClientError::Daemon {
        code: pb::ErrorCode::InvalidArgument as i32,
        retryable: false,
        message: "invalid argument: line_status".into(),
        what: "line_status".into(),
    };
    assert_eq!(refused_here(err, "fallback").to_string(), said_here("line_status").unwrap());
}

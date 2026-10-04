//! The follower writing back to the store, on a scratch `$HOME` holding the
//! orchestrator's session and its subagents' transcripts, cut from real
//! records (`session_log/claude_workers_tests`' fixtures, ids synthetic).

use std::io::Write;
use std::sync::Arc;

use farcooler_protocol::v1 as pb;
use farcooler_store::models::{NoteKind, Task, TaskStatus};

use super::*;
use crate::service::Service;

const NOTIFICATIONS: &str = include_str!("../../../core/fixtures/session-logs/claude-subagent-notifications.jsonl");
const TRANSCRIPT: &str = include_str!("../../../core/fixtures/session-logs/claude-subagent-transcript.jsonl");
const SESSION: &str = "00000000-0000-4000-8000-0000000000aa";
const CWD: &str = "/Users/example/project";

struct Runner {
    _dir: tempfile::TempDir,
    home: PathBuf,
    svc: Arc<Service>,
    watcher: Arc<Watcher>,
    tasks: Vec<Task>,
    /// The clock the follower is told: the fixture's own.
    now: i64,
}

async fn runner(n: usize) -> Runner {
    let dir = tempfile::tempdir().unwrap();
    let (repository, first) = (Uuid::now_v7(), Uuid::now_v7());
    farcooler_store::testing::write_prefixless_board_at_schema_11(&dir.path().join("farcooler.db"), repository, first);
    let svc = Arc::new(Service::open_in(dir.path().to_path_buf()).await.unwrap());
    let main = svc.store.ensure_main_workspace(repository).unwrap().id;
    let tasks = (0..n).map(|i| svc.store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    let watcher = Watcher::new(svc.clone());
    let home = dir.path().join("home");
    let project = home.join(".claude/projects").join(farcooler_core::session_log::claude_slug(CWD));
    std::fs::create_dir_all(project.join(SESSION).join("subagents")).unwrap();
    watcher.worker_follow.lock().unwrap().under(home.clone());
    let now = farcooler_core::session_log::claude::timestamp_ms(NOTIFICATIONS.lines().next().unwrap()).unwrap();
    Runner { _dir: dir, home, svc, watcher, tasks, now }
}

fn agent(n: u32) -> String {
    format!("a{n:016}")
}

impl Runner {
    fn project(&self) -> PathBuf {
        self.home.join(".claude/projects").join(farcooler_core::session_log::claude_slug(CWD))
    }

    fn append(&self, path: PathBuf, lines: &[String]) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    fn session(&self, lines: &[&str]) {
        self.append(self.project().join(format!("{SESSION}.jsonl")), &lines.iter().map(|l| l.to_string()).collect::<Vec<_>>());
    }

    fn transcript(&self, agent: &str, lines: &[String]) {
        self.append(self.project().join(SESSION).join("subagents").join(format!("agent-{agent}.jsonl")), lines);
    }

    /// Record `agent` on `task`, as `task worker` does from the orchestrator's session.
    fn record(&self, task: &Task, agent: &str) {
        let record = WorkerRecord {
            harness: "claude".into(),
            agent_id: agent.into(),
            session_id: Some(SESSION.into()),
            session_cwd: Some(CWD.into()),
            orchestrator_terminal: None,
            label: Some("ov-12 polish".into()),
            model: None,
            linked_by: LinkedBy::Orchestrator,
        };
        self.svc.store.record_worker(task.id, &record, Actor::Manager).unwrap();
    }

    async fn pass(&mut self) {
        self.now += 10_000;
        self.watcher.follow_workers(self.now).await;
    }

    fn workers(&self, task: &Task) -> Vec<pb::TaskWorker> {
        crate::task_starts::pb_one(&self.svc, &self.svc.store.get_task(task.id).unwrap()).unwrap().workers
    }

    fn notes(&self, task: &Task) -> Vec<String> {
        self.svc.store.notes_for(task.id, Some(NoteKind::Worker)).unwrap().into_iter().map(|n| n.body).collect()
    }
}

fn note(n: usize) -> &'static str {
    NOTIFICATIONS.lines().nth(n).unwrap()
}

/// A launch of `agent_n` with `description`, as the two records a background
/// `Agent` call writes (the fixture's, renamed).
fn launch(description: &str, n: u32) -> [String; 2] {
    let rename = |line: &str| {
        line.replace("ov-12: polish the sidebar", description)
            .replace("a0000000000000001", &agent(n))
            .replace("toolu_000000000000000000000001", &format!("toolu_{n:024}"))
    };
    [rename(note(0)), rename(note(1))]
}

fn state(w: &pb::TaskWorker) -> pb::TaskWorkerState {
    pb::TaskWorkerState::try_from(w.state).unwrap()
}

/// What the whole chain says: a subagent recorded on a task is running while
/// the runner reads its session, and its notification ends it, in each of
/// the four ways, with a note on the card. Without the notification arm
/// (`claude::notified`) it stays running forever.
#[tokio::test]
async fn a_notification_ends_the_recorded_subagent_in_each_of_four_ways() {
    let mut r = runner(4).await;
    r.session(&[]);
    for (i, task) in r.tasks.clone().iter().enumerate() {
        r.record(task, &agent(i as u32 + 1));
    }
    r.pass().await;
    for task in &r.tasks {
        assert_eq!(state(&r.workers(task)[0]), pb::TaskWorkerState::Running);
    }
    r.session(&[note(2), note(3), note(4), note(5)]);
    r.pass().await;
    let states: Vec<_> = r.tasks.iter().map(|t| state(&r.workers(t)[0])).collect();
    use pb::TaskWorkerState::{Finished, Stopped};
    assert_eq!(states, [Finished, Stopped, Stopped, Stopped]);
    let said: Vec<_> = r.tasks.iter().map(|t| r.notes(t).pop().unwrap()).collect();
    assert_eq!(
        said,
        ["Claude subagent finished.", "Claude subagent failed.", "Claude subagent stopped.", "Claude subagent stopped."]
    );
    // The same notification again (its attachment, its user turn) changes nothing.
    let before = r.notes(&r.tasks[0]).len();
    r.session(&[note(7), note(8)]);
    r.pass().await;
    assert_eq!(r.notes(&r.tasks[0]).len(), before);
}

/// A message to a stopped subagent wakes the same row.
#[tokio::test]
async fn a_resume_reopens_the_subagent_with_a_note() {
    let mut r = runner(1).await;
    r.session(&[]);
    r.record(&r.tasks[0], &agent(1));
    r.pass().await;
    r.session(&[note(2)]);
    r.pass().await;
    assert_eq!(state(&r.workers(&r.tasks[0])[0]), pb::TaskWorkerState::Finished);
    r.session(&[note(9)]);
    r.pass().await;
    let workers = r.workers(&r.tasks[0]);
    assert_eq!((workers.len(), state(&workers[0])), (1, pb::TaskWorkerState::Running));
    assert_eq!(r.notes(&r.tasks[0]).last().unwrap(), "Claude subagent resumed: ov-12 polish");
}

/// A subagent writing again after its stop is working again, whoever woke it:
/// but a transcript read late, whose lines the stop came after, isn't.
#[tokio::test]
async fn new_transcript_lines_after_the_stop_reopen_it_and_old_ones_do_not() {
    let mut r = runner(1).await;
    r.session(&[]);
    r.record(&r.tasks[0], &agent(1));
    r.pass().await;
    r.session(&[note(2)]);
    r.pass().await;
    // September's lines, read in October: the stop came after them.
    let old: Vec<String> = TRANSCRIPT.lines().take(4).map(str::to_string).collect();
    r.transcript(&agent(1), &old);
    r.pass().await;
    assert_eq!(state(&r.workers(&r.tasks[0])[0]), pb::TaskWorkerState::Finished);
    // A line stamped after the stop.
    let later = farcooler_store::testing::now_millis() + 3_600_000;
    let stamp = |ms: i64| {
        let secs = ms / 1000;
        let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
        // Civil-from-days (Howard Hinnant), enough for a test's timestamp.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z", rem / 3600, rem % 3600 / 60, rem % 60)
    };
    let fresh = TRANSCRIPT.lines().nth(1).unwrap().replace("2026-09-06T07:35:35.395Z", &stamp(later));
    assert!(fresh.contains(&stamp(later)), "the fixture line carries the stamp it's replaced at");
    r.transcript(&agent(1), &[fresh]);
    r.pass().await;
    assert_eq!(state(&r.workers(&r.tasks[0])[0]), pb::TaskWorkerState::Running);
}

/// The link from a description: only a key that opens it. "After ov-92
/// lands" names a card the subagent isn't working.
#[tokio::test]
async fn a_description_links_only_on_a_leading_key() {
    let mut r = runner(2).await;
    let (key, other) = (r.tasks[0].key.clone(), r.tasks[1].key.clone());
    // A subagent recorded in the session teaches the runner where it lives.
    r.session(&[]);
    r.record(&r.tasks[1], &agent(99));
    r.pass().await;
    let lines: Vec<String> = [
        launch(&format!("after {key} lands, tidy up"), 11),
        launch(&format!("{key}: polish the sidebar"), 12),
    ]
    .into_iter()
    .flatten()
    .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    r.session(&refs);
    r.pass().await;
    let on = r.workers(&r.tasks[0]);
    assert_eq!(on.len(), 1, "only the leading key linked: {on:?}");
    assert_eq!(on[0].agent_id, agent(12));
    assert!(on[0].linked_by_description);
    assert_eq!(r.svc.store.get_task(r.tasks[0].id).unwrap().status, TaskStatus::InProgress, "a subagent at work started it");
    assert!(r.notes(&r.tasks[0])[0].starts_with("Claude subagent linked from its description"), "{:?}", r.notes(&r.tasks[0]));
    assert_eq!(r.workers(&r.tasks[1]).len(), 1, "{other} has only the one the orchestrator recorded");
}

/// A subagent the orchestrator recorded on a task isn't taken from it by its
/// description naming another.
#[tokio::test]
async fn an_explicit_record_is_not_overridden_by_a_description() {
    let mut r = runner(2).await;
    let key = r.tasks[0].key.clone();
    r.session(&[]);
    r.record(&r.tasks[1], &agent(12));
    r.pass().await;
    let lines = launch(&format!("{key}: polish"), 12);
    r.session(&[&lines[0], &lines[1]]);
    r.pass().await;
    assert!(r.workers(&r.tasks[0]).is_empty());
    assert_eq!(r.workers(&r.tasks[1]).len(), 1);
}

/// Last activity and what it's doing are the transcript's, and its spend is
/// filed to the task.
#[tokio::test]
async fn what_it_did_when_and_what_it_cost_are_read_from_its_transcript() {
    let mut r = runner(1).await;
    r.session(&[]);
    r.record(&r.tasks[0], &agent(1));
    r.pass().await;
    let lines: Vec<String> = TRANSCRIPT.lines().map(str::to_string).collect();
    r.transcript(&agent(1), &lines);
    r.pass().await;
    let worker = &r.workers(&r.tasks[0])[0];
    let latest = lines.iter().filter_map(|l| farcooler_core::session_log::claude::timestamp_ms(l)).max().unwrap();
    assert_eq!(worker.last_activity_at, latest, "the transcript's time, not October's");
    assert!(!worker.doing.is_empty(), "what it's doing now");
    assert_eq!(worker.model, "claude-opus-5");
    let (usage, _) = r.svc.store.task_usage(r.tasks[0].id).unwrap();
    assert_eq!(usage.subagent_runs, 1);
    assert!(usage.tokens.output > 0, "{usage:?}");
}

/// A session the runner can't find is one it can't say anything about.
#[tokio::test]
async fn a_session_it_cannot_find_is_unobserved() {
    let mut r = runner(1).await;
    let record = WorkerRecord {
        harness: "claude".into(),
        agent_id: agent(1),
        session_id: Some("00000000-0000-4000-8000-0000000000bb".into()),
        session_cwd: Some(CWD.into()),
        orchestrator_terminal: None,
        label: None,
        model: None,
        linked_by: LinkedBy::Orchestrator,
    };
    r.svc.store.record_worker(r.tasks[0].id, &record, Actor::Manager).unwrap();
    r.pass().await;
    assert_eq!(state(&r.workers(&r.tasks[0])[0]), pb::TaskWorkerState::Unobserved);
}

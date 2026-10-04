//! The follower on scratch files: a session in a project directory, its
//! subagents' own transcripts beside it, and the claude records of
//! `fixtures/session-logs/` (notifications and a launch, and a subagent's
//! own transcript).

use std::io::Write;
use std::path::PathBuf;

use super::claude::timestamp_ms;
use super::worker_follow::{Seen, Wanted, WorkerFollow};
use super::{SubagentStatus, claude_slug};

const NOTIFICATIONS: &str = include_str!("../../fixtures/session-logs/claude-subagent-notifications.jsonl");
const TRANSCRIPT: &str = include_str!("../../fixtures/session-logs/claude-subagent-transcript.jsonl");

const SESSION: &str = "00000000-0000-4000-8000-0000000000aa";
const CWD: &str = "/Users/example/project";

fn note(n: usize) -> &'static str {
    NOTIFICATIONS.lines().nth(n).expect("the fixture has that many lines")
}

fn agent(n: u32) -> String {
    format!("a{n:016}")
}

struct Home {
    home: PathBuf,
    project: PathBuf,
    follow: WorkerFollow,
}

impl Home {
    fn new(tag: &str) -> Home {
        let home = std::env::temp_dir().join(format!("farcooler-worker-follow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let project = home.join(".claude/projects").join(claude_slug(CWD));
        std::fs::create_dir_all(project.join(SESSION).join("subagents")).unwrap();
        Home { follow: WorkerFollow::new(home.clone()), home, project }
    }

    fn session_file(&self) -> PathBuf {
        self.project.join(format!("{SESSION}.jsonl"))
    }

    fn append(&self, path: &PathBuf, lines: &[&str]) {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    fn session(&self, lines: &[&str]) {
        self.append(&self.session_file(), lines);
    }

    fn transcript(&self, agent: &str, lines: &[&str]) {
        self.append(&self.project.join(SESSION).join("subagents").join(format!("agent-{agent}.jsonl")), lines);
    }

    fn pass(&mut self, agents: &[String], now_ms: i64) -> Vec<Seen> {
        let want = Wanted { session_id: SESSION.into(), cwd: CWD.into(), agents: agents.to_vec() };
        self.follow.follow(&[want], now_ms).seen
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The fixture's own clock: the launch's time.
fn then() -> i64 {
    timestamp_ms(note(0)).unwrap()
}

/// A subagent launched after the session began being followed is seen with
/// the description its call carried, which is what a leading task key is
/// read from.
#[test]
fn a_new_spawn_is_seen_with_its_description() {
    let mut h = Home::new("spawn");
    h.session(&[]);
    assert!(h.pass(&[], then()).is_empty());
    h.session(&[note(0), note(1)]);
    assert_eq!(
        h.pass(&[], then() + 10_000),
        [Seen::Spawned {
            session: SESSION.into(),
            agent_id: agent(1),
            description: "ov-12: polish the sidebar".into(),
            at_ms: timestamp_ms(note(1)),
        }]
    );
}

/// A spawn already in the file when it was first read is history: linking it
/// would move a card to In Progress for work from a day ago.
#[test]
fn a_spawn_from_before_the_session_was_followed_is_history() {
    let mut h = Home::new("history");
    h.session(&[note(0), note(1)]);
    assert!(h.pass(&[], then() + 24 * 3_600_000).is_empty());
}

/// All four statuses, each naming its agent.
#[test]
fn every_status_a_notification_carries_is_seen_for_a_wanted_agent() {
    let mut h = Home::new("statuses");
    h.session(&[]);
    let want: Vec<String> = (1..=4).map(agent).collect();
    h.pass(&want, then());
    h.session(&[note(2), note(3), note(4), note(5)]);
    let ended: Vec<(String, SubagentStatus)> = h
        .pass(&want, then() + 1)
        .into_iter()
        .filter_map(|s| match s {
            Seen::Ended { agent_id, status, .. } => Some((agent_id, status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        ended,
        [
            (agent(1), SubagentStatus::Completed),
            (agent(2), SubagentStatus::Failed),
            (agent(3), SubagentStatus::Killed),
            (agent(4), SubagentStatus::Stopped),
        ]
    );
}

/// Written three times, said once per pass.
#[test]
fn the_three_copies_of_a_notification_are_one_stop() {
    let mut h = Home::new("copies");
    h.session(&[]);
    h.pass(&[agent(1)], then());
    h.session(&[note(2), note(7), note(8)]);
    let seen = h.pass(&[agent(1)], then() + 1);
    assert_eq!(seen.len(), 1, "{seen:?}");
}

/// A notification for a background shell, a monitor, or a subagent nobody
/// recorded isn't a recorded subagent's stop.
#[test]
fn a_stop_for_an_agent_nobody_recorded_is_not_reported() {
    let mut h = Home::new("others");
    h.session(&[]);
    h.pass(&[agent(9)], then());
    h.session(&[note(2), note(3), note(6)]);
    assert!(h.pass(&[agent(9)], then() + 1).is_empty());
}

#[test]
fn a_resume_is_seen_for_a_wanted_agent() {
    let mut h = Home::new("resume");
    h.session(&[]);
    h.pass(&[agent(1)], then());
    h.session(&[note(9)]);
    assert_eq!(h.pass(&[agent(1)], then() + 1), [Seen::Resumed { session: SESSION.into(), agent_id: agent(1) }]);
}

/// A file read from its start says where each subagent stands now: it
/// stopped and was resumed is one working.
#[test]
fn a_first_read_says_only_where_each_agent_stands() {
    let mut h = Home::new("catch-up");
    h.session(&[note(2), note(9)]);
    assert_eq!(h.pass(&[agent(1)], then()), [Seen::Resumed { session: SESSION.into(), agent_id: agent(1) }]);
}

/// The session is found by its id alone: the cwd the orchestrator recorded
/// is wherever it had moved to, not where the session began.
#[test]
fn a_session_is_found_in_whichever_project_holds_it() {
    let mut h = Home::new("elsewhere");
    h.session(&[]);
    h.append(&h.session_file(), &[]);
    let moved = Wanted { session_id: SESSION.into(), cwd: "/Users/example/elsewhere".into(), agents: vec![agent(1)] };
    h.follow.follow(std::slice::from_ref(&moved), then());
    h.session(&[note(2)]);
    let seen = h.follow.follow(std::slice::from_ref(&moved), then() + 1).seen;
    assert_eq!(seen.len(), 1, "{seen:?}");
}

/// The time a subagent last moved is its transcript's own: replaying lines
/// from September on a clock set to October says September.
#[test]
fn last_activity_is_the_transcripts_time_and_not_the_clock() {
    let mut h = Home::new("clock");
    h.session(&[]);
    h.pass(&[agent(1)], then());
    let lines: Vec<&str> = TRANSCRIPT.lines().collect();
    h.transcript(&agent(1), &lines[..3]);
    let latest = |upto: usize| lines[..upto].iter().filter_map(|l| timestamp_ms(l)).max();
    let first = h.pass(&[agent(1)], then() + 365 * 24 * 3_600_000);
    let at = |seen: &[Seen]| match seen {
        [Seen::Active { at_ms, .. }] => *at_ms,
        other => panic!("{other:?}"),
    };
    assert_eq!(at(&first), latest(3));
    // A pass that reads nothing new reports nothing; one that reads more
    // reports the later time. The clock never enters into it.
    assert!(h.pass(&[agent(1)], then() + 400 * 24 * 3_600_000).is_empty());
    h.transcript(&agent(1), &lines[3..8]);
    let second = h.pass(&[agent(1)], then() + 401 * 24 * 3_600_000);
    assert_eq!(at(&second), latest(8));
    assert!(latest(8) > latest(3));
}

/// What a subagent is doing now is its last tool call, and its spend is read
/// from the same lines.
#[test]
fn what_a_subagent_is_doing_and_what_it_spent_come_from_its_transcript() {
    let mut h = Home::new("doing");
    h.session(&[]);
    let want = Wanted { session_id: SESSION.into(), cwd: CWD.into(), agents: vec![agent(1)] };
    h.follow.follow(std::slice::from_ref(&want), then());
    let lines: Vec<&str> = TRANSCRIPT.lines().collect();
    h.transcript(&agent(1), &lines);
    let followed = h.follow.follow(std::slice::from_ref(&want), then() + 1);
    let doing = followed.seen.iter().find_map(|s| match s {
        Seen::Active { doing, model, .. } => Some((doing.clone(), model.clone())),
        _ => None,
    });
    let (doing, model) = doing.expect("it was seen working");
    assert!(doing.is_some(), "a tool call among its lines");
    assert_eq!(model.as_deref(), Some("claude-opus-5"));
    assert_eq!(followed.spend.len(), 1);
    assert_eq!(followed.spend[0].1.key, format!("claude-log:agent:{}", agent(1)));
}

/// A foreground run writes its launch and its end in one result line. A
/// subagent recorded after that line was read, in a session already being
/// followed for another, is ended the pass it's recorded, not left running.
fn foreground(n: u32) -> [String; 2] {
    let rename = |line: &str| {
        line.replace("a0000000000000001", &agent(n))
            .replace("toolu_000000000000000000000001", &format!("toolu_{n:024}"))
    };
    [rename(note(0)), rename(note(1)).replace("async_launched", "completed")]
}

#[test]
fn an_end_read_before_the_agent_was_wanted_is_told_when_it_is() {
    let mut h = Home::new("late");
    h.session(&[]);
    h.pass(&[agent(1)], then());
    let lines = foreground(77);
    h.session(&[&lines[0], &lines[1]]);
    let early = h.pass(&[agent(1)], then() + 1);
    assert!(!early.iter().any(|s| matches!(s, Seen::Ended { .. })), "not wanted yet: {early:?}");
    assert_eq!(
        h.pass(&[agent(1), agent(77)], then() + 2),
        [Seen::Ended { session: SESSION.into(), agent_id: agent(77), status: SubagentStatus::Completed }]
    );
    assert!(h.pass(&[agent(1), agent(77)], then() + 3).is_empty(), "said once");
}

/// A notification with a long `<result>`, and a launch with a long prompt:
/// both are lines the ordinary 64 KiB cap skips whole.
#[test]
fn a_line_over_64_kib_still_ends_an_agent_and_still_links() {
    let mut h = Home::new("long");
    h.session(&[]);
    h.pass(&[agent(1)], then());
    let long = "x".repeat(100 * 1024);
    let notice = note(2).replace("<result>removed</result>", &format!("<result>{long}</result>"));
    let launch = foreground(5);
    let spawn = launch[0].replace("\"prompt\":\"removed\"", &format!("\"prompt\":\"{long}\""));
    let launched = launch[1].replace("completed", "async_launched");
    assert!(notice.len() > 100 * 1024 && spawn.len() > 100 * 1024);
    h.session(&[&notice, &spawn, &launched]);
    let seen = h.pass(&[agent(1)], then() + 1);
    assert!(seen.iter().any(|s| matches!(s, Seen::Ended { agent_id, .. } if agent_id == &agent(1))), "{seen:?}");
    assert!(seen.iter().any(|s| matches!(s, Seen::Spawned { agent_id, .. } if agent_id == &agent(5))), "{seen:?}");
}

/// A tool result with an `agentId` in it is a subagent's only on the result
/// of an `Agent` call this follower saw made.
#[test]
fn a_result_for_a_call_never_seen_is_not_believed() {
    let mut h = Home::new("trust");
    h.session(&[]);
    h.pass(&[agent(6)], then());
    let lines = foreground(6);
    h.session(&[&lines[1]]);
    assert!(h.pass(&[agent(6)], then() + 1).is_empty());
    let async_only = lines[1].replace("completed", "async_launched");
    h.session(&[&async_only]);
    assert!(h.pass(&[agent(6)], then() + 2).is_empty(), "no spawn, no link");
}

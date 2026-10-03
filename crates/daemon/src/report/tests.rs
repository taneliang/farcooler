//! The report over a seeded board whose every event has a known time, with
//! every number worked out by hand in the comments beside it.
//!
//! Hours are the unit: the period is hour 100 to hour 200, and the clock
//! reads hour 250.

use farcooler_store::models::{AcceptanceItem, Actor, NoteKind, Task, TaskNote, TaskStatus};
use serde_json::json;
use uuid::Uuid;

use super::*;

const H: i64 = 3_600_000;
const PERIOD: Period = Period { since: 100 * H, until: 200 * H };
const NOW: i64 = 250 * H;

struct Seed {
    facts: TaskFacts,
}

fn task(key: &str, title: &str, created: i64) -> Seed {
    let task = Task {
        id: Uuid::now_v7(),
        key: key.into(),
        repository_id: Uuid::nil(),
        workspace_id: Uuid::nil(),
        title: title.into(),
        status: TaskStatus::Backlog,
        status_since: created * H,
        intent: String::new(),
        acceptance: Vec::new(),
        constraints: Vec::new(),
        worktree_id: None,
        labels: Vec::new(),
        resource_version: 1,
        created_at: created * H,
        updated_at: created * H,
    };
    let mut seed = Seed {
        facts: TaskFacts { task, workspace: "Main".into(), repository: "overnight".into(), notes: Vec::new() },
    };
    seed.note(NoteKind::Created, Actor::Manager, created, json!({}));
    seed
}

impl Seed {
    fn note(&mut self, kind: NoteKind, actor: Actor, at: i64, extra: serde_json::Value) -> Uuid {
        let id = Uuid::now_v7();
        self.facts.notes.push(TaskNote {
            id,
            task_id: self.facts.task.id,
            kind,
            actor,
            at: at * H,
            body: String::new(),
            extra,
            supersedes: None,
        });
        id
    }

    /// Moves to `to` at hour `at`, from wherever the last move left it.
    fn to(mut self, at: i64, to: TaskStatus) -> Seed {
        let from = self.facts.task.status;
        self.note(NoteKind::StatusChange, Actor::Manager, at, json!({ "from": from.as_str(), "to": to.as_str() }));
        self.facts.task.status = to;
        self.facts.task.status_since = at * H;
        self
    }

    fn ask(mut self, at: i64) -> Seed {
        self.note(NoteKind::Question, Actor::Manager, at, json!({}));
        self
    }

    fn answer(mut self, at: i64, by: Actor) -> Seed {
        self.note(NoteKind::Answer, by, at, json!({}));
        self
    }

    fn decide(mut self, at: i64) -> Seed {
        self.note(NoteKind::Decision, Actor::Manager, at, json!({}));
        self
    }

    fn lines(mut self, met: &[bool]) -> Seed {
        self.facts.task.acceptance =
            met.iter().map(|&met| AcceptanceItem { id: Uuid::now_v7(), text: "a line".into(), met }).collect();
        self
    }

    fn labels(mut self, labels: &[&str]) -> Seed {
        self.facts.task.labels = labels.iter().map(|l| l.to_string()).collect();
        self
    }

    fn in_workspace(mut self, workspace: &str, repository: &str) -> Seed {
        self.facts.workspace = workspace.into();
        self.facts.repository = repository.into();
        self
    }
}

use TaskStatus::*;

/// The board. Hour by hour, against the period 100 to 200:
///
/// - `ov-1`, filed at 90, before the period: in progress at 95, in review at
///   120, back to in progress at 125 (a fix round), in review at 130, done
///   at 140. 50 hours to done. A decision recorded at 135. Two lines, both
///   met.
/// - `ov-2`, filed at 110: in progress at 112, asks at 113 and waits in
///   Needs Decision until a person answers at 116, done at 130. 20 hours.
///   Three lines, two met.
/// - `bil-1`, in another repository's workspace, filed at 150: in progress
///   at 160, done at 180, reopened at 190, done again at 195. 45 hours. No
///   lines.
/// - `ov-3`, filed at 105, canceled at 150.
/// - `ov-4`, all before the period: filed at 10, a question at 40 answered
///   at 99, a decision at 60, done at 50.
/// - `ov-5`, filed at 180, asks at 190 and is still waiting.
/// - `ov-6`, filed at exactly 100 (inside), in progress at 101, done at
///   exactly 200 (outside: the period is half open).
fn board() -> Inputs {
    let seeds = [
        task("ov-1", "Mac: the jumpbar jumps anywhere", 90)
            .labels(&["found"])
            .lines(&[true, true])
            .to(95, InProgress)
            .to(120, InReview)
            .to(125, InProgress)
            .to(130, InReview)
            .decide(135)
            .to(140, Done),
        task("ov-2", "Daemon: answers reach agents", 110)
            .labels(&["found", "review"])
            .lines(&[true, true, false])
            .to(112, InProgress)
            .ask(113)
            .to(113, NeedsDecision)
            .answer(116, Actor::User)
            .to(116, InProgress)
            .to(130, Done),
        task("bil-1", "CLI: report", 150)
            .in_workspace("Billing", "api")
            .to(160, InProgress)
            .to(180, Done)
            .to(190, InProgress)
            .to(195, Done),
        task("ov-3", "Relay: rollup", 105).to(150, Cancelled),
        task("ov-4", "an old title with no area", 10)
            .to(20, InProgress)
            .ask(40)
            .to(50, Done)
            .decide(60)
            .answer(99, Actor::User),
        task("ov-5", "Phones: a question still open", 180).ask(190).to(190, NeedsDecision),
        task("ov-6", "Watch: on the edge", 100).to(101, InProgress).to(200, Done),
    ];
    Inputs { tasks: seeds.into_iter().map(|s| s.facts).collect(), ..Inputs::default() }
}

fn status_time<'a>(t: &'a Tally, status: &str) -> &'a StatusTime {
    t.time_in_status.iter().find(|s| s.status == status).unwrap_or_else(|| panic!("no {status}: {t:#?}"))
}

#[test]
fn counts_what_moved_inside_the_period() {
    let t = compute(&board(), PERIOD, NOW).totals;
    // Filed inside: ov-2, bil-1, ov-3, ov-5 and ov-6 (at exactly `since`).
    assert_eq!(t.created, 5);
    // ov-1, ov-2 and bil-1. Not ov-6, done at exactly `until`; not ov-4.
    assert_eq!(t.completed, 3);
    assert_eq!(t.canceled, 1);
    assert_eq!(t.reopened, 1, "bil-1");
    assert_eq!(t.fix_rounds, 1, "ov-1");
}

#[test]
fn time_to_done_is_filed_to_done() {
    let t = compute(&board(), PERIOD, NOW).totals;
    // 20, 45, 50: the middle one, and the third of three for p90.
    assert_eq!(t.time_to_done, Some(Spread { count: 3, median_ms: 45 * H, p90_ms: 50 * H }));
}

#[test]
fn time_in_each_status_is_clipped_to_the_period() {
    let t = compute(&board(), PERIOD, NOW).totals;
    // ov-2 2, bil-1 10, ov-3 45, ov-5 10, ov-6 1.
    assert_eq!(status_time(&t, "backlog"), &StatusTime { status: "backlog".into(), total_ms: 68 * H, tasks: 5, median_ms: 10 * H });
    // ov-1 20 (from 100, not 95) + 5, ov-2 1 + 14, bil-1 20 + 5, ov-6 99.
    assert_eq!(status_time(&t, "in_progress").total_ms, 164 * H);
    assert_eq!(status_time(&t, "in_progress").median_ms, 25 * H, "the mean of 25 and 25");
    // ov-2 3, ov-5 10 (to the end of the period): the mean of the two.
    assert_eq!(status_time(&t, "needs_decision").median_ms, 13 * H / 2);
    assert_eq!(status_time(&t, "in_review").total_ms, 15 * H);
    assert!(t.time_in_status.iter().all(|s| s.status != "todo"), "nothing sat in To Do");
    let order: Vec<_> = t.time_in_status.iter().map(|s| s.status.as_str()).collect();
    assert_eq!(order, ["backlog", "needs_decision", "in_progress", "in_review"], "board order");
}

#[test]
fn a_question_waits_until_its_answer() {
    let d = compute(&board(), PERIOD, NOW).totals.decisions;
    // ov-2 at 113 and ov-5 at 190. ov-4's was before the period.
    assert_eq!(d.asked, 2);
    // ov-2's, by a person, 3 hours later. ov-4's answer came at 99: outside.
    assert_eq!(d.answered, 1);
    assert_eq!(d.answered_by_you, 1);
    assert_eq!(d.answered_by_orchestrator, 0);
    assert_eq!(d.unanswered, 1, "ov-5");
    assert_eq!(d.latency, Some(Spread { count: 1, median_ms: 3 * H, p90_ms: 3 * H }));
    assert_eq!(d.latency_you, d.latency);
    assert_eq!(d.recorded, 1, "ov-1's at 135; ov-4's at 60 is outside");
}

#[test]
fn an_answer_answers_every_open_question_before_it() {
    let mut seed = task("ov-9", "Mac: two questions", 110);
    seed.note(NoteKind::Question, Actor::Manager, 120, json!({}));
    seed.note(NoteKind::Question, Actor::Manager, 126, json!({}));
    let seed = seed.answer(130, Actor::Manager).answer(140, Actor::User);
    let inputs = Inputs { tasks: vec![seed.facts], ..Inputs::default() };
    let d = compute(&inputs, PERIOD, NOW).totals.decisions;
    // Both by the orchestrator's answer, at 10 and 4 hours; the later
    // answer had nothing left to answer.
    assert_eq!((d.asked, d.answered, d.answered_by_orchestrator, d.answered_by_you), (2, 2, 2, 0));
    assert_eq!(d.latency, Some(Spread { count: 2, median_ms: 7 * H, p90_ms: 10 * H }));
    assert_eq!(d.latency_you, None);
}

#[test]
fn a_withdrawn_question_is_not_counted() {
    let mut seed = task("ov-9", "Mac: asked twice", 110);
    let first = seed.note(NoteKind::Question, Actor::Manager, 120, json!({}));
    let second = seed.note(NoteKind::Question, Actor::Manager, 121, json!({}));
    seed.facts.notes.iter_mut().find(|n| n.id == second).unwrap().supersedes = Some(first);
    let inputs = Inputs { tasks: vec![seed.facts], ..Inputs::default() };
    let d = compute(&inputs, PERIOD, NOW).totals.decisions;
    assert_eq!((d.asked, d.unanswered), (1, 1));
}

#[test]
fn needs_you_counts_spells_in_needs_decision_and_review() {
    let n = compute(&board(), PERIOD, NOW).totals.needs_you;
    // ov-1's two reviews, ov-2's decision, ov-5's decision.
    assert_eq!((n.times, n.decisions, n.reviews), (4, 2, 2));
    // 5 and 10 hours of review, 3 of decision. ov-5 is still waiting.
    assert_eq!(n.cleared, 3);
    assert_eq!(n.waiting, 1);
    assert_eq!(n.time_to_clear, Some(Spread { count: 3, median_ms: 5 * H, p90_ms: 10 * H }));
}

#[test]
fn acceptance_reads_the_tasks_completed_in_the_period() {
    let a = compute(&board(), PERIOD, NOW).totals.acceptance;
    // ov-1 2 of 2, ov-2 2 of 3, bil-1 none.
    assert_eq!(a, Acceptance { met: 4, total: 5, tasks_fully_met: 1, tasks_without_lines: 1 });
}

#[test]
fn groups_split_by_workspace_area_and_label() {
    let r = compute(&board(), PERIOD, NOW);
    let workspaces: Vec<_> =
        r.by_workspace.iter().map(|g| (g.name.as_str(), g.repository.as_deref(), g.tally.completed)).collect();
    assert_eq!(workspaces, [("Main", Some("overnight"), 2), ("Billing", Some("api"), 1)]);
    let repositories: Vec<_> = r.by_repository.iter().map(|g| (g.name.as_str(), g.tally.created)).collect();
    assert_eq!(repositories, [("overnight", 4), ("api", 1)]);
    // Most completed first, then by name. ov-4 has no area and nothing in
    // the period, so "Other" is left out.
    let areas: Vec<_> = r.by_area.iter().map(|g| (g.name.as_str(), g.tally.completed)).collect();
    assert_eq!(areas, [("CLI", 1), ("Daemon", 1), ("Mac", 1), ("Phones", 0), ("Relay", 0), ("Watch", 0)]);
    let labels: Vec<_> = r.by_label.iter().map(|g| (g.name.as_str(), g.tally.completed)).collect();
    assert_eq!(labels, [("found", 2), ("review", 1)]);
    // A group is the same arithmetic as the totals.
    let mac = r.by_area.iter().find(|g| g.name == "Mac").unwrap();
    assert_eq!(mac.tally.time_to_done.map(|s| s.median_ms), Some(50 * H));
}

#[test]
fn notable_names_the_slowest_and_the_longest_waits() {
    let n = compute(&board(), PERIOD, NOW).notable;
    let slowest: Vec<_> = n.slowest.iter().map(|t| (t.key.as_str(), t.ms)).collect();
    assert_eq!(slowest, [("ov-1", Some(50 * H)), ("bil-1", Some(45 * H)), ("ov-2", Some(20 * H))]);
    // ov-5's open question counts to the end of the period, not to now.
    let waits: Vec<_> = n.longest_waits.iter().map(|w| (w.key.as_str(), w.kind.as_str(), w.ms, w.open)).collect();
    assert_eq!(
        waits,
        [
            ("ov-1", "review", 10 * H, false),
            ("ov-5", "question", 10 * H, true),
            ("ov-1", "review", 5 * H, false),
            ("ov-2", "question", 3 * H, false),
        ]
    );
    assert_eq!(n.reopened.iter().map(|t| (t.key.as_str(), t.count)).collect::<Vec<_>>(), [("bil-1", Some(1))]);
    assert_eq!(n.most_fix_rounds.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["ov-1"]);
    assert_eq!(n.canceled.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["ov-3"]);
}

#[test]
fn the_period_is_half_open() {
    // From 140, the moment ov-1 was done: it counts. To 140: it doesn't.
    let from = compute(&board(), Period { since: 140 * H, until: 300 * H }, NOW).totals;
    let to = compute(&board(), Period { since: 0, until: 140 * H }, NOW).totals;
    let has = |r: &Report, key: &str| r.notable.slowest.iter().any(|t| t.key == key);
    assert!(has(&compute(&board(), Period { since: 140 * H, until: 300 * H }, NOW), "ov-1"));
    assert!(!has(&compute(&board(), Period { since: 0, until: 140 * H }, NOW), "ov-1"));
    // From 140 to 300: ov-1, bil-1 and ov-6. From 0 to 140: ov-4 and ov-2.
    assert_eq!((from.completed, to.completed), (3, 2));
}

#[test]
fn done_then_reopened_inside_the_period_is_not_done() {
    // bil-1 between 170 and 192: done at 180, back in progress at 190.
    let t = compute(&board(), Period { since: 170 * H, until: 192 * H }, NOW).totals;
    assert_eq!((t.completed, t.reopened), (0, 1));
    // Up to 185 it was done, and the reopening is after the period.
    let t = compute(&board(), Period { since: 170 * H, until: 185 * H }, NOW).totals;
    assert_eq!((t.completed, t.reopened), (1, 0));
}

#[test]
fn time_after_now_is_not_counted() {
    // A period that runs past the clock: ov-5 has waited from 190 to now
    // (250), not to 300.
    let t = compute(&board(), Period { since: 100 * H, until: 300 * H }, NOW).totals;
    assert_eq!(status_time(&t, "needs_decision").total_ms, 3 * H + 60 * H);
}

#[test]
fn an_empty_period_is_all_zeros() {
    let r = compute(&board(), Period { since: 0, until: 5 * H }, NOW);
    assert_eq!(r.totals, Tally::default());
    assert!(r.by_workspace.is_empty() && r.by_area.is_empty() && r.by_label.is_empty() && r.by_repository.is_empty());
    assert_eq!(r.notable, Notable::default());
    let none = compute(&Inputs::default(), PERIOD, NOW);
    assert_eq!(none.totals, Tally::default());
    assert_eq!(none.scope.kind, "runner");
}

#[test]
fn usage_slots_in_where_a_task_has_it() {
    let mut inputs = board();
    let ov1 = inputs.tasks[0].task.id;
    let ov2 = inputs.tasks[1].task.id;
    inputs.usage.insert(ov1, Usage { input_tokens: Some(1_000), agent_ms: Some(2 * H), ..Usage::default() });
    inputs.usage.insert(ov2, Usage { input_tokens: Some(500), output_tokens: Some(70), ..Usage::default() });
    let r = compute(&inputs, PERIOD, NOW);
    assert_eq!(
        r.totals.usage,
        Some(Usage { input_tokens: Some(1_500), output_tokens: Some(70), agent_ms: Some(2 * H), ..Usage::default() })
    );
    let billing = r.by_workspace.iter().find(|g| g.name == "Billing").unwrap();
    assert_eq!(billing.tally.usage, None, "none of its tasks has any");
    // And today, with none recorded, the key is there and null.
    let json = serde_json::to_value(compute(&board(), PERIOD, NOW)).unwrap();
    assert_eq!(json["totals"]["usage"], serde_json::Value::Null);
    assert_eq!(json["by_workspace"][0]["usage"], serde_json::Value::Null, "flattened into the group");
}

#[test]
fn the_json_reads_back_as_the_same_report() {
    let r = compute(&board(), PERIOD, NOW);
    let back: Report = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(back, r);
    assert_eq!(serde_json::to_value(&r).unwrap()["schema"], SCHEMA);
}

#[test]
fn median_and_p90() {
    assert_eq!((median(&[]), p90(&[])), (0, 0));
    assert_eq!((median(&[7]), p90(&[7])), (7, 7));
    assert_eq!(median(&[4, 1, 3]), 3);
    assert_eq!(median(&[1, 2, 3, 10]), 2, "(2 + 3) / 2, rounded down");
    // Ten values: the ninth. Eleven: the tenth (ceil 9.9).
    let ten: Vec<i64> = (1..=10).collect();
    assert_eq!(p90(&ten), 9);
    let eleven: Vec<i64> = (1..=11).rev().collect();
    assert_eq!(p90(&eleven), 10);
}

#[test]
fn an_area_is_a_short_name_before_a_colon() {
    assert_eq!(area_of("Mac: the jumpbar jumps anywhere"), Some("Mac"));
    assert_eq!(area_of("iOS: widgets clear a stale failed mark"), Some("iOS"));
    assert_eq!(area_of("Planning: reporting on what agents achieved"), Some("Planning"));
    assert_eq!(area_of("Relays: a negation: refused"), Some("Relays"));
    assert_eq!(area_of("no colon here"), None);
    assert_eq!(area_of("Mac:"), None, "nothing after it");
    assert_eq!(area_of("the daemon refuses this: it should not"), None, "a sentence, not an area");
    assert_eq!(area_of("ov-12: fix"), None, "a key, not an area");
    assert_eq!(area_of("2026: plans"), None);
}

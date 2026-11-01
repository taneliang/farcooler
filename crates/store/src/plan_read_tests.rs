//! The plan read (ov-872): a ticket's spend from the agents that worked it, its
//! fix rounds and age, a landing's words, and a theme's last move.

use super::*;

use farcooler_core::usage::TokenCounts;

use crate::agents::{AgentPlace, AgentRole, NewAgent};
use crate::landings::LandingUpdate;
use crate::models::{Actor, Task};
use crate::plan::NewTheme;
use crate::rulings::NewRuling;
use crate::ticket_lines::{LineUpdate, NewTicketLine};
use crate::ticket_work::{LinkedBy, WorkRole};
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("Plan: task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn subagent(store: &Store, main: Uuid, id: &str, harness: &str, role: AgentRole) -> Uuid {
    let new = NewAgent {
        role,
        place: AgentPlace::Subagent { id: id.into(), parent: None },
        harness: harness.into(),
        session_id: None,
        session_cwd: None,
        model: None,
        title: None,
    };
    store.record_agent(main, &new).unwrap().id
}

fn works(store: &Store, agent: Uuid, task: Uuid, role: WorkRole) {
    store.start_work(agent, task, role, LinkedBy::Orchestrator, Actor::Manager).unwrap();
}

fn on_line(store: &Store, task: Uuid) {
    store.put_on_line(task, &NewTicketLine::default(), Actor::Manager).unwrap();
}

fn to(store: &Store, task: Uuid, stage: Stage) {
    store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager).unwrap();
}

fn run_turn(store: &Store, agent: &str, tokens: u64, task_id: Option<Uuid>) {
    store
        .record_turn(&NewTurn {
            key: format!("claude-log:agent:{agent}"),
            terminal_id: None,
            worktree_id: None,
            repository_id: None,
            workspace_id: None,
            task_id,
            harness: "claude".into(),
            surface: Surface::Terminal,
            started_at: None,
            ended_at: 1000,
            active_ms: None,
            usage: "reported",
            models: vec![TurnModel::priced(
                Some("claude-opus-5".into()),
                TokenCounts { input: tokens, output: tokens / 2, cache_read: 0, cache_write: 0, cache_write_1h: 0, fast: false },
                Some(2_000),
            )],
            kind: TurnKind::Subagent,
        })
        .unwrap();
}

fn line(plan: &Plan, task: Uuid) -> &LineView {
    plan.lines.iter().find(|l| l.line.task_id == task).unwrap()
}

/// A ticket's spend is its agents' runs and nothing else's; an agent the
/// runner has read no turn of is counted apart, never as zero.
#[test]
fn a_ticket_s_spend_is_its_own_agents_runs() {
    let (store, main, t) = board(1);
    on_line(&store, t[0].id);
    let a1 = subagent(&store, main, "a1", "claude", AgentRole::Builder);
    let a2 = subagent(&store, main, "a2", "claude", AgentRole::Reviewer);
    let c1 = subagent(&store, main, "c1", "codex", AgentRole::Builder);
    works(&store, a1, t[0].id, WorkRole::Build);
    works(&store, a2, t[0].id, WorkRole::Review);
    works(&store, c1, t[0].id, WorkRole::Fix);
    run_turn(&store, "a1", 1000, None);
    run_turn(&store, "somebody-else", 99_999, None);

    let spend = line(&store.plan(main, 0).unwrap(), t[0].id).spend;
    assert_eq!((spend.input_tokens, spend.output_tokens, spend.runs), (1000, 500, 1));
    assert_eq!(spend.cost_micros, Some(2_000));
    assert_eq!(spend.unmeasured_agents, 2, "a2 has no turn yet, and codex reports none");
    run_turn(&store, "a2", 10, None);
    assert_eq!(line(&store.plan(main, 0).unwrap(), t[0].id).spend.unmeasured_agents, 1);
}

/// An agent on two tickets counts once across them: each holds half its
/// spend and says one agent is shared, so the tickets sum to what it spent.
#[test]
fn an_agent_on_two_tickets_is_split_between_them_and_counted_once() {
    let (store, main, t) = board(2);
    let a1 = subagent(&store, main, "a1", "claude", AgentRole::Builder);
    let b1 = subagent(&store, main, "b1", "claude", AgentRole::Builder);
    let rev = subagent(&store, main, "rev", "claude", AgentRole::Reviewer);
    for (task, builder) in [(t[0].id, a1), (t[1].id, b1)] {
        on_line(&store, task);
        works(&store, builder, task, WorkRole::Build);
        works(&store, rev, task, WorkRole::Review);
    }
    run_turn(&store, "a1", 1000, None);
    run_turn(&store, "rev", 600, None);

    let plan = store.plan(main, 0).unwrap();
    let (a, b) = (line(&plan, t[0].id).spend, line(&plan, t[1].id).spend);
    assert_eq!((a.input_tokens, a.output_tokens), (1000 + 300, 500 + 150), "a1 whole, half the reviewer's");
    assert_eq!((b.input_tokens, b.output_tokens), (300, 150), "half the reviewer's");
    assert_eq!(a.input_tokens + b.input_tokens, 1000 + 600, "the reviewer counts once across the tickets");
    assert_eq!((a.cost_micros, b.cost_micros), (Some(2_000 + 1_000), Some(1_000)));
    assert_eq!((a.shared_agents, b.shared_agents), (1, 1));
    assert_eq!((a.runs, b.runs), (2, 1), "a run is in each ticket it worked for");
}

/// A ticket's spend is what `usage.report` says for the same agents on its
/// card.
#[test]
fn a_ticket_s_spend_matches_the_usage_report_for_its_agents() {
    use crate::usage::UsageFilter;
    let (store, main, t) = board(2);
    on_line(&store, t[0].id);
    let a1 = subagent(&store, main, "a1", "claude", AgentRole::Builder);
    works(&store, a1, t[0].id, WorkRole::Build);
    works(&store, a1, t[0].id, WorkRole::Fix);
    let a2 = subagent(&store, main, "a2", "claude", AgentRole::Builder);
    works(&store, a2, t[0].id, WorkRole::Fix);
    run_turn(&store, "a1", 1000, Some(t[0].id));
    run_turn(&store, "a2", 250, Some(t[0].id));
    run_turn(&store, "other", 77_777, Some(t[1].id));

    let report = store.usage_summary(&UsageFilter { task_id: Some(t[0].id), ..Default::default() }, &[], 0).unwrap().pop().unwrap().totals;
    let spend = line(&store.plan(main, 0).unwrap(), t[0].id).spend;
    assert_eq!(spend.input_tokens, 1250);
    assert_eq!(
        (spend.input_tokens, spend.output_tokens, spend.cache_read_tokens, spend.cache_write_tokens),
        (report.tokens.input, report.tokens.output, report.tokens.cache_read, report.tokens.cache_write)
    );
    assert_eq!(spend.cost_micros, Some(report.cost_reported_micros + report.cost_estimated_micros));
    assert_eq!(u64::from(spend.runs), report.subagent_runs);
}

/// Fix rounds are counted from the fixes the timeline records starting: a
/// second fix by the same agent reopens its work and counts again.
#[test]
fn fix_rounds_are_counted_from_the_fixes_started() {
    let (store, main, t) = board(1);
    on_line(&store, t[0].id);
    let a1 = subagent(&store, main, "a1", "claude", AgentRole::Builder);
    works(&store, a1, t[0].id, WorkRole::Build);
    assert_eq!(line(&store.plan(main, 0).unwrap(), t[0].id).fix_rounds, 0);
    works(&store, a1, t[0].id, WorkRole::Fix);
    store.end_work(a1, t[0].id, WorkRole::Fix, crate::ticket_work::Verdict::Finished, Actor::Manager).unwrap();
    works(&store, a1, t[0].id, WorkRole::Fix);
    assert_eq!(line(&store.plan(main, 0).unwrap(), t[0].id).fix_rounds, 2);
}

/// A finished ticket shows only for a while; a ticket sitting in one stage for
/// over an hour is stale, and one waiting never is.
#[test]
fn finished_tickets_age_out_and_a_long_stage_is_stale() {
    let (store, main, t) = board(2);
    on_line(&store, t[0].id);
    to(&store, t[0].id, Stage::Dropped);
    assert_eq!(store.plan(main, 0).unwrap().lines.len(), 1);
    assert!(store.plan(main, crate::tasks::now_millis() + 1000).unwrap().lines.is_empty());

    on_line(&store, t[1].id);
    let old = crate::tasks::now_millis() - 2 * STALE_AFTER_MS;
    store.conn().execute("UPDATE ticket_lines SET stage_since = ?1", [old]).unwrap();
    assert!(!line(&store.plan(main, 0).unwrap(), t[1].id).stale, "waiting is never stale");
    to(&store, t[1].id, Stage::Building);
    store.conn().execute("UPDATE ticket_lines SET stage_since = ?1", [old]).unwrap();
    assert!(line(&store.plan(main, 0).unwrap(), t[1].id).stale);
}

/// A landing reads "Landing N", says what it carries in its tickets' words,
/// each of its tickets knows its number, and the empty collecting landing is
/// left out.
#[test]
fn a_landing_reads_its_number_and_what_it_carries() {
    let (store, main, t) = board(2);
    for task in [t[0].id, t[1].id] {
        on_line(&store, task);
        for stage in [Stage::Building, Stage::Reviewing, Stage::Landing] {
            to(&store, task, stage);
        }
    }
    let plan = store.plan(main, 0).unwrap();
    let first = &plan.landings[0];
    assert_eq!(first.title, format!("Landing {}", first.landing.number));
    assert_eq!(first.summary, "Task 0", "the area lead dropped, capitalized");
    assert_eq!(line(&plan, t[0].id).landing, Some(first.landing.number));
    assert_eq!(first.phase, crate::landing_phase::LandingPhase::GatingLocally, "it left at once");
    let second = &plan.landings[1];
    assert_eq!(second.summary, "Task 1");
    assert_eq!(second.phase, crate::landing_phase::LandingPhase::WaitingForPreviousLanding);
    assert_eq!(plan.landings.len(), 2, "no empty collecting landing");

    let titled = LandingUpdate { title: Some("Agents message the plan".into()), ..Default::default() };
    store.update_landing(first.landing.id, &titled, Actor::Manager).unwrap();
    assert_eq!(store.plan(main, 0).unwrap().landings[0].title, "Agents message the plan");
}

fn conn_set(store: &Store, sql: &str) {
    store.conn().execute_batch(sql).unwrap();
}

fn hex(id: Uuid) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

/// A theme's `last_moved_at` (ov-331), term by term: its story, its cards'
/// own clock, a live ticket, a dropped one, and its rulings. Removing one term,
/// or the filter that keeps another theme's ruling off it, changes exactly one
/// answer.
#[test]
fn a_theme_last_moved_is_the_newest_of_its_story_cards_tickets_and_rulings() {
    let (store, main, _) = board(0);
    let theme_and_card = |name: &str| {
        let task = store.create_task(main, &format!("card of {name}"), Actor::Manager).unwrap();
        let theme = store
            .create_theme(main, &NewTheme { name: name.into(), outcome: String::new() }, &[task.id], Actor::Manager)
            .unwrap();
        (theme, task)
    };
    let (ruled, _) = theme_and_card("Ruled");
    let (_, dropped_card) = theme_and_card("Dropped");
    let (_, live_card) = theme_and_card("Live");
    let (told, _) = theme_and_card("Told");
    let (_, edited_card) = theme_and_card("Edited");
    theme_and_card("Other");

    on_line(&store, dropped_card.id);
    to(&store, dropped_card.id, Stage::Dropped);
    on_line(&store, live_card.id);

    // One baseline for every card and story, so each term below is the only newer thing on its theme.
    conn_set(&store, "UPDATE tasks SET created_at = 1000, status_since = 1000, edited_at = NULL");
    conn_set(&store, "UPDATE board_themes SET story_at = 1000");
    let ruling = store
        .add_ruling(
            main,
            &NewRuling { decision: "d".into(), why: "w".into(), reversal: "r".into(), theme_id: Some(ruled.id) },
            &[],
            Actor::Manager,
        )
        .unwrap();
    conn_set(&store, &format!("UPDATE board_rulings SET created_at = 9000, settled_at = 9500, state = 'confirmed' WHERE number = {}", ruling.number));
    // A dropped ticket, newer than anything: it is gone, and must not count.
    conn_set(&store, &format!("UPDATE ticket_lines SET stage_since = 20000 WHERE task_id = x'{}'", hex(dropped_card.id)));
    conn_set(&store, &format!("UPDATE ticket_lines SET stage_since = 7000 WHERE task_id = x'{}'", hex(live_card.id)));
    conn_set(&store, &format!("UPDATE board_themes SET story_at = 12000 WHERE name = '{}'", told.name));
    conn_set(&store, &format!("UPDATE tasks SET edited_at = 15000 WHERE id = x'{}'", hex(edited_card.id)));

    let plan = store.plan(main, 0).unwrap();
    let at = |name: &str| plan.themes.iter().find(|v| v.theme.name == name).unwrap().last_moved_at;
    assert_eq!(at("Ruled"), 9500, "its ruling, settled");
    assert_eq!(at("Dropped"), 1000, "a dropped ticket, and another theme's ruling, don't move it");
    assert_eq!(at("Live"), 7000, "its live ticket");
    assert_eq!(at("Told"), 12000, "its story");
    assert_eq!(at("Edited"), 15000, "its card's own clock, which counts an edit");
    assert_eq!(at("Other"), 1000, "nothing newer than its card and story touches it");
}

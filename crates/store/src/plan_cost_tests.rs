use super::*;

use crate::models::Task;
use crate::plan::{AgentRecord, AgentRole, LaneCard, LaneState, LaneUpdate, NewLane, NewTheme};
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use farcooler_core::usage::TokenCounts;

const DAY: i64 = DAY_MS;

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn counts(tokens: u64) -> TokenCounts {
    TokenCounts { input: tokens, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0 }
}

/// A Claude subagent's run, the kind a lane's agent is read from.
fn agent_turn(store: &Store, agent: &str, tokens: u64, ended_at: i64) {
    turn(store, &format!("claude-log:agent:{agent}"), None, "claude", "claude-opus-5", tokens, Some(1_000), ended_at);
}

#[allow(clippy::too_many_arguments)]
fn turn(
    store: &Store,
    key: &str,
    task: Option<Uuid>,
    harness: &str,
    model: &str,
    tokens: u64,
    cost: Option<i64>,
    ended_at: i64,
) {
    let model = if cost.is_some() {
        TurnModel::priced(Some(model.into()), counts(tokens), cost)
    } else {
        TurnModel::priced(Some("model-with-no-price".into()), counts(tokens), None)
    };
    store
        .record_turn(&NewTurn {
            key: key.into(),
            terminal_id: None,
            worktree_id: None,
            repository_id: None,
            workspace_id: None,
            task_id: task,
            harness: harness.into(),
            surface: Surface::Terminal,
            started_at: None,
            ended_at,
            active_ms: None,
            usage: "reported",
            models: vec![model],
            kind: TurnKind::Turn,
        })
        .unwrap();
}

fn card(task: &Task) -> LaneCard {
    LaneCard { task_id: task.id, slice: String::new() }
}

fn claude(id: &str) -> AgentRecord {
    AgentRecord { harness: "claude".into(), agent_id: id.into(), role: AgentRole::Build, model: None, ended: false }
}

fn lane(store: &Store, main: Uuid, name: &str, cards: &[LaneCard], agent: &str) -> crate::plan::Lane {
    store
        .create_lane(main, &NewLane { name: name.into(), ..Default::default() }, cards, Some(&claude(agent)), Actor::Manager)
        .unwrap()
}

fn theme(store: &Store, main: Uuid, name: &str, tasks: &[&Task]) -> crate::plan::BoardTheme {
    let ids: Vec<Uuid> = tasks.iter().map(|t| t.id).collect();
    store.create_theme(main, &NewTheme { name: name.into(), outcome: String::new() }, &ids, Actor::Manager).unwrap()
}

fn total(s: &crate::plan_read::LaneSpend) -> u64 {
    s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens
}

/// A budget is stored per theme and per lane, read back on the plan, changed
/// by a second set, removed by `None`, and refused at nothing or at nonsense.
#[test]
fn a_budget_is_kept_changed_and_removed() {
    let (store, main, t) = board(1);
    let th = theme(&store, main, "Cost", &[&t[0]]);
    let ln = lane(&store, main, "cost-lane", &[card(&t[0])], "a1");

    store.set_budget(Subject::Theme(th.id), Some(5_000), Actor::Manager).unwrap();
    store.set_budget(Subject::Lane(ln.id), Some(900), Actor::Manager).unwrap();
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.themes[0].budget_tokens, Some(5_000));
    assert_eq!(plan.lanes[0].budget_tokens, Some(900));

    store.set_budget(Subject::Theme(th.id), Some(7_000), Actor::Manager).unwrap();
    assert_eq!(store.plan(main, 0).unwrap().themes[0].budget_tokens, Some(7_000), "a second set replaces, never adds");
    store.set_budget(Subject::Theme(th.id), None, Actor::Manager).unwrap();
    assert_eq!(store.plan(main, 0).unwrap().themes[0].budget_tokens, None);

    assert!(matches!(store.set_budget(Subject::Lane(ln.id), Some(0), Actor::Manager), Err(DomainError::InvalidArgument { .. })));
    assert!(matches!(store.set_budget(Subject::Lane(Uuid::from_u128(77)), Some(1), Actor::Manager), Err(DomainError::NotFound)));

    let events = store.plan_events(Subject::Theme(th.id), 0).unwrap();
    let budget: Vec<_> = events.iter().filter(|e| e.kind == "budget").map(|e| e.body.as_str()).collect();
    assert_eq!(budget, ["budget 5000 tokens", "budget 7000 tokens", "budget removed"]);
}

/// A budget goes with its lane, and with the layer: the drill drops the table
/// with the rest and the board reads the same.
#[test]
fn a_lanes_budget_goes_with_the_lane() {
    let (store, main, t) = board(1);
    let ln = lane(&store, main, "gone", &[card(&t[0])], "a1");
    store.set_budget(Subject::Lane(ln.id), Some(10), Actor::Manager).unwrap();
    store.conn().execute("DELETE FROM lanes", []).unwrap();
    let left: i64 = store.conn().query_row("SELECT count(*) FROM plan_budgets", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 0);
}

/// A theme's trend is its lanes' tokens by UTC day, shared out over their
/// cards as its spend is: a lane on one card inside and one outside gives half.
#[test]
fn a_theme_trend_is_its_lanes_by_day_and_shares_like_its_spend() {
    let (store, main, t) = board(3);
    let th = theme(&store, main, "Cost", &[&t[0], &t[1]]);
    lane(&store, main, "half", &[card(&t[0]), card(&t[2])], "a1");
    let whole = lane(&store, main, "whole", &[card(&t[1])], "a2");
    store.record_lane_agent(whole.id, &claude("a3"), Actor::Manager).unwrap();
    let now = now_millis();
    let today = now.div_euclid(DAY) * DAY;
    agent_turn(&store, "a1", 1000, today + 1);
    agent_turn(&store, "a2", 300, today - 2 * DAY + 1);
    // Older than the window: in the spend, not in the trend.
    agent_turn(&store, "a3", 40, today - 9 * DAY);

    let plan = store.plan(main, 0).unwrap();
    let view = plan.themes.iter().find(|v| v.theme.id == th.id).unwrap();
    assert_eq!(view.trend, [0, 0, 0, 0, 300, 0, 500], "oldest first, today last");
    assert_eq!(total(&view.spend), 500 + 340);
}

/// The week's tokens are the runner's own, every harness, and stop at seven days.
#[test]
fn the_week_counts_seven_days_and_no_limit() {
    let (store, main, _) = board(0);
    let now = now_millis();
    turn(&store, "k1", None, "claude", "claude-opus-5", 100, Some(1), now - 1000);
    turn(&store, "k2", None, "codex", "gpt-5.6", 20, None, now - 6 * DAY);
    turn(&store, "k3", None, "claude", "claude-opus-5", 9_999, Some(1), now - 8 * DAY);
    assert_eq!(store.plan(main, 0).unwrap().cost.week_tokens, 120);
}

fn finish(store: &Store, task: &Task) {
    use crate::models::TaskStatus::*;
    for to in [Todo, InProgress, InReview, Done] {
        store.set_task_status(task.id, to, Actor::Manager).unwrap();
    }
}

/// Cost per finished card, by harness and model: a pair needs
/// `MIN_CARDS_TO_COMPARE` finished cards, a card still open counts for
/// nothing, and a pair held back is counted, not drawn.
#[test]
fn a_comparison_needs_three_finished_cards_and_says_how_many_it_held_back() {
    let (store, main, t) = board(7);
    let now = now_millis();
    // Three finished cards on claude/opus, one with an unpriced turn after it.
    for (i, task) in t[..3].iter().enumerate() {
        finish(&store, task);
        turn(&store, &format!("c{i}"), Some(task.id), "claude", "claude-opus-5", 100, Some(2_000), now);
    }
    // Two finished on codex: held back. One still open on claude: not counted.
    for (i, task) in t[3..5].iter().enumerate() {
        finish(&store, task);
        turn(&store, &format!("x{i}"), Some(task.id), "codex", "gpt-5.6", 50, None, now);
    }
    turn(&store, "open", Some(t[5].id), "claude", "claude-opus-5", 1_000_000, Some(9), now);

    let cost = store.plan(main, 0).unwrap().cost;
    assert_eq!(cost.compare_held_back, 1, "codex has two");
    assert_eq!(cost.compare.len(), 1);
    let c = &cost.compare[0];
    assert_eq!((c.harness.as_str(), c.model.as_str(), c.cards, c.tokens), ("claude", "claude-opus-5", 3, 300));
    assert_eq!(c.cost_micros, Some(6_000));

    // A third codex card brings it over the line, and its unpriced turns leave
    // the dollars out rather than reading as free.
    finish(&store, &t[5]);
    finish(&store, &t[6]);
    turn(&store, "x2", Some(t[6].id), "codex", "gpt-5.6", 50, None, now);
    let cost = store.plan(main, 0).unwrap().cost;
    assert_eq!(cost.compare_held_back, 0);
    let codex = cost.compare.iter().find(|c| c.harness == "codex").unwrap();
    assert_eq!((codex.cards, codex.cost_micros), (3, None));
}

/// A lane that landed still counts in the trend, and a dropped one too.
#[test]
fn a_closed_lane_keeps_its_days() {
    let (store, main, t) = board(1);
    let th = theme(&store, main, "Cost", &[&t[0]]);
    let ln = lane(&store, main, "done", &[card(&t[0])], "a1");
    agent_turn(&store, "a1", 70, now_millis());
    store.update_lane(ln.id, &LaneUpdate { state: Some(LaneState::Dropped), ..Default::default() }, Actor::Manager).unwrap();
    let plan = store.plan(main, i64::MAX).unwrap();
    assert_eq!(plan.themes.iter().find(|v| v.theme.id == th.id).unwrap().trend.iter().sum::<u64>(), 70);
}

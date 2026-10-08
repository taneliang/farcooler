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

fn turn_between(store: &Store, key: &str, started: i64, ended: i64, tokens: u64) {
    store
        .record_turn(&NewTurn {
            key: key.into(),
            terminal_id: None,
            worktree_id: None,
            repository_id: None,
            workspace_id: None,
            task_id: None,
            harness: "claude".into(),
            surface: Surface::Terminal,
            started_at: Some(started),
            ended_at: ended,
            active_ms: None,
            usage: "reported",
            models: vec![TurnModel::priced(Some("claude-opus-5".into()), counts(tokens), Some(1_000))],
            kind: TurnKind::Subagent,
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

/// A run's row starts when the agent first spoke and ends when it last did: its
/// tokens fall on the days it ran, in proportion, not all on the last one.
#[test]
fn a_run_is_spread_over_the_days_it_ran() {
    let (store, main, t) = board(1);
    let th = theme(&store, main, "Cost", &[&t[0]]);
    lane(&store, main, "long", &[card(&t[0])], "a1");
    let today = now_millis().div_euclid(DAY) * DAY;
    // Four whole days: three before today's midnight, then today so far.
    turn_between(&store, "claude-log:agent:a1", today - 3 * DAY, today + 1000, 3_000);
    let plan = store.plan(main, 0).unwrap();
    let view = plan.themes.iter().find(|v| v.theme.id == th.id).unwrap();
    assert_eq!(view.trend[..3], [0, 0, 0], "before the run began");
    assert!(view.trend[3] >= 999 && view.trend[3] <= 1001, "{:?}", view.trend);
    assert!((view.trend[4] + view.trend[5]).abs_diff(2000) <= 2, "{:?}", view.trend);
    assert!(view.trend[6] <= 2, "today holds its sliver, not the run: {:?}", view.trend);
    assert_eq!(plan.cost.week_tokens, view.trend.iter().sum::<u64>(), "one window for both");
}

/// Part of a run that began before the window is not in the week or the trend.
#[test]
fn what_a_run_spent_before_the_window_is_not_in_it() {
    let (store, main, _) = board(0);
    let today = now_millis().div_euclid(DAY) * DAY;
    // Two days that began a day and a half before the window and ended half a
    // day into it: a quarter of the run is in.
    let window = today - 6 * DAY;
    turn_between(&store, "k1", window - DAY - DAY / 2, window + DAY / 2, 8_000);
    let week = store.plan(main, 0).unwrap().cost.week_tokens;
    assert!(week.abs_diff(2_000) <= 2, "a quarter of the run, not the whole: {week}");
}

/// An agent on two lanes gives each half its tokens each day.
#[test]
fn a_shared_agent_is_split_in_the_trend() {
    let (store, main, t) = board(2);
    let th = theme(&store, main, "Cost", &[&t[0]]);
    lane(&store, main, "one", &[card(&t[0])], "a1");
    lane(&store, main, "two", &[card(&t[1])], "a1");
    agent_turn(&store, "a1", 1000, now_millis().div_euclid(DAY) * DAY + 1);
    let plan = store.plan(main, 0).unwrap();
    let view = plan.themes.iter().find(|v| v.theme.id == th.id).unwrap();
    assert_eq!(view.trend[6], 500);
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

/// The week's total is split by harness and model, the parts add up to the
/// total in tokens and dollars, and one unpriced turn leaves the dollars out
/// rather than understating them (ov-434).
#[test]
fn the_week_is_split_by_harness_and_model_and_the_parts_add_up() {
    let (store, main, _) = board(0);
    let now = now_millis();
    turn(&store, "k1", None, "claude", "claude-opus-5", 100, Some(4_000), now - 1000);
    turn(&store, "k2", None, "claude", "claude-opus-5", 50, Some(2_000), now - 2 * DAY);
    turn(&store, "k3", None, "codex", "gpt-5.6", 20, Some(500), now - 6 * DAY);
    turn(&store, "k4", None, "claude", "claude-opus-5", 9_999, Some(1), now - 8 * DAY);
    let cost = store.plan(main, 0).unwrap().cost;
    let said: Vec<_> = cost.week.iter().map(|w| (w.harness.as_str(), w.model.as_str(), w.tokens, w.cost_micros)).collect();
    assert_eq!(said, [("claude", "claude-opus-5", 150, Some(6_000)), ("codex", "gpt-5.6", 20, Some(500))]);
    assert_eq!((cost.week_tokens, cost.week_cost_micros), (170, Some(6_500)));
    assert_eq!(cost.week.iter().map(|w| w.tokens).sum::<u64>(), cost.week_tokens);

    turn(&store, "k5", None, "codex", "gpt-5.6", 30, None, now - 1000);
    let cost = store.plan(main, 0).unwrap().cost;
    assert_eq!(cost.week_tokens, 200);
    assert_eq!(cost.week_cost_micros, None, "a week with an unpriced turn has no dollar total");
    assert_eq!(cost.week.iter().find(|w| w.model == "claude-opus-5").unwrap().cost_micros, Some(6_000));
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
    assert_eq!((c.harness.as_str(), c.model.as_str(), c.card_share_milli, c.tokens), ("claude", "claude-opus-5", 3000, 300));
    assert_eq!(c.cost_micros, Some(6_000));

    // A third codex card brings it over the line, and its unpriced turns leave
    // the dollars out rather than reading as free.
    finish(&store, &t[5]);
    finish(&store, &t[6]);
    turn(&store, "x2", Some(t[6].id), "codex", "gpt-5.6", 50, None, now);
    let cost = store.plan(main, 0).unwrap().cost;
    assert_eq!(cost.compare_held_back, 0);
    let codex = cost.compare.iter().find(|c| c.harness == "codex").unwrap();
    assert_eq!((codex.card_share_milli, codex.cost_micros), (3000, None));
}

/// Cost per landed card is all the spend on landed cards over the landed
/// cards: spend on a cancelled or open card is in flight, not dropped and not
/// charged to a landed one, and a card two pairs worked is one card in total.
#[test]
fn spend_on_cards_that_did_not_land_is_in_flight_and_a_shared_card_is_one_card() {
    let (store, main, t) = board(6);
    let now = now_millis();
    // Four landed cards, each built by opus (900 tokens) and reviewed by sonnet (100).
    for (i, task) in t[..4].iter().enumerate() {
        finish(&store, task);
        turn(&store, &format!("b{i}"), Some(task.id), "claude", "claude-opus-5", 900, Some(9_000), now);
        turn(&store, &format!("r{i}"), Some(task.id), "claude", "claude-sonnet-5", 100, Some(1_000), now);
    }
    // Opus also burned 5,000 tokens on a cancelled card and 700 on an open one:
    // none of it is a landed card's.
    store.set_task_status(t[4].id, crate::models::TaskStatus::Cancelled, Actor::Manager).unwrap();
    turn(&store, "x", Some(t[4].id), "claude", "claude-opus-5", 5_000, Some(50_000), now);
    turn(&store, "y", Some(t[5].id), "claude", "claude-opus-5", 700, Some(7_000), now);

    let cost = store.plan(main, 0).unwrap().cost;
    assert_eq!((cost.in_flight_tokens, cost.in_flight_cost_micros), (5_700, Some(57_000)));
    // Opus holds 4 x 0.9 = 3.6 cards' share, so it is compared; sonnet's 0.4 is
    // held back, not drawn as a card of its own.
    assert_eq!((cost.compare.len(), cost.compare_held_back), (1, 1));
    let opus = &cost.compare[0];
    assert_eq!((opus.model.as_str(), opus.card_share_milli, opus.tokens), ("claude-opus-5", 3_600, 3_600));
    assert_eq!(opus.tokens * 1000 / opus.card_share_milli as u64, 1_000, "a landed card cost 1,000 tokens, in total");
    assert_eq!(opus.cost_micros, Some(36_000), "only its spend on landed cards");
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

//! A lane's agent pane (ov-457), on an in-memory board.

use super::*;

use crate::models::Task;
use crate::plan::NewLane;
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use farcooler_core::usage::TokenCounts;

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn lane(store: &Store, main: Uuid, name: &str, cards: &[&Task]) -> Lane {
    let cards: Vec<LaneCard> = cards.iter().map(|t| LaneCard { task_id: t.id, slice: String::new() }).collect();
    store.create_lane(main, &NewLane { name: name.into(), ..Default::default() }, &cards, None, Actor::Manager).unwrap()
}

/// A pane start in a real worktree of the board's repository: the lane's
/// worktree is a foreign key.
fn start_in(store: &Store, main: Uuid, terminal: Uuid, preset: &str) -> PaneStart {
    let repo = store.get_workspace(main).unwrap().repository_id;
    let path = format!("/wt/{terminal}");
    let worktree = store.create_worktree(repo, &terminal.to_string(), &path, false).unwrap().id;
    PaneStart {
        terminal,
        preset: preset.into(),
        worktree,
        worktree_path: "/wt/lane".into(),
        branch: "lane-branch".into(),
    }
}

/// Starting a pane on a queued lane records it as the build agent, moves the
/// lane to building and puts it on the pane's branch, in one write.
#[test]
fn a_pane_start_records_the_agent_and_builds() {
    let (store, main, t) = board(2);
    let made = lane(&store, main, "mac-ux", &[&t[0], &t[1]]);
    assert_eq!(made.state, LaneState::Queued);
    let pane = Uuid::now_v7();
    let after = store.start_lane_pane(made.id, &start_in(&store, main, pane, "codex:gpt-5"), Actor::Manager).unwrap();
    assert_eq!(after.state, LaneState::Building);
    assert_eq!(after.branch, "lane-branch");
    assert_eq!(after.worktree_path, "/wt/lane");
    let plan = store.plan(main, 0).unwrap();
    let view = plan.lanes.iter().find(|l| l.lane.id == made.id).unwrap();
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].agent_id, pane_agent_id(pane));
    assert_eq!(view.agents[0].harness, "codex");
    assert_eq!(view.agents[0].model, "gpt-5");
    assert_eq!(pane_of_agent(&view.agents[0].agent_id), Some(pane));
}

/// A cursor pane can be a lane's agent too: lanes take cursor, which the
/// board's subagent records never did.
#[test]
fn a_cursor_pane_is_a_lane_agent() {
    let (store, main, t) = board(1);
    let made = lane(&store, main, "cursor-lane", &[&t[0]]);
    store.start_lane_pane(made.id, &start_in(&store, main, Uuid::now_v7(), "cursor"), Actor::Manager).unwrap();
}

/// The pane reads its lane back while it works it, and not once it's
/// recorded as finished or the lane has landed.
#[test]
fn a_pane_knows_its_lane_until_it_ends() {
    let (store, main, t) = board(1);
    let made = lane(&store, main, "phones", &[&t[0]]);
    let pane = Uuid::now_v7();
    assert_eq!(store.lane_of_pane(pane).unwrap(), None);
    store.start_lane_pane(made.id, &start_in(&store, main, pane, "claude"), Actor::Manager).unwrap();
    assert_eq!(store.lane_of_pane(pane).unwrap().map(|l| l.name), Some("phones".to_string()));
    store.end_lane_pane(pane, Actor::Runner).unwrap();
    assert_eq!(store.lane_of_pane(pane).unwrap(), None);
    store.end_lane_pane(pane, Actor::Runner).unwrap();
}

/// A live lane is found by name without case; a dropped one isn't.
#[test]
fn a_live_lane_is_found_by_name() {
    let (store, main, t) = board(2);
    let made = lane(&store, main, "Mac-UX", &[&t[1], &t[0]]);
    assert_eq!(store.live_lane_named(main, "mac-ux").unwrap().id, made.id);
    let cards: Vec<Uuid> = store.lane_cards_in_order(made.id).unwrap().into_iter().map(|c| c.task_id).collect();
    assert_eq!(cards, [t[1].id, t[0].id], "in the order they were added");
    store
        .update_lane(made.id, &crate::plan::LaneUpdate { state: Some(LaneState::Dropped), ..Default::default() }, Actor::Manager)
        .unwrap();
    assert!(matches!(store.live_lane_named(main, "mac-ux"), Err(DomainError::NotFound)));
}

/// A pane's turns are its lane's spend, whatever its harness: every turn
/// filed under its terminal, and no other terminal's.
#[test]
fn a_panes_turns_are_its_lanes_spend() {
    let (store, main, t) = board(1);
    let made = lane(&store, main, "spend", &[&t[0]]);
    let pane = Uuid::now_v7();
    store.start_lane_pane(made.id, &start_in(&store, main, pane, "codex"), Actor::Manager).unwrap();
    let turn = |key: &str, terminal: Uuid, tokens: u64| {
        store
            .record_turn(&NewTurn {
                key: key.into(),
                terminal_id: Some(terminal),
                worktree_id: None,
                repository_id: None,
                workspace_id: None,
                task_id: Some(t[0].id),
                harness: "codex".into(),
                surface: Surface::Terminal,
                started_at: None,
                ended_at: 1000,
                active_ms: None,
                usage: "reported",
                models: vec![TurnModel::priced(
                    Some("gpt-5".into()),
                    TokenCounts { input: tokens, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, fast: false },
                    Some(1_000),
                )],
                kind: TurnKind::Turn,
            })
            .unwrap();
    };
    turn("codex:1", pane, 100);
    turn("codex:2", pane, 50);
    turn("codex:3", Uuid::now_v7(), 7_000);
    let plan = store.plan(main, 0).unwrap();
    let view = plan.lanes.iter().find(|l| l.lane.id == made.id).unwrap();
    assert_eq!(view.spend.input_tokens, 150);
    assert_eq!(view.spend.runs, 2);
    assert_eq!(view.spend.unmeasured_agents, 0, "the pane is measured");
}

/// A card finds its live lane, and the lane its working pane, until the pane
/// is recorded as finished.
#[test]
fn a_card_finds_its_lane_and_the_lane_its_pane() {
    let (store, main, t) = board(2);
    assert_eq!(store.live_lane_of_card(t[0].id).unwrap(), None);
    let made = lane(&store, main, "phones", &[&t[0]]);
    assert_eq!(store.live_lane_of_card(t[0].id).unwrap().map(|l| l.id), Some(made.id));
    assert_eq!(store.live_lane_of_card(t[1].id).unwrap(), None);
    assert_eq!(store.lane_pane(made.id).unwrap(), None);
    let pane = Uuid::now_v7();
    store.start_lane_pane(made.id, &start_in(&store, main, pane, "claude"), Actor::Manager).unwrap();
    assert_eq!(store.lane_pane(made.id).unwrap(), Some(pane));
    store.end_lane_pane(pane, Actor::Runner).unwrap();
    assert_eq!(store.lane_pane(made.id).unwrap(), None);
}

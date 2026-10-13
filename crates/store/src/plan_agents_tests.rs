//! The agents on the plan (ov-503), over turns read from the repository's
//! recorded session logs (`crates/core/fixtures/session-logs/`), through the
//! same `LogUsage` fold the daemon uses: a subagent's own transcript, and a
//! pane's complete turn.

use super::*;

use crate::models::{Actor, Task};
use crate::plan::{AgentRecord, AgentRole, LaneCard, NewLane};
use crate::plan_panes::PaneStart;
use crate::store::Store;
use crate::trains::{NewTrain, TrainAgentRecord};
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use crate::workers::{LinkedBy, WorkerRecord};
use farcooler_core::session_log::usage::{LogUsage, LoggedTurn};

fn fixture(name: &str) -> Vec<String> {
    let path = format!("{}/../core/fixtures/session-logs/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap().lines().map(str::to_string).collect()
}

/// The one turn of a recorded pane log, under `key`.
fn pane_turn(key: &str) -> LoggedTurn {
    let mut usage = LogUsage::default();
    fixture("claude-complete-turn.jsonl").iter().for_each(|l| usage.claude_line(l));
    let mut turns = usage.take();
    assert_eq!(turns.len(), 1, "the recording is one finished turn");
    // The two recordings were made on different days: move the pane's turn to
    // five minutes before the subagent's run, so one window holds both and the
    // pane is still recent.
    let mut turn = turns.remove(0).with_key(key);
    let end = subagent_turn("clock").ended_at_ms.unwrap() - 300_000;
    turn.started_at_ms = Some(end - 5_000);
    turn.ended_at_ms = Some(end);
    turn
}

/// A recorded subagent transcript, as the run `agent` made.
fn subagent_turn(agent: &str) -> LoggedTurn {
    let mut usage = LogUsage::default();
    fixture("claude-subagent-transcript.jsonl").iter().for_each(|l| usage.subagent_line(agent, l));
    let mut turns = usage.take();
    assert_eq!(turns.len(), 1);
    turns.remove(0)
}

trait WithKey {
    fn with_key(self, key: &str) -> Self;
}
impl WithKey for LoggedTurn {
    fn with_key(mut self, key: &str) -> Self {
        self.key = key.to_string();
        self
    }
}

fn tokens_of(t: &LoggedTurn) -> u64 {
    t.models.iter().map(|(_, c)| c.input + c.output + c.cache_read + c.cache_write).sum()
}

struct Fixture {
    store: Store,
    ws: Uuid,
    repo: Uuid,
    worktree: Uuid,
    cards: Vec<Task>,
}

fn fixture_board() -> Fixture {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let cards = (0..2).map(|i| store.create_task(ws, &format!("card {i}"), Actor::Manager).unwrap()).collect();
    let worktree = store.create_worktree(repo, "main", "/wt/main", true).unwrap().id;
    Fixture { store, ws, repo, worktree, cards }
}

impl Fixture {
    fn terminal(&self) -> Uuid {
        self.store.create_terminal_for_test(self.worktree, self.ws)
    }

    /// Record `turn` the way the daemon does: a subagent's whole run, or a pane's turn.
    fn record(&self, turn: &LoggedTurn, terminal: Option<Uuid>) {
        self.store
            .record_turn(&NewTurn {
                key: turn.key.clone(),
                terminal_id: terminal,
                worktree_id: None,
                repository_id: Some(self.repo),
                workspace_id: None,
                task_id: None,
                harness: "claude".into(),
                surface: Surface::Terminal,
                started_at: turn.started_at_ms,
                ended_at: turn.ended_at_ms.unwrap(),
                active_ms: turn.active_ms,
                usage: turn.state.as_str(),
                models: turn.models.iter().map(|(m, c)| TurnModel::priced(m.clone(), *c, None)).collect(),
                kind: if turn.subagent { TurnKind::Subagent } else { TurnKind::Turn },
            })
            .unwrap();
    }

    fn agents(&self, now: i64) -> Vec<PlanAgent> {
        self.agents_live(now, &|_| None)
    }

    fn agents_live(&self, now: i64, live: &dyn Fn(Uuid) -> Option<bool>) -> Vec<PlanAgent> {
        agents_of(&self.store.conn(), self.ws, now, live).unwrap()
    }

    fn lane(&self, name: &str, cards: &[&Task]) -> Uuid {
        let cards: Vec<LaneCard> = cards.iter().map(|t| LaneCard { task_id: t.id, slice: String::new() }).collect();
        self.store
            .create_lane(self.ws, &NewLane { name: name.into(), ..Default::default() }, &cards, None, Actor::Manager)
            .unwrap()
            .id
    }
}

fn find<'a>(agents: &'a [PlanAgent], id: &str) -> &'a PlanAgent {
    agents.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no agent {id} in {:?}", agents.iter().map(|a| &a.id).collect::<Vec<_>>()))
}

/// The whole board at once: an orchestrator, a lane's pane, a lane's subagent,
/// a card's subagent, a train's integrator, and a subagent nothing names.
#[test]
fn every_kind_of_agent_is_named_by_what_it_works_for() {
    let f = fixture_board();
    let (orch, pane) = (f.terminal(), f.terminal());
    f.store.set_terminal_role(orch, TerminalRole::Orchestrator).unwrap();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    f.store
        .start_lane_pane(
            lane,
            &PaneStart { terminal: pane, preset: "claude:opus".into(), worktree: f.worktree, worktree_path: "/wt/lane".into(), branch: "mac-ux".into() },
            Actor::Manager,
        )
        .unwrap();
    f.store
        .record_lane_agent(
            lane,
            &AgentRecord { harness: "claude".into(), agent_id: "a-review".into(), role: AgentRole::Review, model: None, ended: false },
            Actor::Manager,
        )
        .unwrap();
    f.store
        .record_worker(
            f.cards[1].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-card".into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: Some(orch),
                label: Some("Design ov-503".into()),
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    f.store
        .start_train(
            f.ws,
            &NewTrain {
                name: "train-86".into(),
                agent: Some(TrainAgentRecord { harness: "claude".into(), agent_id: "a-train".into(), model: None, ended: false }),
                ..Default::default()
            },
            &[],
            Actor::Manager,
        )
        .unwrap();

    let orch_turn = pane_turn("claude-log:orch-1");
    let pane_turn = pane_turn("claude-log:pane-1");
    f.record(&orch_turn, Some(orch));
    f.record(&pane_turn, Some(pane));
    // The daemon files a subagent's run to its orchestrator's terminal too.
    let mut now = 0;
    for agent in ["a-review", "a-card", "a-train", "a-ghost"] {
        let run = subagent_turn(agent);
        now = now.max(run.ended_at_ms.unwrap());
        f.record(&run, Some(orch));
    }
    let now = now.max(orch_turn.ended_at_ms.unwrap()) + 60_000;

    let agents = f.agents(now);
    assert_eq!(agents.len(), 6, "{agents:#?}");

    let o = find(&agents, &format!("pane:{orch}"));
    assert_eq!(o.role, PlanAgentRole::Orchestrator);
    assert_eq!(total(&o.spend), tokens_of(&orch_turn), "its own turns only: the subagents filed to its pane are theirs");
    assert!(o.working, "its terminal still runs");

    let p = find(&agents, &format!("pane:{pane}"));
    assert_eq!(p.role, PlanAgentRole::Build);
    assert_eq!(p.lane.as_ref().map(|l| l.1.as_str()), Some("mac-ux"));
    assert_eq!(total(&p.spend), tokens_of(&pane_turn));

    let r = find(&agents, "agent:a-review");
    assert_eq!((r.role, r.lane.as_ref().map(|l| l.1.as_str())), (PlanAgentRole::Review, Some("mac-ux")));
    assert_eq!(total(&r.spend), tokens_of(&subagent_turn("a-review")));
    assert!(r.model.starts_with("claude-"), "the model that spent: {}", r.model);

    let c = find(&agents, "agent:a-card");
    assert_eq!(c.role, PlanAgentRole::Subagent);
    assert_eq!(c.card.as_ref().map(|c| c.1.clone()), Some(f.cards[1].key.clone()));
    assert_eq!(c.label, "Design ov-503", "the description is the worker record's: the transcript carries none");
    assert!(c.lane.is_none());

    let t = find(&agents, "agent:a-train");
    assert_eq!(t.role, PlanAgentRole::Integrator);
    assert_eq!(t.train.as_ref().map(|t| t.1.as_str()), Some("Train 86"));
}

/// An agent nothing joins is still on the list, with its spend, and the
/// list adds up to every turn in the window.
#[test]
fn an_agent_nothing_joins_is_still_counted() {
    let f = fixture_board();
    let ghost = subagent_turn("a-ghost");
    let stray = pane_turn("claude-log:stray");
    f.record(&ghost, None);
    f.record(&stray, None);
    let now = ghost.ended_at_ms.unwrap().max(stray.ended_at_ms.unwrap()) + 60_000;

    let agents = f.agents(now);
    let g = find(&agents, "agent:a-ghost");
    assert_eq!(g.role, PlanAgentRole::Subagent);
    assert!(g.lane.is_none() && g.train.is_none() && g.card.is_none());
    assert_eq!(g.label, "", "no worker record, so no description to show");
    assert!(g.working, "it moved a minute ago and nothing says it stopped");
    let s = find(&agents, "turns:claude");
    assert_eq!(s.role, PlanAgentRole::Agent, "turns filed to no terminal are grouped, not dropped");
    assert_eq!(agents.iter().map(|a| total(&a.spend)).sum::<u64>(), tokens_of(&ghost) + tokens_of(&stray));
    assert!(total(&g.spend) > 0);
}

/// A turn older than a day is not on the list, and a quiet unrecorded
/// subagent has stopped working.
#[test]
fn the_list_covers_the_last_day() {
    let f = fixture_board();
    let run = subagent_turn("a-old");
    f.record(&run, None);
    let ended = run.ended_at_ms.unwrap();
    assert_eq!(f.agents(ended + WINDOW_MS).len(), 1, "a turn exactly a day old is in");
    assert!(f.agents(ended + WINDOW_MS + 1).is_empty(), "finished more than 24 hours ago");
    assert!(f.agents(ended + 10 * 60_000)[0].working, "moved ten minutes ago");
    assert!(!f.agents(ended + 20 * 60_000)[0].working, "quiet for twenty");
}

/// A terminal that exited is done, and an orchestrator that was removed is an
/// unnamed pane, not a vanished row.
#[test]
fn a_pane_that_exited_is_done() {
    let f = fixture_board();
    let pane = f.terminal();
    let turn = pane_turn("claude-log:p");
    f.record(&turn, Some(pane));
    let now = turn.ended_at_ms.unwrap() + 1000;
    assert!(find(&f.agents(now), &format!("pane:{pane}")).working);
    f.store.conn().execute("UPDATE terminals SET exit_code = 0 WHERE id = ?1", params![uuid_blob(pane)]).unwrap();
    assert!(!find(&f.agents(now), &format!("pane:{pane}")).working);
    f.store.conn().execute("DELETE FROM terminals WHERE id = ?1", params![uuid_blob(pane)]).unwrap();
    let gone = f.agents(now);
    assert_eq!((gone[0].role, gone[0].working), (PlanAgentRole::Agent, false));
}

/// The plan read carries the list, from the real clock.
#[test]
fn the_plan_read_carries_the_agents() {
    let f = fixture_board();
    let mut run = subagent_turn("a-now");
    run.ended_at_ms = Some(crate::tasks::now_millis());
    f.record(&run, None);
    assert!(f.store.plan(f.ws, 0).unwrap().agents.is_empty(), "only plan.get pays for the list");
    let plan = f.store.plan_with_agents(f.ws, 0, &|_| None).unwrap();
    assert_eq!(plan.agents.len(), 1);
    assert_eq!(plan.agents[0].id, "agent:a-now");
}

/// A second repository's turn, and a repository-less turn for a card on
/// another board, are not this board's agents.
#[test]
fn another_projects_agents_are_not_on_the_list() {
    let f = fixture_board();
    let mine = subagent_turn("a-mine");
    f.record(&mine, None);
    let other_repo = f.store.register_repository_for_test("elsewhere");
    let other_ws = f.store.ensure_main_workspace(other_repo).unwrap().id;
    let other_card = f.store.create_task(other_ws, "theirs", Actor::Manager).unwrap();
    let theirs = |key: &str, repo: Option<Uuid>, task: Option<Uuid>| {
        let run = subagent_turn(key);
        f.store
            .record_turn(&NewTurn {
                key: run.key.clone(),
                terminal_id: None,
                worktree_id: None,
                repository_id: repo,
                workspace_id: None,
                task_id: task,
                harness: "claude".into(),
                surface: Surface::Terminal,
                started_at: run.started_at_ms,
                ended_at: run.ended_at_ms.unwrap(),
                active_ms: None,
                usage: "reported",
                models: run.models.iter().map(|(m, c)| TurnModel::priced(m.clone(), *c, None)).collect(),
                kind: TurnKind::Subagent,
            })
            .unwrap();
    };
    theirs("a-other-repo", Some(other_repo), None);
    theirs("a-other-board", None, Some(other_card.id));
    // A repository-less turn for this board's own card does belong.
    theirs("a-mine-no-repo", None, Some(f.cards[0].id));
    let mut ids: Vec<String> = f.agents(mine.ended_at_ms.unwrap() + 60_000).into_iter().map(|a| a.id).collect();
    ids.sort();
    assert_eq!(ids, ["agent:a-mine", "agent:a-mine-no-repo"]);
}

/// Working agents come first, however little they spent.
#[test]
fn a_working_agent_sorts_before_a_finished_one_with_more_tokens() {
    let f = fixture_board();
    let big = subagent_turn("a-big");
    f.record(&big, None);
    let mut small = pane_turn("claude-log:small");
    small.models.iter_mut().for_each(|(_, c)| *c = farcooler_core::usage::TokenCounts { input: 1, ..Default::default() });
    let pane = f.terminal();
    f.record(&small, Some(pane));
    // The big run is 20 minutes old by now; the pane's small turn is recent.
    let now = big.ended_at_ms.unwrap() + 20 * 60_000;
    let mut small_late = small.clone();
    small_late.key = "claude-log:late".into();
    small_late.ended_at_ms = Some(now - 60_000);
    f.record(&small_late, Some(pane));
    let agents = f.agents(now);
    assert_eq!(agents.iter().map(|a| a.working).collect::<Vec<_>>(), [true, false], "{agents:#?}");
    assert!(total(&agents[0].spend) < total(&agents[1].spend));
}

/// A recorded subagent that went quiet is done, and so is a build record once
/// its lane is past building.
#[test]
fn an_open_record_alone_does_not_make_an_agent_work() {
    let f = fixture_board();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    let record = |agent: &str, role: AgentRole| {
        f.store
            .record_lane_agent(lane, &AgentRecord { harness: "claude".into(), agent_id: agent.into(), role, model: None, ended: false }, Actor::Manager)
            .unwrap();
    };
    record("a-builder", AgentRole::Build);
    record("a-reviewer", AgentRole::Review);
    let run = subagent_turn("a-builder");
    f.record(&run, None);
    f.record(&subagent_turn("a-reviewer"), None);
    let now = run.ended_at_ms.unwrap() + 60_000;
    let working = |now: i64, id: &str| find(&f.agents(now), id).working;
    assert!(working(now, "agent:a-builder"), "open record, moved a minute ago");
    assert!(!working(now + 30 * 60_000, "agent:a-builder"), "open record, quiet for half an hour");
    f.store.conn().execute("UPDATE lanes SET state = 'review'", []).unwrap();
    assert!(!working(now, "agent:a-builder"), "a build record is done once the lane is past building");
    assert!(working(now, "agent:a-reviewer"), "a review record stays open in review");
}

/// A train that landed has no integrator working, and a pane's idle
/// orchestrator is idle.
#[test]
fn a_landed_trains_integrator_and_an_idle_orchestrator_are_not_working() {
    let f = fixture_board();
    let train = f
        .store
        .start_train(
            f.ws,
            &NewTrain {
                name: "train-86".into(),
                agent: Some(TrainAgentRecord { harness: "claude".into(), agent_id: "a-train".into(), model: None, ended: false }),
                ..Default::default()
            },
            &[],
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-train");
    f.record(&run, None);
    let now = run.ended_at_ms.unwrap() + 60_000;
    assert_eq!(find(&f.agents(now), "agent:a-train").role, PlanAgentRole::Integrator);
    assert!(find(&f.agents(now), "agent:a-train").working);
    f.store.conn().execute("UPDATE board_trains SET state = 'landed' WHERE id = ?1", params![uuid_blob(train.id)]).unwrap();
    let after = find(&f.agents(now), "agent:a-train").clone();
    assert_eq!((after.role, after.train, after.working), (PlanAgentRole::Subagent, None, true), "unrecorded and recent: still moving, but not an integrator");

    let orch = f.terminal();
    f.store.set_terminal_role(orch, TerminalRole::Orchestrator).unwrap();
    let turn = pane_turn("claude-log:orch");
    f.record(&turn, Some(orch));
    let o = format!("pane:{orch}");
    assert!(find(&f.agents(turn.ended_at_ms.unwrap() + 60_000), &o).working);
    assert!(!find(&f.agents(turn.ended_at_ms.unwrap() + 30 * 60_000), &o).working, "no turn in half an hour");
}

/// The orchestrator is the orchestrator even when a lane records its pane.
#[test]
fn the_orchestrator_is_named_before_any_lane() {
    let f = fixture_board();
    let orch = f.terminal();
    f.store.set_terminal_role(orch, TerminalRole::Orchestrator).unwrap();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    f.store
        .record_lane_agent(
            lane,
            &AgentRecord { harness: "claude".into(), agent_id: format!("pane:{orch}"), role: AgentRole::Build, model: None, ended: false },
            Actor::Manager,
        )
        .unwrap();
    let turn = pane_turn("claude-log:orch");
    f.record(&turn, Some(orch));
    assert_eq!(f.agents(turn.ended_at_ms.unwrap() + 1)[0].role, PlanAgentRole::Orchestrator);
}

/// A subagent no record names works for the lane of the pane that spawned it.
#[test]
fn a_subagent_inherits_the_lane_of_its_pane() {
    let f = fixture_board();
    let pane = f.terminal();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    f.store
        .record_lane_agent(
            lane,
            &AgentRecord { harness: "claude".into(), agent_id: format!("pane:{pane}"), role: AgentRole::Build, model: None, ended: false },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-child");
    f.record(&run, Some(pane));
    let a = find(&f.agents(run.ended_at_ms.unwrap() + 1), "agent:a-child").clone();
    assert_eq!(a.lane.map(|l| l.1), Some("mac-ux".to_string()));
    assert_eq!(a.role, PlanAgentRole::Build);
}

/// A lane that records a pane and that pane's subagent counts the subagent once.
#[test]
fn a_lane_does_not_count_a_recorded_subagent_twice() {
    let f = fixture_board();
    let pane = f.terminal();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    for id in [format!("pane:{pane}"), "a-child".to_string()] {
        f.store
            .record_lane_agent(
                lane,
                &AgentRecord { harness: "claude".into(), agent_id: id, role: AgentRole::Build, model: None, ended: false },
                Actor::Manager,
            )
            .unwrap();
    }
    let (p, c) = (pane_turn("claude-log:p"), subagent_turn("a-child"));
    f.record(&p, Some(pane));
    f.record(&c, Some(pane));
    let plan = f.store.plan(f.ws, 0).unwrap();
    let spend = plan.lanes.iter().find(|l| l.lane.id == lane).unwrap().spend;
    let total = spend.input_tokens + spend.output_tokens + spend.cache_read_tokens + spend.cache_write_tokens;
    assert_eq!(total, tokens_of(&p) + tokens_of(&c));
}

/// A pane in the middle of a long turn has no recent turn row, but the
/// runner's live state says it is working; with none, recency decides.
#[test]
fn a_pane_mid_turn_is_working_whatever_its_last_turn() {
    let f = fixture_board();
    let pane = f.terminal();
    let turn = pane_turn("claude-log:p");
    f.record(&turn, Some(pane));
    let id = format!("pane:{pane}");
    let forty_later = turn.ended_at_ms.unwrap() + 40 * 60_000;
    assert!(find(&f.agents_live(forty_later, &|t| (t == pane).then_some(true)), &id).working, "live: working");
    assert!(!find(&f.agents_live(forty_later, &|_| None), &id).working, "no live state: recency says idle");
    let soon = turn.ended_at_ms.unwrap() + 60_000;
    assert!(!find(&f.agents_live(soon, &|_| Some(false)), &id).working, "live idle beats a recent turn");
}

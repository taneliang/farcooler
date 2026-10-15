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
        self.fleet_live(now, live).agents
    }

    fn fleet_live(&self, now: i64, live: &dyn Fn(Uuid) -> Option<bool>) -> Fleet {
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

fn has(agents: &[PlanAgent], id: &str) -> bool {
    agents.iter().any(|a| a.id == id)
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

/// An unrecorded subagent that is still moving is on the list, with its
/// spend. Turns filed to no terminal are not an agent that exists, but their
/// spend is still in the day's total.
#[test]
fn an_agent_nothing_joins_is_listed_while_it_moves_and_its_spend_is_counted() {
    let f = fixture_board();
    let ghost = subagent_turn("a-ghost");
    let stray = pane_turn("claude-log:stray");
    f.record(&ghost, None);
    f.record(&stray, None);
    let now = ghost.ended_at_ms.unwrap().max(stray.ended_at_ms.unwrap()) + 60_000;

    let fleet = f.fleet_live(now, &|_| None);
    let g = find(&fleet.agents, "agent:a-ghost");
    assert_eq!(g.role, PlanAgentRole::Subagent);
    assert!(g.lane.is_none() && g.train.is_none() && g.card.is_none());
    assert_eq!(g.label, "", "no worker record, so no description to show");
    assert!(g.working, "it moved a minute ago and nothing says it stopped");
    assert!(total(&g.spend) > 0);
    assert!(!has(&fleet.agents, "turns:claude"), "turns filed to no terminal are no agent that exists");
    assert_eq!(fleet.agents.len(), 1);
    assert_eq!(total(&fleet.spend), tokens_of(&ghost) + tokens_of(&stray), "the day's total counts every turn");
    assert_eq!(fleet.spend.runs, 2);
}

/// Only a subagent that moved in the last quarter hour is on the list, and
/// the day's spend still counts the ones that stopped, until they are a day old.
#[test]
fn a_quiet_subagent_is_off_the_list_and_its_spend_stays_in_the_day() {
    let f = fixture_board();
    let run = subagent_turn("a-old");
    f.record(&run, None);
    let ended = run.ended_at_ms.unwrap();
    assert!(has(&f.agents(ended + 10 * 60_000), "agent:a-old"), "moved ten minutes ago");
    assert!(f.agents(ended + 20 * 60_000).is_empty(), "quiet for twenty: finished");
    assert_eq!(total(&f.fleet_live(ended + 20 * 60_000, &|_| None).spend), tokens_of(&run), "its spend is still the day's");
    assert_eq!(total(&f.fleet_live(ended + WINDOW_MS, &|_| None).spend), tokens_of(&run), "a turn exactly a day old is in");
    assert_eq!(total(&f.fleet_live(ended + WINDOW_MS + 1, &|_| None).spend), 0, "finished more than 24 hours ago");
}

/// A subagent the runner saw end (its worker record is closed) is not listed,
/// however fresh its last turn, and one resumed for another round is listed
/// Working again, whether the runner has reopened the record yet or not.
#[test]
fn a_finished_subagent_is_not_listed_and_a_resumed_one_is_working() {
    let f = fixture_board();
    let orch = f.terminal();
    let worker = |agent: &str| WorkerRecord {
        harness: "claude".into(),
        agent_id: agent.into(),
        session_id: None,
        session_cwd: None,
        orchestrator_terminal: Some(orch),
        label: Some(format!("Work of {agent}")),
        model: None,
        linked_by: LinkedBy::Orchestrator,
    };
    f.store.record_worker(f.cards[0].id, &worker("a-done"), Actor::Manager).unwrap();
    f.store.record_worker(f.cards[1].id, &worker("a-again"), Actor::Manager).unwrap();
    let (done, again) = (subagent_turn("a-done"), subagent_turn("a-again"));
    f.record(&done, Some(orch));
    f.record(&again, Some(orch));
    let ended = done.ended_at_ms.unwrap();
    let now = ended + 60_000;
    let end_at = |agent: &str, at: i64| {
        f.store.conn().execute("UPDATE task_workers SET ended_at = ?2 WHERE agent_id = ?1", params![agent, at]).unwrap();
    };
    assert!(has(&f.agents(now), "agent:a-done"), "open record, moved a minute ago");

    // The runner read the Agent call's result, or the task notification.
    end_at("a-done", ended + 1_000);
    end_at("a-again", ended + 1_000);
    let agents = f.agents(now);
    assert!(!has(&agents, "agent:a-done"), "the runner saw it end: {agents:#?}");
    assert!(!has(&agents, "agent:a-again"), "the runner saw it end: {agents:#?}");

    // `SendMessage` resumes one: the runner reopens its record.
    f.store.conn().execute("UPDATE task_workers SET ended_at = NULL WHERE agent_id = 'a-again'", []).unwrap();
    assert!(find(&f.agents(now), "agent:a-again").working, "resumed, record reopened");

    // The other is woken some way the runner didn't read: a turn after the end.
    end_at("a-done", ended - 5_000);
    let woken = find(&f.agents(now), "agent:a-done").clone();
    assert!(woken.working, "a turn after the record closed means it was resumed");
    assert_eq!(woken.label, "Work of a-done");
}

/// The runner's end is not overridden by a lane record the manager left open.
#[test]
fn a_finished_reviewer_the_lane_still_records_is_not_listed() {
    let f = fixture_board();
    let orch = f.terminal();
    let lane = f.lane("mac-ux", &[&f.cards[0]]);
    f.store
        .record_lane_agent(
            lane,
            &AgentRecord { harness: "claude".into(), agent_id: "a-review".into(), role: AgentRole::Review, model: None, ended: false },
            Actor::Manager,
        )
        .unwrap();
    f.store
        .record_worker(
            f.cards[0].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-review".into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: Some(orch),
                label: None,
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-review");
    f.record(&run, Some(orch));
    let now = run.ended_at_ms.unwrap() + 60_000;
    assert!(find(&f.agents(now), "agent:a-review").working);
    f.store.conn().execute("UPDATE task_workers SET ended_at = ?1", params![run.ended_at_ms.unwrap() + 1_000]).unwrap();
    assert!(!has(&f.agents(now), "agent:a-review"), "the lane record is open, but the runner saw it end");
}

/// A terminal that exited is not on the list, nor is one that was removed;
/// its turns stay in the day's spend.
#[test]
fn a_pane_that_exited_is_not_listed() {
    let f = fixture_board();
    let pane = f.terminal();
    let turn = pane_turn("claude-log:p");
    f.record(&turn, Some(pane));
    let now = turn.ended_at_ms.unwrap() + 1000;
    let id = format!("pane:{pane}");
    assert!(find(&f.agents(now), &id).working);
    f.store.conn().execute("UPDATE terminals SET exit_code = 0 WHERE id = ?1", params![uuid_blob(pane)]).unwrap();
    let fleet = f.fleet_live(now, &|_| Some(true));
    assert!(!has(&fleet.agents, &id), "the terminal exited, whatever the live state says");
    assert_eq!(total(&fleet.spend), tokens_of(&turn));
    f.store.conn().execute("DELETE FROM terminals WHERE id = ?1", params![uuid_blob(pane)]).unwrap();
    assert!(f.agents(now).is_empty(), "a removed terminal is no agent either");
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
    assert_eq!(total(&plan.agents_spend), total(&plan.agents[0].spend));
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

/// Working agents come first, however little they spent. An idle pane that
/// still runs is on the list, after them.
#[test]
fn a_working_agent_sorts_before_an_idle_one_with_more_tokens() {
    let f = fixture_board();
    let big = pane_turn("claude-log:big");
    let pane = f.terminal();
    f.record(&big, Some(pane));
    let mut small = subagent_turn("a-small");
    small.models.iter_mut().for_each(|(_, c)| *c = farcooler_core::usage::TokenCounts { input: 1, ..Default::default() });
    // The pane's big turn is 20 minutes old by now; the subagent's small run is recent.
    let now = big.ended_at_ms.unwrap() + 20 * 60_000;
    small.ended_at_ms = Some(now - 60_000);
    f.record(&small, None);
    let agents = f.agents(now);
    assert_eq!(agents.iter().map(|a| a.working).collect::<Vec<_>>(), [true, false], "{agents:#?}");
    assert!(total(&agents[0].spend) < total(&agents[1].spend));
}

/// A recorded subagent that went quiet is done, and so is a build record once
/// its lane is past building and not yet fixing (the builder is resumed for a
/// fix round).
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
    let working = |now: i64, id: &str| has(&f.agents(now), id);
    assert!(working(now, "agent:a-builder"), "open record, moved a minute ago");
    assert!(!working(now + 30 * 60_000, "agent:a-builder"), "open record, quiet for half an hour");
    f.store.conn().execute("UPDATE lanes SET state = 'review'", []).unwrap();
    assert!(!working(now, "agent:a-builder"), "a build record is done once the lane is past building");
    assert!(working(now, "agent:a-reviewer"), "a review record stays open in review");
    f.store.conn().execute("UPDATE lanes SET state = 'fixing'", []).unwrap();
    assert!(working(now, "agent:a-builder"), "the builder is resumed for the fix round");
}

/// A builder pane resumed for a fix round is listed and reads Working; one
/// whose lane is in review is listed only while it is mid-turn.
#[test]
fn a_builder_pane_resumed_for_a_fix_round_is_working() {
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
    let turn = pane_turn("claude-log:p");
    f.record(&turn, Some(pane));
    let long_after = turn.ended_at_ms.unwrap() + 3 * 60 * 60_000;
    let id = format!("pane:{pane}");
    let mid_turn = |t: Uuid| (t == pane).then_some(true);
    f.store.conn().execute("UPDATE lanes SET state = 'review'", []).unwrap();
    assert!(has(&f.agents_live(long_after, &mid_turn), &id), "in review but mid-turn: it exists and is working");
    assert!(!has(&f.agents_live(long_after, &|_| Some(false)), &id), "built, in review and idle: done");
    assert!(!has(&f.agents_live(long_after, &|_| None), &id), "built, in review, no live state: done");
    f.store.conn().execute("UPDATE lanes SET state = 'fixing'", []).unwrap();
    assert!(find(&f.agents_live(long_after, &mid_turn), &id).working, "fixing: the builder is working again");
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
    assert!(!has(&f.agents(now + 30 * 60_000), "agent:a-train"), "and quiet for half an hour, it's gone");

    let orch = f.terminal();
    f.store.set_terminal_role(orch, TerminalRole::Orchestrator).unwrap();
    let turn = pane_turn("claude-log:orch");
    f.record(&turn, Some(orch));
    let o = format!("pane:{orch}");
    assert!(find(&f.agents(turn.ended_at_ms.unwrap() + 60_000), &o).working);
    let idle = f.agents(turn.ended_at_ms.unwrap() + 30 * 60_000);
    assert!(!find(&idle, &o).working, "no turn in half an hour: idle, but its terminal runs, so it's listed");
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

/// A subagent with an open record is listed for as long as the pane that
/// spawned it runs, however long its last tool call takes (a train's full test
/// run writes nothing for many minutes); without a record, or once the parent
/// is gone, the quiet quarter hour ends it.
#[test]
fn a_long_tool_call_does_not_end_a_recorded_subagent() {
    let f = fixture_board();
    let orch = f.terminal();
    f.store
        .record_worker(
            f.cards[0].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-gate".into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: Some(orch),
                label: Some("Run the train's gate".into()),
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-gate");
    f.record(&run, Some(orch));
    f.record(&subagent_turn("a-loose"), Some(orch));
    let ended = run.ended_at_ms.unwrap();
    let forty = ended + 40 * 60_000;
    let agents = f.agents(forty);
    assert!(!find(&agents, "agent:a-gate").working, "open record, parent running, a 40-minute Bash call: listed, Idle");
    assert!(!has(&agents, "agent:a-loose"), "no record: the quiet quarter hour ends it");
    f.store.conn().execute("UPDATE terminals SET exit_code = 0 WHERE id = ?1", params![uuid_blob(orch)]).unwrap();
    assert!(!has(&f.agents(forty), "agent:a-gate"), "the parent exited: nothing keeps it");
}

/// The orchestrator's terminal is listed while it runs, however long idle,
/// with no turn in the window at all.
#[test]
fn an_orchestrator_idle_for_days_is_still_listed() {
    let f = fixture_board();
    let orch = f.terminal();
    f.store.set_terminal_role(orch, TerminalRole::Orchestrator).unwrap();
    let fleet = f.fleet_live(5 * WINDOW_MS, &|_| None);
    let o = find(&fleet.agents, &format!("pane:{orch}"));
    assert_eq!((o.role, o.working), (PlanAgentRole::Orchestrator, false));
    assert_eq!((o.last_at, o.started_at), (0, 0), "no turn in the window: no invented age");
    assert_eq!(o.harness, "claude", "the harness is the terminal's preset's");
    f.store.conn().execute("UPDATE terminals SET command_preset = 'codex' WHERE id = ?1", params![uuid_blob(orch)]).unwrap();
    assert_eq!(find(&f.agents(5 * WINDOW_MS), &format!("pane:{orch}")).harness, "codex", "not assumed Claude");
    f.store.conn().execute("UPDATE terminals SET exit_code = 0 WHERE id = ?1", params![uuid_blob(orch)]).unwrap();
    assert!(f.agents(5 * WINDOW_MS).is_empty(), "an exited orchestrator is gone");
}

/// A stop and a last turn at the same millisecond is a stop: the comparison is strict.
#[test]
fn a_stop_at_the_last_turn_is_a_stop() {
    let f = fixture_board();
    let orch = f.terminal();
    f.store
        .record_worker(
            f.cards[0].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-tie".into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: Some(orch),
                label: None,
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-tie");
    f.record(&run, Some(orch));
    let at = run.ended_at_ms.unwrap();
    f.store.conn().execute("UPDATE task_workers SET ended_at = ?1", params![at]).unwrap();
    assert!(!has(&f.agents(at + 60_000), "agent:a-tie"));
    f.store.conn().execute("UPDATE task_workers SET ended_at = ?1", params![at - 1]).unwrap();
    assert!(has(&f.agents(at + 60_000), "agent:a-tie"), "one millisecond earlier it was resumed");
}

/// An open record's liveness is the parent's session, not its terminal, and it
/// is capped: after a /clear the worker's session is not the one the terminal
/// runs, and nothing keeps its subagent; within the session it is Idle past
/// the quiet quarter hour and gone after two hours.
#[test]
fn an_open_record_is_tied_to_the_parent_session_and_capped() {
    let f = fixture_board();
    let orch = f.terminal();
    f.store
        .conn()
        .execute("UPDATE terminals SET agent_session_id = 'session-1' WHERE id = ?1", params![uuid_blob(orch)])
        .unwrap();
    f.store
        .record_worker(
            f.cards[0].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-gate".into(),
                session_id: Some("session-1".into()),
                session_cwd: None,
                orchestrator_terminal: Some(orch),
                label: None,
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-gate");
    f.record(&run, Some(orch));
    let ended = run.ended_at_ms.unwrap();
    let minutes = |m: i64| ended + m * 60_000;
    assert!(find(&f.agents(minutes(5)), "agent:a-gate").working, "moving: working");
    let quiet = f.agents(minutes(40));
    assert!(!find(&quiet, "agent:a-gate").working, "quiet 40 minutes in its session: listed, but Idle");
    assert!(!has(&f.agents(minutes(3 * 60)), "agent:a-gate"), "the 2-hour cap");
    // /clear (or a relaunch) in the same pane: the terminal now runs another session.
    f.store
        .conn()
        .execute("UPDATE terminals SET agent_session_id = 'session-2' WHERE id = ?1", params![uuid_blob(orch)])
        .unwrap();
    assert!(!has(&f.agents(minutes(40)), "agent:a-gate"), "its parent session is gone, though the terminal runs");
    assert!(has(&f.agents(minutes(5)), "agent:a-gate"), "still moving: the quarter hour keeps it");
}

/// A subagent the runner linked from its description names no terminal, and its
/// turns are filed to none: the parent is the running terminal that runs the
/// worker's session, and the same rule holds it through a long tool call.
#[test]
fn a_description_linked_subagent_stays_listed_through_a_long_tool_call() {
    let f = fixture_board();
    let pane = f.terminal();
    f.store
        .conn()
        .execute("UPDATE terminals SET agent_session_id = 'session-1' WHERE id = ?1", params![uuid_blob(pane)])
        .unwrap();
    f.store
        .record_worker(
            f.cards[0].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a-desc".into(),
                session_id: Some("session-1".into()),
                session_cwd: None,
                orchestrator_terminal: None,
                label: None,
                model: None,
                linked_by: LinkedBy::Description,
            },
            Actor::Manager,
        )
        .unwrap();
    let run = subagent_turn("a-desc");
    f.record(&run, None);
    let minutes = |m: i64| run.ended_at_ms.unwrap() + m * 60_000;
    assert!(find(&f.agents(minutes(5)), "agent:a-desc").working, "moving: working");
    assert!(!find(&f.agents(minutes(40)), "agent:a-desc").working, "40 quiet minutes in a tool call: listed, Idle");
    assert!(!has(&f.agents(minutes(3 * 60)), "agent:a-desc"), "the 2-hour cap holds");
    // The binding lags: no terminal runs that session yet, so nothing is held on a guess.
    f.store.conn().execute("UPDATE terminals SET agent_session_id = NULL WHERE id = ?1", params![uuid_blob(pane)]).unwrap();
    assert!(!has(&f.agents(minutes(40)), "agent:a-desc"), "no running terminal is bound to its session");
    assert!(has(&f.agents(minutes(5)), "agent:a-desc"), "still moving: the quarter hour keeps it");
    // The binding catches up; the pane exits, and nothing keeps it.
    f.store
        .conn()
        .execute("UPDATE terminals SET agent_session_id = 'session-1', exit_code = 0 WHERE id = ?1", params![uuid_blob(pane)])
        .unwrap();
    assert!(!has(&f.agents(minutes(40)), "agent:a-desc"), "the parent exited");
}

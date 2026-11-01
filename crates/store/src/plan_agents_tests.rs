//! The agents on the plan (ov-872): recorded agents still at work, with their
//! tickets, whether they are working, and their spend in the window.

use super::*;

use farcooler_core::usage::TokenCounts;
use rusqlite::params;

use crate::agents::NewAgent;
use crate::models::{Actor, Task, TerminalRole};
use crate::store::Store;
use crate::ticket_work::{LinkedBy, Verdict, WorkRole};
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};

const MIN: i64 = 60_000;

struct Board {
    store: Store,
    ws: Uuid,
    repo: Uuid,
    worktree: Uuid,
    cards: Vec<Task>,
}

fn board() -> Board {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let cards = (0..2).map(|i| store.create_task(ws, &format!("card {i}"), Actor::Manager).unwrap()).collect();
    let worktree = store.create_worktree(repo, "main", "/wt/main", true).unwrap().id;
    Board { store, ws, repo, worktree, cards }
}

impl Board {
    fn terminal(&self) -> Uuid {
        self.store.create_terminal_for_test(self.worktree, self.ws)
    }

    fn record(&self, role: AgentRole, place: AgentPlace, session: Option<&str>) -> Agent {
        let new = NewAgent {
            role,
            place,
            harness: "claude".into(),
            session_id: session.map(str::to_string),
            session_cwd: None,
            model: Some("opus".into()),
            title: None,
        };
        self.store.record_agent(self.ws, &new).unwrap()
    }

    fn subagent(&self, id: &str, parent: Option<Uuid>, session: Option<&str>) -> Agent {
        self.record(AgentRole::Builder, AgentPlace::Subagent { id: id.into(), parent }, session)
    }

    fn pane(&self, role: AgentRole) -> (Uuid, Agent) {
        let terminal = self.terminal();
        (terminal, self.record(role, AgentPlace::Pane { terminal }, None))
    }

    /// A turn of `tokens` ending at `ended_at`: a subagent's run under its key,
    /// or a pane's turn in its terminal.
    fn turn(&self, key: &str, terminal: Option<Uuid>, kind: TurnKind, tokens: u64, ended_at: i64) {
        self.store
            .record_turn(&NewTurn {
                key: key.into(),
                terminal_id: terminal,
                worktree_id: None,
                repository_id: Some(self.repo),
                workspace_id: None,
                task_id: None,
                harness: "claude".into(),
                surface: Surface::Terminal,
                started_at: Some(ended_at - MIN),
                ended_at,
                active_ms: None,
                usage: "reported",
                models: vec![TurnModel::priced(
                    Some("claude-opus-5".into()),
                    TokenCounts { input: tokens, output: 0, cache_read: 0, cache_write: 0, cache_write_1h: 0, fast: false },
                    Some(1_000),
                )],
                kind,
            })
            .unwrap();
    }

    fn fleet(&self, now: i64, live: &dyn Fn(Uuid) -> Option<bool>) -> Fleet {
        agents_of(&self.store.conn(), self.ws, now, live).unwrap()
    }

    fn listed(&self, now: i64) -> Vec<Uuid> {
        self.fleet(now, &|_| None).agents.into_iter().map(|a| a.agent.id).collect()
    }

    fn exit(&self, terminal: Uuid) {
        self.store.conn().execute("UPDATE terminals SET exit_code = 0 WHERE id = ?1", params![uuid_blob(terminal)]).unwrap();
    }
}

/// A pane is listed while its terminal runs, working by the runner's live
/// state, and gone once it exits, whatever the live state says.
#[test]
fn a_pane_is_listed_while_its_terminal_runs() {
    let b = board();
    let (terminal, pane) = b.pane(AgentRole::Builder);
    let now = crate::tasks::now_millis();
    let fleet = b.fleet(now, &|_| Some(true));
    assert_eq!(fleet.agents.len(), 1);
    assert!(fleet.agents[0].working);
    assert!(!b.fleet(now, &|_| Some(false)).agents[0].working);
    b.exit(terminal);
    assert!(b.fleet(now, &|_| Some(true)).agents.is_empty(), "{:?} exited", pane.id);
}

/// A subagent is listed while it moved lately, or, with its parent's pane still
/// running the session it was recorded under, for two hours; then it is gone.
/// One just recorded, with no turn yet, is listed and working.
#[test]
fn a_subagent_is_listed_while_it_runs() {
    let b = board();
    let parent = b.terminal();
    b.store.conn().execute("UPDATE terminals SET agent_session_id = 's-1' WHERE id = ?1", params![uuid_blob(parent)]).unwrap();
    let now = crate::tasks::now_millis();
    let fresh = b.subagent("a-fresh", Some(parent), Some("s-1"));
    assert_eq!(b.listed(now), [fresh.id]);
    assert!(b.fleet(now, &|_| None).agents[0].working, "just dispatched");

    let gating = b.subagent("a-gate", Some(parent), Some("s-1"));
    b.turn("claude-log:agent:a-gate", None, TurnKind::Subagent, 100, now);
    let forty = now + 40 * MIN;
    let listed = b.fleet(forty, &|_| None).agents;
    let gate = listed.iter().find(|a| a.agent.id == gating.id).expect("held by its running parent");
    assert!(!gate.working, "quiet past a quarter hour");
    assert!(b.listed(now + 3 * 60 * MIN).is_empty(), "two hours with nothing: gone");

    // Another session in the same pane: the old one's subagents are gone.
    b.store.conn().execute("UPDATE terminals SET agent_session_id = 's-2' WHERE id = ?1", params![uuid_blob(parent)]).unwrap();
    assert!(!b.listed(forty).contains(&gating.id));
    let _ = fresh;
}

/// An ended agent is not listed; its spend is still the day's.
#[test]
fn an_ended_agent_leaves_the_list_but_not_the_day_s_spend() {
    let b = board();
    let a = b.subagent("a1", None, None);
    let now = crate::tasks::now_millis();
    b.turn("claude-log:agent:a1", None, TurnKind::Subagent, 500, now);
    assert_eq!(b.listed(now), [a.id]);
    b.store.end_agent(a.id, Verdict::Finished).unwrap();
    let fleet = b.fleet(now, &|_| None);
    assert!(fleet.agents.is_empty());
    assert_eq!(fleet.spend.input_tokens, 500);
    assert_eq!(fleet.spend.cost_micros, Some(1_000));
}

/// An agent carries the tickets it works now, not the ones it finished, and
/// its own spend in the window: a pane's turns without the runs of subagents
/// recorded as agents of their own.
#[test]
fn an_agent_carries_its_open_tickets_and_its_own_spend() {
    let b = board();
    let (terminal, pane) = b.pane(AgentRole::Builder);
    for card in &b.cards {
        b.store.start_work(pane.id, card.id, WorkRole::Build, LinkedBy::Orchestrator, Actor::Manager).unwrap();
    }
    b.store.end_work(pane.id, b.cards[0].id, WorkRole::Build, Verdict::Finished, Actor::Manager).unwrap();
    let sub = b.subagent("s1", Some(terminal), None);
    let now = crate::tasks::now_millis();
    b.turn(&format!("claude-log:{terminal}"), Some(terminal), TurnKind::Turn, 300, now);
    b.turn("claude-log:agent:s1", Some(terminal), TurnKind::Subagent, 900, now);
    let fleet = b.fleet(now, &|_| Some(false));
    let row = fleet.agents.iter().find(|a| a.agent.id == pane.id).unwrap();
    assert_eq!(row.tickets.iter().map(|w| w.task_id).collect::<Vec<_>>(), [b.cards[1].id]);
    assert_eq!(row.spend.input_tokens, 300, "the subagent's run is its own");
    let child = fleet.agents.iter().find(|a| a.agent.id == sub.id).unwrap();
    assert_eq!(child.spend.input_tokens, 900);
    assert_eq!(fleet.spend.input_tokens, 1200, "the day counts every turn once");
}

/// Working agents first, the orchestrator before the rest, then by tokens.
#[test]
fn working_first_then_the_orchestrator_then_tokens() {
    let b = board();
    let (orch_terminal, orch) = b.pane(AgentRole::Orchestrator);
    b.store.set_terminal_role(orch_terminal, TerminalRole::Orchestrator).unwrap();
    let small = b.subagent("small", None, None);
    let big = b.subagent("big", None, None);
    let now = crate::tasks::now_millis();
    b.turn("claude-log:agent:small", None, TurnKind::Subagent, 10, now - 20 * MIN);
    b.store.conn().execute("UPDATE agents SET started_at = ?1 WHERE id = ?2", params![now - 30 * MIN, uuid_blob(small.id)]).unwrap();
    b.turn("claude-log:agent:big", None, TurnKind::Subagent, 1000, now);
    let ids = b.fleet(now, &|_| Some(false)).agents.into_iter().map(|a| a.agent.id).collect::<Vec<_>>();
    assert_eq!(ids, [big.id, orch.id], "small is quiet, and gone with no parent to hold it");
    let _ = small;
}

/// A subagent named by nobody takes what its parent called it.
#[test]
fn a_subagent_untitled_reads_its_parent_s_description() {
    let b = board();
    b.subagent("d1", None, None);
    b.store.record_subagent_description("d1", "ov-9 sweep the board").unwrap();
    let now = crate::tasks::now_millis();
    assert_eq!(b.fleet(now, &|_| None).agents[0].label, "ov-9 sweep the board");
}

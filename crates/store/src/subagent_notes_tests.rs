//! A subagent's description on the plan's agents list (ov-509), from the
//! recorded parent call, meta file and transcript in
//! `crates/core/fixtures/session-logs/`, through the same readers the daemon
//! uses.

use rusqlite::{Connection, params};
use uuid::Uuid;

use super::*;
use crate::migrate::{MIGRATIONS, migrate, migrate_only_to};
use crate::models::{Actor, uuid_blob};
use crate::plan_agents::{PlanAgent, agents_of};
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use crate::workers::{LinkedBy, WorkerRecord};
use farcooler_core::session_log::subagents::SubagentLogs;
use farcooler_core::session_log::usage::{LogUsage, LoggedTurn};
use farcooler_core::session_log::{TurnEvent, claude};

/// The agent id the recorded transcript's records carry.
const AGENT: &str = "a0000000000000001";

fn fixtures() -> String {
    format!("{}/../core/fixtures/session-logs", env!("CARGO_MANIFEST_DIR"))
}

/// The recording laid out the way claude writes it: the parent's session file
/// beside `<session>/subagents/agent-<id>.{jsonl,meta.json}`. Read through the
/// pane follower's own `SubagentLogs`, so the meta file is found the way the
/// daemon finds it. Returns the subagent's run, and the scratch directory.
fn the_recorded_run() -> (LoggedTurn, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("farcooler-subagent-notes-{}", Uuid::now_v7()));
    let session = root.join("session.jsonl");
    let subagents = root.join("session").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    let copy = |from: &str, to: std::path::PathBuf| {
        std::fs::copy(format!("{}/{from}", fixtures()), to).unwrap();
    };
    copy("claude-subagent-parent.jsonl", session.clone());
    copy("claude-subagent-transcript.jsonl", subagents.join(format!("agent-{AGENT}.jsonl")));
    copy("claude-subagent-transcript.meta.json", subagents.join(format!("agent-{AGENT}.meta.json")));
    let mut usage = LogUsage::default();
    SubagentLogs::default().follow(&session, true, &mut usage);
    let mut turns = usage.take();
    assert_eq!(turns.len(), 1, "one subagent ran");
    (turns.remove(0), root)
}

struct Board {
    store: Store,
    ws: Uuid,
    repo: Uuid,
}

fn board() -> Board {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    Board { store, ws, repo }
}

impl Board {
    /// File `turn` as the daemon does, naming the agent when the run did.
    fn record(&self, turn: &LoggedTurn) {
        self.store
            .record_turn(&NewTurn {
                key: turn.key.clone(),
                terminal_id: None,
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
                kind: TurnKind::Subagent,
            })
            .unwrap();
        if let Some(description) = &turn.description {
            self.store.record_subagent_description(AGENT, description).unwrap();
        }
    }

    fn agent(&self, now: i64) -> PlanAgent {
        let agents = agents_of(&self.store.conn(), self.ws, now, &|_| None).unwrap().agents;
        assert_eq!(agents.len(), 1, "{agents:?}");
        agents.into_iter().next().unwrap()
    }
}

/// The description rides from the parent's `Agent` call, through the sibling
/// meta file the pane follower reads, to an agent that no lane, train or card
/// names.
#[test]
fn an_unjoined_subagent_shows_the_description_its_parent_gave_it() {
    let (run, root) = the_recorded_run();
    // The parent's own call says the same thing the meta file does.
    let parent = std::fs::read_to_string(format!("{}/claude-subagent-parent.jsonl", fixtures())).unwrap();
    let from_parent: Vec<String> = claude::parse_line(parent.lines().next().unwrap())
        .into_iter()
        .filter_map(|e| match e {
            TurnEvent::Subagent { description, .. } => Some(description),
            _ => None,
        })
        .collect();
    assert_eq!(from_parent, ["Sleep 200 then report"]);
    assert_eq!(run.description.as_deref(), Some("Sleep 200 then report"));

    let b = board();
    b.record(&run);
    let agent = b.agent(run.ended_at_ms.unwrap() + 60_000);
    assert_eq!(agent.id, format!("agent:{AGENT}"));
    assert!(agent.lane.is_none() && agent.train.is_none() && agent.card.is_none(), "nothing joins it");
    assert_eq!(agent.label, "Sleep 200 then report");
    std::fs::remove_dir_all(root).unwrap();
}

/// A worker record's label is what the orchestrator typed for the card, and
/// wins over the description the parent's call carried.
#[test]
fn a_worker_label_still_wins() {
    let (run, root) = the_recorded_run();
    let b = board();
    b.record(&run);
    let card = b.store.create_task(b.ws, "a card", Actor::Manager).unwrap();
    b.store
        .record_worker(
            card.id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: AGENT.into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: None,
                label: Some("Design ov-503".into()),
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    let agent = b.agent(run.ended_at_ms.unwrap() + 60_000);
    assert_eq!(agent.label, "Design ov-503");
    assert_eq!(agent.card.map(|c| c.1), Some(card.key));
    std::fs::remove_dir_all(root).unwrap();
}

/// A worker record with no label (a resume's reopen writes none) doesn't hide
/// the description behind an empty string.
#[test]
fn an_empty_worker_label_falls_back_to_the_description() {
    let (run, root) = the_recorded_run();
    let b = board();
    b.record(&run);
    let card = b.store.create_task(b.ws, "a card", Actor::Manager).unwrap();
    let record = WorkerRecord {
        harness: "claude".into(),
        agent_id: AGENT.into(),
        session_id: None,
        session_cwd: None,
        orchestrator_terminal: None,
        label: None,
        model: None,
        linked_by: LinkedBy::Orchestrator,
    };
    b.store.record_worker(card.id, &record, Actor::Manager).unwrap();
    assert_eq!(b.agent(run.ended_at_ms.unwrap() + 60_000).label, "Sleep 200 then report");
    std::fs::remove_dir_all(root).unwrap();
}

/// A subagent whose meta file was never read has no name, as before.
#[test]
fn a_subagent_without_a_description_has_no_label() {
    let (mut run, root) = the_recorded_run();
    run.description = None;
    let b = board();
    b.record(&run);
    assert_eq!(b.agent(run.ended_at_ms.unwrap() + 60_000).label, "");
    std::fs::remove_dir_all(root).unwrap();
}

/// The latest description wins, and blank text names nothing.
#[test]
fn the_latest_description_wins_and_blank_changes_nothing() {
    let b = board();
    b.store.record_subagent_description("a1", "  First\n  try ").unwrap();
    assert_eq!(b.store.subagent_description("a1").unwrap().as_deref(), Some("First try"));
    b.store.record_subagent_description("a1", "Second try").unwrap();
    b.store.record_subagent_description("a1", "   ").unwrap();
    assert_eq!(b.store.subagent_description("a1").unwrap().as_deref(), Some("Second try"));
    assert_eq!(b.store.subagent_description("nobody").unwrap(), None);
}

/// A database from before the table migrates cleanly: its rows are as they
/// were, the plan lists its subagent with no label, and a description can then
/// be kept.
#[test]
fn an_old_database_migrates_and_lists_its_subagents_unlabeled() {
    let ours = MIGRATIONS
        .iter()
        .position(|(m, _)| std::ptr::fn_addr_eq(*m, migration_0049_subagent_descriptions as fn(&rusqlite::Transaction) -> rusqlite::Result<()>))
        .expect("the migration is registered") as u32;
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
    migrate_only_to(&mut conn, ours);
    assert!(
        conn.prepare("SELECT 1 FROM subagent_descriptions").is_err(),
        "the old schema has no such table"
    );
    let (root, ident, repo, ws, turn) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let (r, i, rp, w, t) = (uuid_blob(root), uuid_blob(ident), uuid_blob(repo), uuid_blob(ws), uuid_blob(turn));
    conn.execute("INSERT INTO repository_roots VALUES (?1, ?2, '/r', 0, 1)", params![r, i]).unwrap();
    conn.execute("INSERT INTO repositories VALUES (?1, ?2, ?3, 'r', '/r/.git', '', 1, 'r')", params![rp, i, r]).unwrap();
    conn.execute(
        "INSERT INTO workspaces (id, repository_id, name, task_prefix, is_main, ordinal, created_at)
             VALUES (?1, ?2, 'Main', 'r', 1, 0, 0)",
        params![w, rp],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_turns (id, turn_key, repository_id, harness, surface, started_at, ended_at, usage, kind)
             VALUES (?1, 'claude-log:agent:old', ?2, 'claude', 'terminal', 1000, 2000, 'reported', 'subagent')",
        params![t, rp],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_turn_models (turn_id, model, input_tokens, output_tokens, cache_read_tokens,
             cache_write_tokens, cost_source) VALUES (?1, 'm', 1, 2, 3, 4, 'unknown')",
        params![t],
    )
    .unwrap();

    migrate(&mut conn, ours).unwrap();

    let agents = agents_of(&conn, ws, 60_000, &|_| None).unwrap().agents;
    assert_eq!(agents.len(), 1, "the old subagent is still listed");
    assert_eq!((agents[0].id.as_str(), agents[0].label.as_str()), ("agent:old", ""));
    conn.execute(
        "INSERT INTO subagent_descriptions (agent_id, description, recorded_at) VALUES ('old', 'Named late', 1)",
        [],
    )
    .unwrap();
    assert_eq!(agents_of(&conn, ws, 60_000, &|_| None).unwrap().agents[0].label, "Named late");
}

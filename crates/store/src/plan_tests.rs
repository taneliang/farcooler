use super::*;

use crate::models::NoteKind;
use crate::models::TaskStatus::{Done, InProgress, InReview};
use crate::plan_read::StatusCounts;
use crate::tasks::TaskScope;
use crate::usage::{NewTurn, Surface, TurnKind, TurnModel};
use crate::workers::{LinkedBy, WorkerRecord};
use farcooler_core::usage::TokenCounts;

use crate::models::Task;

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

fn builder(id: &str) -> AgentRecord {
    AgentRecord { harness: "claude".into(), agent_id: id.into(), role: AgentRole::Build, model: None, ended: false }
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn set_state(store: &Store, lane: &Lane, state: LaneState) -> Result<Lane> {
    store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager)
}

fn events(store: &Store, subject: Subject) -> Vec<String> {
    store.plan_events(subject, 0).unwrap().into_iter().map(|e| e.body).collect()
}

/// A lane with no agent is queued, and one with an agent is building.
#[test]
fn a_lane_starts_queued_or_building() {
    let (store, main, t) = board(2);
    let queued = lane(&store, main, "mac-fu3", &[&t[0]]);
    assert_eq!(queued.state, LaneState::Queued);
    let building = store
        .create_lane(main, &NewLane { name: "mac-ux".into(), ..Default::default() }, &[], Some(&builder("a1")), Actor::Manager)
        .unwrap();
    assert_eq!(building.state, LaneState::Building);
    let plan = store.plan(main, 0).unwrap();
    let view = plan.lanes.iter().find(|l| l.lane.id == building.id).unwrap();
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].role, AgentRole::Build);
}

/// States move forward with two loops, and any live state may be dropped; a
/// finished lane takes no more moves.
#[test]
fn lane_states_move_forward_with_two_loops() {
    let (store, main, t) = board(1);
    let l = lane(&store, main, "a", &[&t[0]]);
    assert_eq!(refused(set_state(&store, &l, LaneState::Review)), "lane_state", "queued can't skip building");
    set_state(&store, &l, LaneState::Building).unwrap();
    assert_eq!(refused(set_state(&store, &l, LaneState::Landed)), "lane_state");
    set_state(&store, &l, LaneState::Review).unwrap();
    set_state(&store, &l, LaneState::Fixing).unwrap();
    set_state(&store, &l, LaneState::Review).unwrap();
    set_state(&store, &l, LaneState::Landing).unwrap();
    set_state(&store, &l, LaneState::Fixing).unwrap();
    set_state(&store, &l, LaneState::Review).unwrap();
    set_state(&store, &l, LaneState::Landing).unwrap();
    let landed = set_state(&store, &l, LaneState::Landed).unwrap();
    assert_eq!(landed.state, LaneState::Landed);
    assert_eq!(refused(set_state(&store, &l, LaneState::Dropped)), "lane_closed");
    assert_eq!(refused(set_state(&store, &l, LaneState::Building)), "lane_closed");

    let d = lane(&store, main, "b", &[]);
    set_state(&store, &d, LaneState::Building).unwrap();
    assert_eq!(set_state(&store, &d, LaneState::Dropped).unwrap().state, LaneState::Dropped);
}

/// Fix rounds are derived from the moves into fixing, not stored.
#[test]
fn fix_rounds_are_counted_from_the_moves() {
    let (store, main, t) = board(1);
    let l = lane(&store, main, "a", &[&t[0]]);
    for s in [LaneState::Building, LaneState::Review, LaneState::Fixing, LaneState::Review, LaneState::Landing, LaneState::Fixing] {
        set_state(&store, &l, s).unwrap();
    }
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.lanes[0].fix_rounds, 2);
    let moves = store.plan_events(Subject::Lane(l.id), 0).unwrap().into_iter().filter(|e| e.kind == "state").count();
    assert_eq!(moves, 7, "one event per move, and the first");
}

/// A live lane name is unique on a board, ignoring case, and free again once
/// the lane has landed or dropped.
#[test]
fn a_live_lane_name_is_taken_until_it_closes() {
    let (store, main, _) = board(0);
    let a = lane(&store, main, "mac-ux", &[]);
    assert_eq!(
        refused(store.create_lane(main, &NewLane { name: "MAC-ux".into(), ..Default::default() }, &[], None, Actor::Manager)),
        "name_taken"
    );
    assert_eq!(refused(store.create_lane(main, &NewLane { name: "two words".into(), ..Default::default() }, &[], None, Actor::Manager)), "name");
    set_state(&store, &a, LaneState::Dropped).unwrap();
    lane(&store, main, "mac-ux", &[]);
}

/// The plan is replaced whole, only queued lanes can be in it, and a lane
/// that leaves queued loses its place in the same write.
#[test]
fn the_plan_is_replaced_whole_and_holds_only_queued_lanes() {
    let (store, main, _) = board(0);
    let [a, b, c] = ["a", "b", "c"].map(|n| lane(&store, main, n, &[]));
    store.set_plan(main, &[a.id, b.id, c.id], Actor::Manager).unwrap();
    let ranks = |store: &Store| -> Vec<(String, Option<u32>)> {
        store.plan(main, 0).unwrap().lanes.into_iter().map(|l| (l.lane.name, l.lane.plan_rank)).collect()
    };
    assert_eq!(ranks(&store), [("a".into(), Some(1)), ("b".into(), Some(2)), ("c".into(), Some(3))]);

    store.set_plan(main, &[c.id, a.id], Actor::Manager).unwrap();
    assert_eq!(ranks(&store), [("a".into(), Some(2)), ("b".into(), None), ("c".into(), Some(1))], "b is out, not behind");
    assert_eq!(store.plan(main, 0).unwrap().order, [c.id, a.id]);
    assert_eq!(events(&store, Subject::Lane(b.id)).last().unwrap(), "Out of the plan.");

    assert_eq!(refused(store.set_plan(main, &[a.id, a.id], Actor::Manager)), "lane_twice");
    set_state(&store, &a, LaneState::Building).unwrap();
    assert_eq!(refused(store.set_plan(main, &[a.id], Actor::Manager)), "plan_state");
    assert_eq!(store.plan(main, 0).unwrap().order, [c.id], "leaving queued cleared a's rank, and a refusal wrote nothing");
}

/// A card is in at most one theme: adding it to a second moves it, and both
/// themes' timelines say so.
#[test]
fn a_card_is_in_one_theme() {
    let (store, main, t) = board(2);
    let one = store.create_theme(main, &NewTheme { name: "Visual language".into(), outcome: "One app.".into() }, &[t[0].id, t[1].id], Actor::Manager).unwrap();
    let two = store.create_theme(main, &NewTheme { name: "Reliability".into(), outcome: String::new() }, &[], Actor::Manager).unwrap();
    store.theme_cards(two.id, &[t[1].id], &[], Actor::Manager).unwrap();
    let plan = store.plan(main, 0).unwrap();
    let tasks = |name: &str| plan.themes.iter().find(|v| v.theme.name == name).unwrap().tasks.clone();
    assert_eq!(tasks("Visual language"), [t[0].id]);
    assert_eq!(tasks("Reliability"), [t[1].id]);
    assert!(events(&store, Subject::Theme(one.id)).iter().any(|e| e.contains("moved to Reliability")));
    assert_eq!(
        refused(store.create_theme(main, &NewTheme { name: "visual LANGUAGE".into(), outcome: String::new() }, &[], Actor::Manager)),
        "name_taken"
    );
}

/// Rewriting the story keeps the old one, so a reader can see what changed.
#[test]
fn rewriting_the_story_keeps_the_old_one() {
    let (store, main, _) = board(0);
    let theme = store.create_theme(main, &NewTheme { name: "T".into(), outcome: "o".into() }, &[], Actor::Manager).unwrap();
    assert_eq!(theme.story_at, 0);
    store.update_theme(theme.id, &ThemeUpdate { story: Some("First.".into()), ..Default::default() }, Actor::Manager).unwrap();
    let second = store.update_theme(theme.id, &ThemeUpdate { story: Some("Second.".into()), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(second.story, "Second.");
    assert!(second.story_at > 0);
    let kept: Vec<_> = store.plan_events(Subject::Theme(theme.id), 0).unwrap().into_iter().filter(|e| e.kind == "story").collect();
    assert_eq!(kept.iter().map(|e| e.body.as_str()).collect::<Vec<_>>(), ["", "First."], "each event holds the story it replaced");
    assert_eq!(kept[1].extra["to"], "Second.");
    // Touching another field leaves the story, and writes no story event.
    store.update_theme(theme.id, &ThemeUpdate { next: Some("Ship.".into()), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(store.plan_events(Subject::Theme(theme.id), 0).unwrap().iter().filter(|e| e.kind == "story").count(), 2);
}

/// A theme and a lane are on the card's own board.
#[test]
fn a_card_from_another_board_is_refused() {
    let (store, main, t) = board(1);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theme = store.create_theme(other.id, &NewTheme { name: "T".into(), outcome: String::new() }, &[], Actor::Manager).unwrap();
    assert_eq!(refused(store.theme_cards(theme.id, &[t[0].id], &[], Actor::Manager)), "other_board");
    assert_eq!(
        refused(store.create_lane(other.id, &NewLane { name: "l".into(), ..Default::default() }, &[LaneCard { task_id: t[0].id, slice: String::new() }], None, Actor::Manager)),
        "other_board"
    );
    assert!(matches!(store.theme_cards(theme.id, &[Uuid::now_v7()], &[], Actor::Manager), Err(DomainError::NotFound)));
}

/// `task move` needs no hook: a moved card drops out of every read at once,
/// and the next write on the board removes its rows.
#[test]
fn a_moved_card_leaves_its_theme_and_lanes() {
    let (store, main, t) = board(2);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theme = store.create_theme(main, &NewTheme { name: "T".into(), outcome: String::new() }, &[t[0].id, t[1].id], Actor::Manager).unwrap();
    lane(&store, main, "l", &[&t[0], &t[1]]);
    store.move_tasks(&[t[0].id], other.id, Actor::Manager).unwrap();

    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.themes[0].tasks, [t[1].id], "the read already ignores it");
    assert_eq!(plan.lanes[0].cards.iter().map(|c| c.task_id).collect::<Vec<_>>(), [t[1].id]);
    assert_eq!(plan.themes[0].counts.total(), 1);
    let rows = |sql: &str| -> i64 { store.conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(rows("SELECT count(*) FROM board_theme_tasks"), 2, "nothing wrote yet");

    store.theme_cards(theme.id, &[], &[], Actor::Manager).unwrap();
    assert_eq!(rows("SELECT count(*) FROM board_theme_tasks"), 1);
    assert_eq!(rows("SELECT count(*) FROM lane_tasks"), 1);
}

/// Progress and coverage are derived: statuses are the board's own, and a card
/// with all its lanes landed shows as such while its own status is untouched.
#[test]
fn progress_and_coverage_are_derived_from_the_board() {
    let (store, main, t) = board(3);
    store.create_theme(main, &NewTheme { name: "T".into(), outcome: String::new() }, &[t[0].id, t[1].id, t[2].id], Actor::Manager).unwrap();
    store.set_task_status(t[0].id, Done, Actor::Manager).unwrap();
    store.set_task_status(t[1].id, InProgress, Actor::Manager).unwrap();
    let a = lane(&store, main, "a", &[&t[1]]);
    let b = store
        .create_lane(main, &NewLane { name: "b".into(), ..Default::default() }, &[LaneCard { task_id: t[1].id, slice: "Mac".into() }], None, Actor::Manager)
        .unwrap();
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.themes[0].counts, StatusCounts { done: 1, in_progress: 1, backlog: 1, ..Default::default() });
    let cov = |plan: &crate::plan_read::Plan| plan.coverage.iter().find(|c| c.task_id == t[1].id).map(|c| (c.live, c.landed));
    assert_eq!(cov(&plan), Some((2, 0)), "one card, two lanes");

    for s in [LaneState::Building, LaneState::Review, LaneState::Landing, LaneState::Landed] {
        set_state(&store, &a, s).unwrap();
    }
    assert_eq!(cov(&store.plan(main, 0).unwrap()), Some((1, 1)), "b is still live, so the card isn't covered yet");
    set_state(&store, &b, LaneState::Dropped).unwrap();
    assert_eq!(cov(&store.plan(main, 0).unwrap()), Some((0, 1)), "LANDED-NOT-CLOSED: landed lanes, card still in progress");
    assert_eq!(store.get_task(t[1].id).unwrap().status, InProgress);
}

/// A finished lane shows only for a while.
#[test]
fn finished_lanes_age_out_of_the_read() {
    let (store, main, _) = board(0);
    let l = lane(&store, main, "a", &[]);
    set_state(&store, &l, LaneState::Dropped).unwrap();
    assert_eq!(store.plan(main, 0).unwrap().lanes.len(), 1);
    assert!(store.plan(main, crate::tasks::now_millis() + 1000).unwrap().lanes.is_empty());
}

fn run_turn(store: &Store, agent: &str, tokens: u64) {
    store
        .record_turn(&NewTurn {
            key: format!("claude-log:agent:{agent}"),
            terminal_id: None,
            worktree_id: None,
            repository_id: None,
            workspace_id: None,
            task_id: None,
            harness: "claude".into(),
            surface: Surface::Terminal,
            started_at: None,
            ended_at: 1000,
            active_ms: None,
            usage: "reported",
            models: vec![TurnModel::priced(
                Some("claude-opus-5".into()),
                TokenCounts { input: tokens, output: tokens / 2, cache_read: 0, cache_write: 0, cache_write_1h: 0 },
                Some(2_000),
            )],
            kind: TurnKind::Subagent,
        })
        .unwrap();
}

/// A lane's spend is the sum of its agents' runs and nothing else's; an agent
/// the runner has read no turn of is counted apart, never as zero.
#[test]
fn a_lane_s_spend_is_its_own_agents_runs() {
    let (store, main, t) = board(1);
    let l = store
        .create_lane(main, &NewLane { name: "a".into(), ..Default::default() }, &[LaneCard { task_id: t[0].id, slice: String::new() }], Some(&builder("a1")), Actor::Manager)
        .unwrap();
    store
        .update_lane(l.id, &LaneUpdate { state: Some(LaneState::Review), agent: Some(AgentRecord { role: AgentRole::Review, ..builder("a2") }), ..Default::default() }, Actor::Manager)
        .unwrap();
    store
        .record_lane_agent(l.id, &AgentRecord { harness: "codex".into(), agent_id: "c1".into(), role: AgentRole::Fix, model: None, ended: false }, Actor::Manager)
        .unwrap();
    run_turn(&store, "a1", 1000);
    run_turn(&store, "somebody-else", 99_999);

    let spend = store.plan(main, 0).unwrap().lanes[0].spend;
    assert_eq!((spend.input_tokens, spend.output_tokens, spend.runs), (1000, 500, 1));
    assert_eq!(spend.cost_micros, Some(2_000));
    assert_eq!(spend.unmeasured_agents, 2, "a2 has no turn yet, and codex reports none");
    run_turn(&store, "a2", 10);
    assert_eq!(store.plan(main, 0).unwrap().lanes[0].spend.unmeasured_agents, 1);
}

/// An agent recorded again is the same agent: ending it ends it, recording it
/// without `ended` reopens it.
#[test]
fn an_agent_recorded_twice_is_one_agent() {
    let (store, main, _) = board(0);
    let l = store.create_lane(main, &NewLane { name: "a".into(), ..Default::default() }, &[], Some(&builder("a1")), Actor::Manager).unwrap();
    store.record_lane_agent(l.id, &AgentRecord { ended: true, ..builder("a1") }, Actor::Manager).unwrap();
    let ended = |store: &Store| store.plan(main, 0).unwrap().lanes[0].agents.clone();
    assert_eq!(ended(&store).len(), 1);
    assert!(ended(&store)[0].ended_at.is_some());
    store.record_lane_agent(l.id, &builder("a1"), Actor::Manager).unwrap();
    assert!(ended(&store)[0].ended_at.is_none());
    assert_eq!(refused(store.record_lane_agent(l.id, &AgentRecord { harness: "gemini".into(), ..builder("x") }, Actor::Manager)), "harness");
}

/// The migration is `Welcome`, and the schema it stamps still lets the build
/// before it open the file.
#[test]
fn the_migration_is_welcome() {
    use crate::compat::Older;
    let last = crate::migrate::MIGRATIONS.last().unwrap();
    assert!(std::ptr::fn_addr_eq(last.0, migration_0023_plan_layer as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 23);
}

/// Nothing existing carries a column for the layer: every table old code
/// writes has the columns it had before the migration.
#[test]
fn no_existing_table_gained_a_column() {
    let store = Store::open_in_memory().unwrap();
    let conn = store.conn();
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'").unwrap();
    let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let ours = ["board_themes", "board_theme_tasks", "lanes", "lane_tasks", "lane_agents", "plan_events"];
    for table in tables.iter().filter(|t| !ours.contains(&t.as_str())) {
        let mut info = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
        let cols: Vec<String> = info.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert!(
            !cols.iter().any(|c| c.contains("lane") || c.contains("theme") || c.contains("plan_")),
            "{table} has a column for the plan layer: {cols:?}"
        );
    }
    let triggers: Vec<String> = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type = 'trigger'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for sql in triggers {
        assert!(!ours.iter().any(|t| sql.contains(t)), "a trigger touches the layer: {sql}");
    }
}

// ---- the removal drill (ov-268, section 8) ----

/// A board with everything on it: notes, a worker, a line, a blocker, and
/// then the layer on top.
fn populated() -> (Store, Uuid, Vec<Task>) {
    let (store, main, t) = board(4);
    store.add_note(t[0].id, NoteKind::Progress, Actor::Manager, "Dispatched in the mac-ux lane.", serde_json::json!({})).unwrap();
    store.set_task_status(t[1].id, InProgress, Actor::Manager).unwrap();
    store.set_task_status(t[2].id, InProgress, Actor::Manager).unwrap();
    store.set_task_status(t[2].id, InReview, Actor::Manager).unwrap();
    store
        .record_worker(
            t[1].id,
            &WorkerRecord {
                harness: "claude".into(),
                agent_id: "a1".into(),
                session_id: None,
                session_cwd: None,
                orchestrator_terminal: None,
                label: None,
                model: None,
                linked_by: LinkedBy::Orchestrator,
            },
            Actor::Manager,
        )
        .unwrap();
    store.set_line(main, crate::waits::TaskLine::Agent, &[t[3].id, t[0].id], Actor::Manager).unwrap();

    let theme = store
        .create_theme(main, &NewTheme { name: "Visual language".into(), outcome: "One app.".into() }, &[t[0].id, t[1].id], Actor::Manager)
        .unwrap();
    store.update_theme(theme.id, &ThemeUpdate { story: Some("Going.".into()), ..Default::default() }, Actor::Manager).unwrap();
    let a = store
        .create_lane(
            main,
            &NewLane { name: "mac-ux".into(), ..Default::default() },
            &[LaneCard { task_id: t[1].id, slice: "Mac".into() }, LaneCard { task_id: t[2].id, slice: String::new() }],
            Some(&builder("a1")),
            Actor::Manager,
        )
        .unwrap();
    set_state(&store, &a, LaneState::Review).unwrap();
    let q = lane(&store, main, "next-one", &[&t[3]]);
    store.set_plan(main, &[q.id], Actor::Manager).unwrap();
    (store, main, t)
}

fn repo_of(store: &Store, main: Uuid) -> Uuid {
    store.get_workspace(main).unwrap().repository_id
}

/// Everything the board reads that a task could depend on the layer for.
fn board_reads(store: &Store, main: Uuid, tasks: &[Task]) -> String {
    let mut out = String::new();
    out += &format!("{:#?}\n", store.list_tasks(TaskScope::Workspace(main), None).unwrap());
    for t in tasks {
        let task = store.get_task(t.id).unwrap();
        out += &format!("{task:#?}\n{:#?}\n", store.notes_for(t.id, None).unwrap());
        out += &format!("{:#?}\n", store.task_facts(std::slice::from_ref(&task)).unwrap());
        out += &format!("{:#?}\n", store.workers_for(t.id).unwrap());
    }
    out += &format!("{:#?}\n", store.search_notes(repo_of(store, main), "lane", None).unwrap());
    out
}

/// Drop the layer's six tables, children first.
fn drop_the_layer(store: &Store) {
    store
        .conn()
        .execute_batch(
            "DROP TABLE plan_events; DROP TABLE lane_agents; DROP TABLE lane_tasks; DROP TABLE lanes;
             DROP TABLE board_theme_tasks; DROP TABLE board_themes;",
        )
        .unwrap();
}

/// The drill: with the layer's tables gone, every board read is the same
/// bytes and every board write still works. `Err` names what broke.
fn drill(store: &Store, main: Uuid, tasks: &[Task]) -> std::result::Result<(), String> {
    drill_with(store, main, tasks, &|_| String::new())
}

/// The drill, with one more read to compare: how a test stands in for a board
/// read that came to depend on the layer.
fn drill_with(
    store: &Store,
    main: Uuid,
    tasks: &[Task],
    extra: &dyn Fn(&Store) -> String,
) -> std::result::Result<(), String> {
    let before = board_reads(store, main, tasks) + &extra(store);
    drop_the_layer(store);
    let after = board_reads(store, main, tasks) + &extra(store);
    if before != after {
        return Err("a board read changed when the layer went".into());
    }
    let new = store.create_task(main, "after", Actor::Manager).map_err(|e| format!("create_task: {e}"))?;
    store.set_task_status(tasks[0].id, InProgress, Actor::Manager).map_err(|e| format!("set_task_status: {e}"))?;
    store
        .add_note(tasks[0].id, NoteKind::Progress, Actor::Manager, "still writing", serde_json::json!({}))
        .map_err(|e| format!("add_note: {e}"))?;
    let repo = store.get_workspace(main).map_err(|e| e.to_string())?.repository_id;
    let other = store.create_workspace(repo, "Other", "ot").map_err(|e| format!("create_workspace: {e}"))?;
    store.move_tasks(&[new.id], other.id, Actor::Manager).map_err(|e| format!("move_tasks: {e}"))?;
    store.move_tasks(&[new.id], main, Actor::Manager).map_err(|e| format!("move_tasks back: {e}"))?;
    store.delete_workspace(other.id).map_err(|e| format!("delete_workspace: {e}"))?;
    Ok(())
}

/// Removing the layer changes nothing the board says, and every board write
/// still works.
#[test]
fn the_board_is_the_same_bytes_with_the_layer_dropped() {
    let (store, main, tasks) = populated();
    let rows: i64 = store.conn().query_row("SELECT count(*) FROM plan_events", [], |r| r.get(0)).unwrap();
    assert!(rows > 5, "the layer had something to remove");
    drill(&store, main, &tasks).unwrap();
}

/// The drill can fail: a trigger on `tasks` that writes into the layer is a
/// task depending on it, and the writes break once the tables are gone.
#[test]
fn the_drill_goes_red_when_a_task_write_depends_on_the_layer() {
    let (store, main, tasks) = populated();
    store
        .conn()
        .execute_batch(
            "CREATE TRIGGER injected AFTER UPDATE OF status ON tasks
             BEGIN UPDATE lanes SET reason = 'touched' WHERE state = 'queued'; END;",
        )
        .unwrap();
    let err = drill(&store, main, &tasks).unwrap_err();
    assert!(err.starts_with("set_task_status"), "{err}");
}

/// The drill can fail on a read too: a task list that joined `lane_tasks`
/// reads differently, or not at all, once the table is gone.
#[test]
fn the_drill_goes_red_when_a_read_depends_on_the_layer() {
    let (store, main, tasks) = populated();
    let joined = |store: &Store| -> String {
        store
            .conn()
            .query_row("SELECT count(*) FROM tasks t JOIN lane_tasks l ON l.task_id = t.id", [], |r| r.get::<_, i64>(0))
            .map(|n| n.to_string())
            .unwrap_or_else(|e| e.to_string())
    };
    let err = drill_with(&store, main, &tasks, &joined).unwrap_err();
    assert_eq!(err, "a board read changed when the layer went");
}

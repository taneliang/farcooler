//! The move from lanes and trains, run on a board built the way the runner
//! builds one today: through the old store calls, with every shape the live
//! board has (one-card and many-card lanes, a card worked twice, slices, a fix
//! round, a pane, a landed and a collecting train, worker records, budgets).

use super::*;

use rusqlite::OptionalExtension;

use crate::agents::{AgentPlace, AgentRole};
use crate::landings::{LandingState, Route};
use crate::models::{Actor, Task, TerminalRole};
use crate::plan::{AgentRecord, Lane, LaneCard, LaneState, LaneUpdate, NewLane, NewTheme, Subject};
use crate::ticket_lines::{LineSubject, Stage};
use crate::ticket_work::{LinkedBy, Verdict, WorkRole};
use crate::trains::{NewTrain, TrainAgentRecord, TrainState, TrainUpdate};
use crate::workers::{EndReason, LinkedBy as WorkerLinkedBy, WorkerRecord};
use crate::Store;

/// The board, and the ids the assertions name.
struct Board {
    store: Store,
    main: Uuid,
    t: Vec<Task>,
    orchestrator: Uuid,
    pane: Uuid,
    lanes: BTreeMap<&'static str, Lane>,
    theme: Uuid,
}

fn agent(harness: &str, id: &str, role: crate::plan::AgentRole) -> AgentRecord {
    AgentRecord { harness: harness.into(), agent_id: id.into(), role, model: Some("opus".into()), ended: false }
}

fn to(store: &Store, lane: &Lane, state: LaneState) {
    store.update_lane(lane.id, &LaneUpdate { state: Some(state), ..Default::default() }, Actor::Manager).unwrap();
}

/// The live board's shapes, built through the calls the runner makes.
fn realistic_board() -> Board {
    use crate::plan::AgentRole::{Build, Fix, Review};
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let worktree = store.create_worktree(repo, "main", "/wt/main", true).unwrap().id;
    let t: Vec<Task> = (0..11).map(|i| store.create_task(main, &format!("card {i}"), Actor::Manager).unwrap()).collect();
    let orchestrator = store.create_terminal_for_test(worktree, main);
    store.set_terminal_role(orchestrator, TerminalRole::Orchestrator).unwrap();
    let pane = store.create_terminal_for_test(worktree, main);

    let roles = NewTheme { name: "Roles".into(), outcome: "Agents, not lanes.".into() };
    let theme = store.create_theme(main, &roles, &[t[0].id, t[2].id], Actor::Manager).unwrap().id;

    let mk = |name: &str, cards: &[(usize, &str)], builder: Option<AgentRecord>| {
        let cards: Vec<LaneCard> = cards.iter().map(|(i, s)| LaneCard { task_id: t[*i].id, slice: s.to_string() }).collect();
        let new = NewLane {
            name: name.into(),
            branch: name.into(),
            worktree_path: format!("/wt/{name}"),
            reason: format!("why {name}"),
            ..Default::default()
        };
        store.create_lane(main, &new, &cards, builder.as_ref(), Actor::Manager).unwrap()
    };
    let mut lanes = BTreeMap::new();
    // Two queued one-card lanes, in the plan in the opposite order to their making.
    lanes.insert("store-1", mk("store-1", &[(0, "")], None));
    lanes.insert("mac-2", mk("mac-2", &[(1, "")], None));
    store.set_plan(main, &[lanes["mac-2"].id, lanes["store-1"].id], Actor::Manager).unwrap();
    // A two-card lane that went through review and is in a fix round.
    let web = mk("web-3", &[(2, ""), (3, "")], Some(agent("claude", "a-build", Build)));
    store.record_lane_agent(web.id, &agent("claude", "a-rev", Review), Actor::Manager).unwrap();
    to(&store, &web, LaneState::Review);
    to(&store, &web, LaneState::Fixing);
    store.record_lane_agent(web.id, &agent("codex", "a-fix", Fix), Actor::Manager).unwrap();
    store.set_budget(Subject::Lane(web.id), Some(50_000), Actor::Manager).unwrap();
    lanes.insert("web-3", web);
    // A lane landed by a train.
    let old = mk("old-5", &[(5, "")], Some(agent("claude", "b-build", Build)));
    to(&store, &old, LaneState::Review);
    to(&store, &old, LaneState::Landing);
    let lander = TrainAgentRecord { harness: "codex".into(), agent_id: "lander-1".into(), model: Some("gpt".into()), ended: false };
    let new = NewTrain { name: "train-70".into(), base: "origin/main".into(), card: Some(t[8].id), agent: Some(lander), ..Default::default() };
    let train = store.start_train(main, &new, &[old.id], Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { sha: Some("abc1234".into()), ..Default::default() }, Actor::Manager).unwrap();
    store.set_train(train.id, &TrainUpdate { state: Some(TrainState::Landed), ..Default::default() }, Actor::Manager).unwrap();
    lanes.insert("old-5", old);
    // One ready to land, built by a pane.
    let site = mk("site-4", &[(4, "")], Some(agent("claude", &crate::plan_panes::pane_agent_id(pane), Build)));
    to(&store, &site, LaneState::Review);
    to(&store, &site, LaneState::Landing);
    lanes.insert("site-4", site);
    // A card dropped in one lane and taken up again in another.
    let gone = mk("gone-6", &[(6, "")], None);
    to(&store, &gone, LaneState::Dropped);
    lanes.insert("gone-6", gone);
    lanes.insert("redo-7", mk("redo-7", &[(6, "")], Some(agent("claude", "c-build", Build))));
    // One card in two slices of one lane.
    lanes.insert("slices-8", mk("slices-8", &[(7, "a"), (7, "b")], None));
    // A lane with no card.
    lanes.insert("spike-9", mk("spike-9", &[], Some(agent("claude", "d-spike", Build))));

    // Worker records: the builder of web-3 recorded by the orchestrator, and a
    // subagent the runner linked from its description that finished.
    let worker = |task: Uuid, id: &str, label: &str, by: WorkerLinkedBy| {
        let record = WorkerRecord {
            harness: "claude".into(),
            agent_id: id.into(),
            session_id: Some("s-1".into()),
            session_cwd: Some("/wt/main".into()),
            orchestrator_terminal: Some(orchestrator),
            label: Some(label.into()),
            model: Some("sonnet".into()),
            linked_by: by,
        };
        store.record_worker(task, &record, Actor::Manager).unwrap();
    };
    worker(t[2].id, "a-build", "Lane web-3: build", WorkerLinkedBy::Orchestrator);
    worker(t[9].id, "w-9", "", WorkerLinkedBy::Description);
    store.end_worker(t[9].id, "claude", Some("w-9"), EndReason::Finished, Actor::Manager).unwrap();
    store.record_subagent_description("w-9", "ov-9 sweep").unwrap();
    store.record_subagent_description("a-rev", "Review web-3").unwrap();
    store.set_budget(Subject::Theme(theme), Some(900_000), Actor::Manager).unwrap();

    Board { store, main, t, orchestrator, pane, lanes, theme }
}

fn count(store: &Store, sql: &str) -> i64 {
    store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn table_exists(store: &Store, name: &str) -> bool {
    store
        .conn()
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |_| Ok(()))
        .optional()
        .unwrap()
        .is_some()
}

fn by_key<'a>(agents: &'a [crate::agents::Agent], key: &str) -> &'a crate::agents::Agent {
    agents
        .iter()
        .find(|a| match &a.place {
            AgentPlace::Subagent { id, .. } => id == key,
            AgentPlace::Pane { terminal } => format!("pane:{terminal}") == key,
        })
        .unwrap_or_else(|| panic!("no agent {key}"))
}

fn hex(id: Uuid) -> String {
    id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn tickets_take_their_lanes_stage_branch_and_plan_order() {
    let b = realistic_board();
    let (t, lanes) = (&b.t, &b.lanes);
    apply(&b.store);
    let line = |i: usize| b.store.ticket_line(t[i].id).unwrap();

    // Every card a lane works is one ticket; a lane with none adds none, and a
    // card no lane names isn't on the line.
    assert_eq!(b.store.ticket_lines(b.main).unwrap().len(), 8);
    for i in [8, 9, 10] {
        assert!(b.store.ticket_line(t[i].id).is_err(), "card {i} was in no lane");
    }

    // The plan, renumbered by ticket: mac-2 first, then store-1.
    assert_eq!((line(1).stage, line(1).plan_rank), (Stage::Waiting, Some(1)));
    assert_eq!((line(0).stage, line(0).plan_rank), (Stage::Waiting, Some(2)));
    assert_eq!(line(0).branch, "store-1");
    assert_eq!(line(0).worktree_path, "/wt/store-1");
    assert_eq!(line(0).reason, "why store-1");
    assert_eq!(line(0).same_branch, None, "a one-card lane is exactly one ticket");

    // A fix round is building; the two cards share the lane's branch.
    for i in [2, 3] {
        assert_eq!(line(i).stage, Stage::Building);
        assert_eq!(line(i).same_branch, Some(lanes["web-3"].id));
        assert_eq!(line(i).branch, "web-3");
    }
    assert_eq!(line(4).stage, Stage::Landing);
    assert_eq!(line(5).stage, Stage::Landed);
    assert_eq!(line(5).landed_sha.as_deref(), Some("abc1234"));
    // The card worked twice takes the lane it is still open in.
    assert_eq!(line(6).stage, Stage::Building);
    assert_eq!(line(6).branch, "redo-7");
    // Two slices of one card are one ticket, alone on its branch.
    assert_eq!((line(7).stage, line(7).plan_rank, line(7).same_branch), (Stage::Waiting, None, None));

    // Each ticket says where it came from.
    let moved = b.store.line_events(LineSubject::Ticket(t[2].id), 0).unwrap();
    assert!(moved.iter().any(|e| e.kind == "moved" && e.body == "Carried over from lane web-3."));
}

#[test]
fn agents_are_one_row_each_with_their_role_and_place() {
    let b = realistic_board();
    apply(&b.store);
    let agents = b.store.agents(b.main, true).unwrap();
    // a-build, a-rev, a-fix, b-build, lander-1, the site pane, c-build,
    // d-spike, w-9, and the orchestrator's pane.
    assert_eq!(agents.len(), 10, "{agents:#?}");

    let build = by_key(&agents, "a-build");
    assert_eq!(build.role, AgentRole::Builder);
    assert_eq!(build.harness, "claude");
    assert_eq!(build.title, "Lane web-3: build", "the worker record's label");
    assert_eq!(build.session_id.as_deref(), Some("s-1"));
    assert_eq!(build.place, AgentPlace::Subagent { id: "a-build".into(), parent: Some(b.orchestrator) });
    assert_eq!(build.ended_at, None);

    let review = by_key(&agents, "a-rev");
    assert_eq!(review.role, AgentRole::Reviewer);
    assert_eq!(review.title, "Review web-3", "what its parent called it");
    assert_eq!(by_key(&agents, "a-fix").role, AgentRole::Builder);
    assert_eq!(by_key(&agents, "a-fix").harness, "codex");

    let lander = by_key(&agents, "lander-1");
    assert_eq!((lander.role, lander.model.as_str()), (AgentRole::Lander, "gpt"));

    let pane = by_key(&agents, &format!("pane:{}", b.pane));
    assert_eq!(pane.place, AgentPlace::Pane { terminal: b.pane });
    assert_eq!(pane.role, AgentRole::Builder);
    assert_eq!(b.store.pane_agent(b.pane).unwrap().map(|a| a.id), Some(pane.id));

    let orchestrator = by_key(&agents, &format!("pane:{}", b.orchestrator));
    assert_eq!(orchestrator.role, AgentRole::Orchestrator);

    let finished = by_key(&agents, "w-9");
    assert!(finished.ended_at.is_some());
    assert_eq!(finished.end_reason, Some(Verdict::Finished));
    assert_eq!(finished.title, "ov-9 sweep");
}

#[test]
fn who_worked_what_comes_from_lanes_trains_and_worker_records() {
    let b = realistic_board();
    let t = &b.t;
    apply(&b.store);
    let agents = b.store.agents(b.main, true).unwrap();
    let roles = |i: usize| {
        let mut out: Vec<(String, WorkRole)> = b
            .store
            .work_on(t[i].id)
            .unwrap()
            .into_iter()
            .map(|w| {
                let a = agents.iter().find(|a| a.id == w.agent_id).unwrap();
                let key = match &a.place {
                    AgentPlace::Subagent { id, .. } => id.clone(),
                    AgentPlace::Pane { .. } => "pane".into(),
                };
                (key, w.role)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    // Each lane agent works every card of its lane; the worker record of the
    // same builder on the same card is the same row.
    let web = vec![("a-build".to_string(), WorkRole::Build), ("a-fix".into(), WorkRole::Fix), ("a-rev".into(), WorkRole::Review)];
    assert_eq!(roles(2), web);
    assert_eq!(roles(3), web);
    assert_eq!(roles(4), vec![("pane".to_string(), WorkRole::Build)]);
    assert_eq!(roles(6), vec![("c-build".to_string(), WorkRole::Build)]);
    // The train's agent lands the train's card.
    assert_eq!(roles(8), vec![("lander-1".to_string(), WorkRole::Land)]);
    // A worker linked from its description keeps that, and how it ended.
    let w9 = b.store.work_on(t[9].id).unwrap();
    assert_eq!(w9.len(), 1);
    assert_eq!((w9[0].linked_by, w9[0].verdict), (LinkedBy::Description, Some(Verdict::Finished)));
    assert!(w9[0].ended_at.is_some());
}

#[test]
fn trains_become_landings_with_their_numbers_and_tickets() {
    let b = realistic_board();
    let t = &b.t;
    let trains: Vec<(Vec<u8>, String)> = {
        let conn = b.store.conn();
        let mut stmt = conn.prepare("SELECT id, name FROM board_trains ORDER BY created_at").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    };
    let site_train: Vec<u8> = b
        .store
        .conn()
        .query_row(
            "SELECT t.id FROM lanes l JOIN board_trains t ON t.name = l.train COLLATE NOCASE WHERE l.name = 'site-4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let collecting = count(&b.store, "SELECT count(*) FROM board_trains WHERE collecting = 1 AND state NOT IN ('landed', 'dropped')");
    assert_eq!(collecting, 1, "the board has its collecting train");
    apply(&b.store);
    let landings = b.store.landings(b.main).unwrap();
    assert_eq!(landings.len(), trains.len());
    // Each keeps its train's id, and the number in its name.
    for (id, name) in &trains {
        let landing = landings.iter().find(|l| l.id.as_bytes().as_slice() == id.as_slice()).expect("its train's id");
        assert_eq!(Some(landing.number), train_number(name), "{name}");
    }
    let seventy = landings.iter().find(|l| l.number == 70).expect("train-70 is Landing 70");
    assert_eq!(seventy.state, LandingState::Landed);
    assert_eq!(seventy.route, Route::Push);
    assert_eq!(seventy.pushed_sha.as_deref(), Some("abc1234"));
    assert_eq!(seventy.tickets, vec![t[5].id]);
    let lander = b.store.agent(seventy.lander_id.expect("its agent lands it")).unwrap();
    assert_eq!(lander.role, AgentRole::Lander);
    // site-4, ready, is on the open train it was labelled with, and at most one
    // landing a board collects.
    let site = b.store.landing_of(t[4].id).unwrap().expect("site-4 is on a landing");
    assert_eq!(site.id.as_bytes().as_slice(), site_train.as_slice());
    assert!(!site.state.is_settled());
    assert_eq!(landings.iter().filter(|l| l.collecting).count(), 1, "it still collects");
    // A train's history is its landing's.
    let history = b.store.line_events(LineSubject::Landing(seventy.id), 0).unwrap();
    assert!(history.iter().any(|e| e.kind == "state"));
    assert!(history.iter().any(|e| e.kind == "ticket_on" && e.task_id == Some(t[5].id)));
}

#[test]
fn history_and_budgets_move_to_tickets() {
    let b = realistic_board();
    let (t, lanes) = (&b.t, &b.lanes);
    let lane_events = count(&b.store, &format!("SELECT count(*) FROM plan_events WHERE lane_id = x'{}'", hex(lanes["web-3"].id)));
    let theme_events = count(&b.store, "SELECT count(*) FROM plan_events WHERE theme_id IS NOT NULL");
    assert!(lane_events > 0 && theme_events > 0);
    apply(&b.store);
    // A lane's history is on each of its cards, naming the lane.
    for i in [2, 3] {
        let events = b.store.line_events(LineSubject::Ticket(t[i].id), 0).unwrap();
        let from_lane = events.iter().filter(|e| e.extra["lane"] == "web-3" && e.kind != "moved").count();
        assert_eq!(from_lane as i64, lane_events);
    }
    assert_eq!(b.store.line_events(LineSubject::Theme(b.theme), 0).unwrap().len() as i64, theme_events);
    // The lane's budget is on each of its tickets; the theme's stays.
    for i in [2, 3] {
        assert_eq!(count(&b.store, &format!("SELECT tokens FROM plan_budgets WHERE task_id = x'{}'", hex(t[i].id))), 50_000);
    }
    assert_eq!(count(&b.store, &format!("SELECT tokens FROM plan_budgets WHERE theme_id = x'{}'", hex(b.theme))), 900_000);
}

#[test]
fn the_old_tables_are_gone_and_the_board_is_untouched() {
    let b = realistic_board();
    let tasks_before = count(&b.store, "SELECT count(*) FROM tasks");
    let notes_before = count(&b.store, "SELECT count(*) FROM task_notes");
    apply(&b.store);
    for gone in ["lanes", "lane_tasks", "lane_agents", "plan_events", "board_trains", "train_events", "task_workers"] {
        assert!(!table_exists(&b.store, gone), "{gone} is dropped");
    }
    for kept in ["board_themes", "board_theme_tasks", "board_rulings", "board_ci", "plan_budgets", "subagent_descriptions"] {
        assert!(table_exists(&b.store, kept), "{kept} is kept");
    }
    assert_eq!(count(&b.store, "SELECT count(*) FROM tasks"), tasks_before);
    assert_eq!(count(&b.store, "SELECT count(*) FROM task_notes"), notes_before);
    let broken: Option<String> = b.store.conn().query_row("PRAGMA foreign_key_check", [], |r| r.get(0)).optional().unwrap();
    assert_eq!(broken, None, "no row points at nothing");
}

/// A train with no number in its name is numbered after the board's last, and
/// keeps its name as the landing's title.
#[test]
fn a_train_named_without_a_number_takes_the_next_one() {
    let b = realistic_board();
    b.store
        .conn()
        .execute_batch("UPDATE board_trains SET name = 'hotfix' WHERE name = 'train-70';
                        UPDATE lanes SET train = 'hotfix' WHERE train = 'train-70';")
        .unwrap();
    let highest = {
        let conn = b.store.conn();
        let mut stmt = conn.prepare("SELECT name FROM board_trains").unwrap();
        let names: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        names.iter().filter_map(|n| train_number(n)).max().unwrap_or(0)
    };
    apply(&b.store);
    let landings = b.store.landings(b.main).unwrap();
    let hotfix = landings.iter().find(|l| l.title == "hotfix").expect("its name is its title");
    assert_eq!(hotfix.number, highest + 1);
    assert_eq!(hotfix.tickets, vec![b.t[5].id]);
}

/// An empty board moves too, and the move is one-way: a second run fails
/// rather than doubling anything.
#[test]
fn an_empty_board_moves_and_the_move_runs_once() {
    let store = Store::open_in_memory().unwrap();
    apply(&store);
    assert!(table_exists(&store, "ticket_lines"));
    let conn = store.conn();
    let tx = conn.unchecked_transaction().unwrap();
    assert!((LINE_MIGRATION.0)(&tx).is_err(), "the tables it made are there");
    assert_eq!(LINE_MIGRATION.1, Older::Refused);
}

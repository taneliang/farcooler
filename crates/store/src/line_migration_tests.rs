//! The move from lanes and trains, run on a board built the way the runner
//! builds one today: through the old store calls, with every shape the live
//! board has (one-card and many-card lanes, a card worked twice, slices, a fix
//! round, a pane, a landed and a collecting train, worker records, budgets).

use super::*;

use rusqlite::{Connection, OptionalExtension};

use crate::agents::{AgentPlace, AgentRole};
use crate::landings::{LandingState, Route};
use crate::models::Task;
use crate::ticket_lines::{LineSubject, Stage};
use crate::ticket_work::{LinkedBy, Verdict, WorkRole};
use crate::Store;

/// The live board's shapes, built through the old store calls before the move
/// (one-card and many-card lanes, a card worked twice, slices, a fix round, a
/// pane, a landed and a collecting train, worker records, budgets), at schema
/// 57: `fixtures/before_the_line.sql` says how it was made.
const BEFORE: &str = include_str!("../fixtures/before_the_line.sql");

/// The board, and the ids the assertions name.
struct Board {
    store: Store,
    main: Uuid,
    t: Vec<Task>,
    orchestrator: Uuid,
    pane: Uuid,
    lanes: BTreeMap<String, Uuid>,
    theme: Uuid,
}

fn uuid(raw: Vec<u8>) -> Uuid {
    Uuid::from_slice(&raw).unwrap()
}

/// The old board, opened by this build, which moves it.
fn realistic_board() -> Board {
    realistic_board_with(|_| ()).0
}

/// The old board, with `before` run on it as it was, then opened by this
/// build, which moves it. What `before` answers comes back beside it.
fn realistic_board_with<T>(before: impl FnOnce(&Connection) -> T) -> (Board, T) {
    let dir = farcooler_core::scratch::dir("line-migration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.db", Uuid::now_v7()));
    let (seen, main, orchestrator, pane, lanes, theme) = {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(BEFORE).unwrap();
        let seen = before(&conn);
        let main = uuid(conn.query_row("SELECT id FROM workspaces", [], |r| r.get(0)).unwrap());
        let orchestrator = uuid(conn.query_row("SELECT id FROM terminals WHERE role = 2", [], |r| r.get(0)).unwrap());
        let pane = uuid(conn.query_row("SELECT id FROM terminals WHERE role <> 2", [], |r| r.get(0)).unwrap());
        let theme = uuid(conn.query_row("SELECT id FROM board_themes", [], |r| r.get(0)).unwrap());
        let mut stmt = conn.prepare("SELECT name, id FROM lanes").unwrap();
        let lanes: BTreeMap<String, Uuid> =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, uuid(r.get(1)?)))).unwrap().map(|r| r.unwrap()).collect();
        (seen, main, orchestrator, pane, lanes, theme)
    };
    let store = Store::open(&path).unwrap();
    let t: Vec<Task> = (1..=11).map(|n| store.tasks_with_key(None, &format!("ov-{n}")).unwrap().remove(0)).collect();
    (Board { store, main, t, orchestrator, pane, lanes, theme }, seen)
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
        assert_eq!(line(i).same_branch, Some(lanes["web-3"]));
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
    let count_on = |conn: &Connection, sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let (b, (trains, site_train, collecting)) = realistic_board_with(|conn| {
        let mut stmt = conn.prepare("SELECT id, name FROM board_trains ORDER BY created_at").unwrap();
        let trains: Vec<(Vec<u8>, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect();
        let site: Vec<u8> = conn
            .query_row(
                "SELECT t.id FROM lanes l JOIN board_trains t ON t.name = l.train COLLATE NOCASE WHERE l.name = 'site-4'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let collecting = count_on(conn, "SELECT count(*) FROM board_trains WHERE collecting = 1 AND state NOT IN ('landed', 'dropped')");
        (trains, site, collecting)
    });
    let t = &b.t;
    assert_eq!(collecting, 1, "the board has its collecting train");
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
    let (b, (lane_events, theme_events)) = realistic_board_with(|conn| {
        let web: Vec<u8> = conn.query_row("SELECT id FROM lanes WHERE name = 'web-3'", [], |r| r.get(0)).unwrap();
        let lane: i64 = conn.query_row("SELECT count(*) FROM plan_events WHERE lane_id = ?1", [web], |r| r.get(0)).unwrap();
        let theme: i64 = conn.query_row("SELECT count(*) FROM plan_events WHERE theme_id IS NOT NULL", [], |r| r.get(0)).unwrap();
        (lane, theme)
    });
    let t = &b.t;
    assert!(lane_events > 0 && theme_events > 0);
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
    let (b, (tasks_before, notes_before)) = realistic_board_with(|conn| {
        let n = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        (n("SELECT count(*) FROM tasks"), n("SELECT count(*) FROM task_notes"))
    });
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
    let (b, highest) = realistic_board_with(|conn| {
        conn.execute_batch(
            "UPDATE board_trains SET name = 'hotfix' WHERE name = 'train-70';
             UPDATE lanes SET train = 'hotfix' WHERE train = 'train-70';",
        )
        .unwrap();
        let mut stmt = conn.prepare("SELECT name FROM board_trains").unwrap();
        let names: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        names.iter().filter_map(|n| train_number(n)).max().unwrap_or(0)
    });
    let landings = b.store.landings(b.main).unwrap();
    let hotfix = landings.iter().find(|l| l.title == "hotfix").expect("its name is its title");
    assert_eq!(hotfix.number, highest + 1);
    assert_eq!(hotfix.tickets, vec![b.t[5].id]);
}

/// An empty board moves too, and the move is one-way: a second run fails
/// rather than doubling anything. Its registration stays at schema version 58,
/// even as later migrations arrive, and an older build is refused the file.
#[test]
fn an_empty_board_moves_and_the_move_runs_once() {
    let store = Store::open_in_memory().unwrap();
    assert!(table_exists(&store, "ticket_lines"));
    assert!(!table_exists(&store, "lanes"));
    let conn = store.conn();
    let tx = conn.unchecked_transaction().unwrap();
    assert!((LINE_MIGRATION.0)(&tx).is_err(), "the tables it made are there");
    assert_eq!(LINE_MIGRATION.1, Older::Refused);
    let registered = crate::migrate::MIGRATIONS[57];
    assert!(std::ptr::fn_addr_eq(registered.0, LINE_MIGRATION.0), "registered at version 58");
    assert_eq!(registered.1, Older::Refused);
    const { assert!(crate::compat::COMPATIBLE_DOWN_TO >= 58, "no downgrade across the line migration") };
}

/// A lane with no `lane_tasks` row is named after its card: `ov-10` exactly,
/// and `ov-11-fix` by a suffix. Each takes its card's ticket, landed SHA,
/// train and history, and a lane's title survives in the carried-over event.
#[test]
fn a_lane_named_after_its_card_carries_that_card_over() {
    let (b, ()) = realistic_board_with(|conn| {
        let ws: Vec<u8> = conn.query_row("SELECT id FROM workspaces", [], |r| r.get(0)).unwrap();
        let named = Uuid::now_v7();
        let suffixed = Uuid::now_v7();
        let train: Vec<u8> = conn.query_row("SELECT id FROM board_trains WHERE name = 'train-71'", [], |r| r.get(0)).unwrap();
        for (id, name, state, sha, title) in [
            (named, "ov-10", "landed", Some("beef123"), "The owner's words"),
            (suffixed, "ov-11-fix", "building", None, ""),
        ] {
            conn.execute(
                "INSERT INTO lanes (id, workspace_id, name, state, branch, landed_sha, state_since, created_at, resource_version, title)
                 VALUES (?1, ?2, ?3, ?4, ?3, ?5, 1793505471290, 1793505471290, 1, ?6)",
                params![id.as_bytes().to_vec(), ws, name, state, sha, title],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO train_events (id, train_id, at, actor, kind, lane_id, extra)
                 VALUES (?1, ?2, 1793505471291, 'manager', 'lane_on', ?3, '{}')",
                params![Uuid::now_v7().as_bytes().to_vec(), train, id.as_bytes().to_vec()],
            )
            .unwrap();
        }
        // A lane that names no card is still dropped.
        conn.execute(
            "INSERT INTO lanes (id, workspace_id, name, state, state_since, created_at, resource_version)
             VALUES (?1, ?2, 'red-69', 'dropped', 1, 1, 1)",
            params![Uuid::now_v7().as_bytes().to_vec(), ws],
        )
        .unwrap();
    });
    let line = |i: usize| b.store.ticket_line(b.t[i].id).unwrap();
    assert_eq!(b.store.ticket_lines(b.main).unwrap().len(), 10, "red-69 adds none");
    assert_eq!((line(9).stage, line(9).landed_sha.as_deref()), (Stage::Landed, Some("beef123")));
    assert_eq!(line(10).stage, Stage::Building);
    assert_eq!(line(10).branch, "ov-11-fix");

    let landing = b.store.landings(b.main).unwrap().into_iter().find(|l| l.number == 71).unwrap();
    for i in [9, 10] {
        assert!(landing.tickets.contains(&b.t[i].id), "ticket {i} is on train-71");
        let events = b.store.line_events(LineSubject::Ticket(b.t[i].id), 0).unwrap();
        assert!(events.iter().any(|e| e.kind == "moved"), "ticket {i} has its history");
        assert!(events.iter().any(|e| e.kind == "ticket_on"), "ticket {i} has its train event");
    }
    let moved = b.store.line_events(LineSubject::Ticket(b.t[9].id), 0).unwrap();
    assert!(
        moved.iter().any(|e| e.kind == "moved" && e.body == "Carried over from lane ov-10: The owner's words"),
        "the lane's title survives"
    );
}

/// A builder pane no worker record titled takes its lane's title, else name.
#[test]
fn a_builder_pane_takes_its_lanes_name_as_its_title() {
    let b = realistic_board();
    let agents = b.store.agents(b.main, true).unwrap();
    let site = agents
        .iter()
        .find(|a| matches!(&a.place, AgentPlace::Pane { terminal } if format!("pane:{terminal}") == "pane:01a19541-ab29-78d3-ac4a-a02745560c8f"))
        .expect("the site lane's pane");
    assert_eq!(site.title, "site-4");
}

use super::*;

use crate::agents::{AgentPlace, AgentRole, NewAgent};
use crate::landings::{LandingUpdate, NewLanding, Route};
use crate::models::NoteKind;
use crate::models::Task;
use crate::models::TaskStatus::{Done, InProgress, InReview};
use crate::plan_read::StatusCounts;
use crate::tasks::TaskScope;
use crate::ticket_lines::{LineUpdate, NewTicketLine, Stage};
use crate::ticket_work::{LinkedBy as WorkLinkedBy, WorkRole};
use crate::workers::{LinkedBy, WorkerRecord};

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn refused(r: Result<impl std::fmt::Debug>) -> &'static str {
    match r {
        Err(DomainError::InvalidArgument { what }) => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn theme_events(store: &Store, theme: Uuid) -> Vec<crate::ticket_lines::LineEvent> {
    store.line_events(LineSubject::Theme(theme), 0).unwrap()
}

/// A card is in one theme: adding it to a second moves it, and both
/// themes' timelines say so.
#[test]
fn a_card_is_in_one_theme() {
    let (store, main, t) = board(2);
    let one = store
        .create_theme(main, &NewTheme { name: "Visual language".into(), outcome: "One app.".into() }, &[t[0].id, t[1].id], Actor::Manager)
        .unwrap();
    let two = store.create_theme(main, &NewTheme { name: "Reliability".into(), outcome: String::new() }, &[], Actor::Manager).unwrap();
    store.theme_cards(two.id, &[t[1].id], &[], Actor::Manager).unwrap();
    let plan = store.plan(main, 0).unwrap();
    let tasks = |name: &str| plan.themes.iter().find(|v| v.theme.name == name).unwrap().tasks.clone();
    assert_eq!(tasks("Visual language"), [t[0].id]);
    assert_eq!(tasks("Reliability"), [t[1].id]);
    assert!(theme_events(&store, one.id).iter().any(|e| e.body.contains("moved to Reliability")));
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
    let kept: Vec<_> = theme_events(&store, theme.id).into_iter().filter(|e| e.kind == "story").collect();
    assert_eq!(kept.iter().map(|e| e.body.as_str()).collect::<Vec<_>>(), ["", "First."], "each event holds the story it replaced");
    assert_eq!(kept[1].extra["to"], "Second.");
    // Touching another field leaves the story, and writes no story event.
    store.update_theme(theme.id, &ThemeUpdate { next: Some("Ship.".into()), ..Default::default() }, Actor::Manager).unwrap();
    assert_eq!(theme_events(&store, theme.id).iter().filter(|e| e.kind == "story").count(), 2);
}

/// A theme is on the card's own board, as a ticket's agent is.
#[test]
fn a_card_from_another_board_is_refused() {
    let (store, main, t) = board(1);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theme = store.create_theme(other.id, &NewTheme { name: "T".into(), outcome: String::new() }, &[], Actor::Manager).unwrap();
    assert_eq!(refused(store.theme_cards(theme.id, &[t[0].id], &[], Actor::Manager)), "other_board");
    let agent = store
        .record_agent(
            other.id,
            &NewAgent {
                role: AgentRole::Builder,
                place: AgentPlace::Subagent { id: "a1".into(), parent: None },
                harness: "claude".into(),
                session_id: None,
                session_cwd: None,
                model: None,
                title: None,
            },
        )
        .unwrap();
    assert_eq!(refused(store.start_work(agent.id, t[0].id, WorkRole::Build, WorkLinkedBy::Orchestrator, Actor::Manager)), "other_board");
    assert!(matches!(store.theme_cards(theme.id, &[Uuid::now_v7()], &[], Actor::Manager), Err(DomainError::NotFound)));
}

/// `task move` needs no hook: a moved card drops out of every read at once,
/// and the next write on the board removes its theme row.
#[test]
fn a_moved_card_leaves_its_theme_and_the_line() {
    let (store, main, t) = board(2);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    let theme = store
        .create_theme(main, &NewTheme { name: "T".into(), outcome: String::new() }, &[t[0].id, t[1].id], Actor::Manager)
        .unwrap();
    store.put_on_line(t[0].id, &NewTicketLine::default(), Actor::Manager).unwrap();
    store.put_on_line(t[1].id, &NewTicketLine::default(), Actor::Manager).unwrap();
    store.move_tasks(&[t[0].id], other.id, Actor::Manager).unwrap();

    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.themes[0].tasks, [t[1].id], "the read already ignores it");
    assert_eq!(plan.lines.iter().map(|l| l.line.task_id).collect::<Vec<_>>(), [t[1].id]);
    assert_eq!(plan.themes[0].counts.total(), 1);
    let rows = |sql: &str| -> i64 { store.conn().query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(rows("SELECT count(*) FROM board_theme_tasks"), 2, "nothing wrote yet");
    store.theme_cards(theme.id, &[], &[], Actor::Manager).unwrap();
    assert_eq!(rows("SELECT count(*) FROM board_theme_tasks"), 1);
}

/// Progress is derived: statuses are the board's own, and a ticket that landed
/// shows as such while its card's status is untouched.
#[test]
fn progress_is_derived_from_the_board() {
    let (store, main, t) = board(3);
    store
        .create_theme(main, &NewTheme { name: "T".into(), outcome: String::new() }, &[t[0].id, t[1].id, t[2].id], Actor::Manager)
        .unwrap();
    store.set_task_status(t[0].id, Done, Actor::Manager).unwrap();
    store.set_task_status(t[1].id, InProgress, Actor::Manager).unwrap();
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.themes[0].counts, StatusCounts { done: 1, in_progress: 1, backlog: 1, ..Default::default() });
    store.put_on_line(t[1].id, &NewTicketLine::default(), Actor::Manager).unwrap();
    for stage in [Stage::Building, Stage::Reviewing, Stage::Landed] {
        let to = LineUpdate { stage: Some(stage), landed_sha: (stage == Stage::Landed).then(|| "abc1234".into()), ..Default::default() };
        store.update_line(t[1].id, &to, Actor::Manager).unwrap();
    }
    let plan = store.plan(main, 0).unwrap();
    assert_eq!(plan.lines[0].line.stage, Stage::Landed, "LANDED-NOT-CLOSED: landed, card still in progress");
    assert_eq!(store.get_task(t[1].id).unwrap().status, InProgress);
}

/// The migrations that make the layer are `Welcome`, and the one that moved it
/// to the line is `Refused`: an older build reads tables it drops.
#[test]
fn the_layer_s_migrations_are_marked() {
    use crate::compat::Older;
    let marked = |i: usize| crate::migrate::MIGRATIONS[i].1;
    assert_eq!(marked(22), Older::Welcome, "0023, the plan layer");
    assert_eq!(crate::migrate::MIGRATIONS[57].1, Older::Refused, "0067, the line");
}

/// Nothing existing carries a column for the layer: every table old code
/// writes has the columns it had before.
#[test]
fn no_existing_table_gained_a_column() {
    let store = Store::open_in_memory().unwrap();
    let conn = store.conn();
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'").unwrap();
    let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    let ours = [
        "board_themes", "board_theme_tasks", "board_rulings", "board_ruling_tasks", "board_ci", "plan_budgets", "agents",
        "ticket_lines", "ticket_work", "landings", "landing_tickets", "line_events",
    ];
    for table in tables.iter().filter(|t| !ours.contains(&t.as_str())) {
        let mut info = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")).unwrap();
        let cols: Vec<String> = info.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert!(
            !cols.iter().any(|c| {
                c.contains("lane") || c.contains("theme") || c.contains("plan_") || c.contains("ruling") || c.contains("landing")
                    // A task's wait has its own `wait_line` (ov-212), which is not the layer's.
                    || c.split('_').any(|w| w == "ci" || w == "ticket")
            }),
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
    store.add_note(t[0].id, NoteKind::Progress, Actor::Manager, "Dispatched on its own branch.", serde_json::json!({})).unwrap();
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
    for task in [t[1].id, t[2].id] {
        store.put_on_line(task, &NewTicketLine { branch: format!("b-{task}"), ..Default::default() }, Actor::Manager).unwrap();
        for stage in [Stage::Building, Stage::Reviewing, Stage::Landing] {
            store.update_line(task, &LineUpdate { stage: Some(stage), ..Default::default() }, Actor::Manager).unwrap();
        }
    }
    // A budget on each (ov-307): the drill drops them with the rest.
    store.set_budget(LineSubject::Theme(theme.id), Some(1_000_000), Actor::Manager).unwrap();
    store.set_budget(LineSubject::Ticket(t[1].id), Some(500_000), Actor::Manager).unwrap();
    store.put_on_line(t[3].id, &NewTicketLine::default(), Actor::Manager).unwrap();
    store.set_line_order(main, &[t[3].id], Actor::Manager).unwrap();
    let ruling = crate::rulings::NewRuling {
        decision: "The inbox is amber.".into(),
        why: "It's the one attention color.".into(),
        reversal: "One token.".into(),
        theme_id: Some(theme.id),
    };
    let r = store.add_ruling(main, &ruling, &[t[0].id, t[2].id], Actor::Manager).unwrap();
    store.set_ruling(r.id, crate::rulings::RulingState::Confirmed, Some("Fine."), Actor::Manager).unwrap();
    store.add_ruling(main, &ruling, &[t[1].id], Actor::Manager).unwrap();
    let landing = store.landing_of(t[1].id).unwrap().expect("a reviewed ticket joins the collecting landing");
    let pushed = LandingUpdate { sha: Some("1a1b3275".into()), ..Default::default() };
    store.update_landing(landing.id, &pushed, Actor::Manager).unwrap();
    let new = NewLanding { route: Route::Ask, base: "origin/main".into(), title: String::new(), lander: None, collecting: false };
    assert!(store.start_landing(main, &new, &[], Actor::Manager).is_ok());
    let ci = crate::board_ci::CiRead {
        subject: crate::board_ci::sha_subject("1a1b3275"),
        sha: "1a1b3275".into(),
        status: crate::board_ci::CiStatus::Passed,
        url: "https://github.com/o/r/actions/runs/1".into(),
        jobs: vec![],
        fetched_at: 0,
        changed_at: 0,
        asked_at: 0,
    };
    store.record_ci(main, &ci).unwrap();
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
    out += &format!("{:#?}\n", store.search_notes(repo_of(store, main), "branch", None).unwrap());
    out
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
    crate::testing::drop_plan_layer(store);
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
    let rows: i64 = store.conn().query_row("SELECT count(*) FROM line_events", [], |r| r.get(0)).unwrap();
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
             BEGIN UPDATE ticket_lines SET reason = 'touched' WHERE stage = 'waiting'; END;",
        )
        .unwrap();
    let err = drill(&store, main, &tasks).unwrap_err();
    assert!(err.starts_with("set_task_status"), "{err}");
}

/// The drill can fail on a read too: a task list that joined `ticket_lines`
/// reads differently, or not at all, once the table is gone.
#[test]
fn the_drill_goes_red_when_a_read_depends_on_the_layer() {
    let (store, main, tasks) = populated();
    let joined = |store: &Store| -> String {
        store
            .conn()
            .query_row("SELECT count(*) FROM tasks t JOIN ticket_lines l ON l.task_id = t.id", [], |r| r.get::<_, i64>(0))
            .map(|n| n.to_string())
            .unwrap_or_else(|e| e.to_string())
    };
    let err = drill_with(&store, main, &tasks, &joined).unwrap_err();
    assert_eq!(err, "a board read changed when the layer went");
}

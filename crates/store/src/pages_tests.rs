use super::*;

use crate::models::NoteKind;
use crate::models::Task;
use crate::models::TaskStatus::{InProgress, InReview};
use crate::plan::{AgentRecord, AgentRole, LaneCard, NewLane, NewTheme, ThemeUpdate};
use crate::tasks::TaskScope;
use crate::workers::{LinkedBy, WorkerRecord};
use farcooler_core::page_doc::{self, Caps};

fn doc(title: &str) -> Page {
    let json = format!(
        r#"{{"v":1,"title":"{title}","summary":"s","blocks":[
            {{"type":"heading","text":"Lanes"}},
            {{"type":"table","columns":[{{"title":"Lane"}},{{"title":"State"}}],
              "rows":[[{{"ref":{{"lane":"mac-ux"}}}},{{"ref":{{"lane":"mac-ux"}},"show":"state"}}]]}},
            {{"type":"list","items":[{{"text":"x","ref":{{"task":"ov-1"}}}}]}}]}}"#
    );
    page_doc::parse(&json, &Caps::default()).unwrap()
}

fn board(n: usize) -> (Store, Uuid, Vec<Task>) {
    let store = Store::open_in_memory().unwrap();
    let repo = store.register_repository_for_test("overnight");
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let tasks = (0..n).map(|i| store.create_task(main, &format!("task {i}"), Actor::Manager).unwrap()).collect();
    (store, main, tasks)
}

fn put(store: &Store, ws: Uuid, slot: &str, page: &Page) -> SetOutcome {
    store.set_page(ws, &PageWrite { slot, page, anchor: Anchor::Keep, ordinal: None, if_revision: None }, Actor::Manager).unwrap()
}

fn put_at(store: &Store, ws: Uuid, slot: &str, page: &Page, now: i64) -> Result<SetOutcome> {
    store.set_page_at(ws, &PageWrite { slot, page, anchor: Anchor::Keep, ordinal: None, if_revision: None }, Actor::Manager, now)
}

fn said(r: Result<impl std::fmt::Debug>) -> String {
    match r {
        Err(DomainError::PageRefused { said }) => said,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn events(store: &Store) -> i64 {
    store.conn().query_row("SELECT count(*) FROM page_events", [], |r| r.get(0)).unwrap()
}

// ---- the store ----

#[test]
fn a_page_round_trips_and_lists_in_order() {
    let (store, ws, _) = board(1);
    let a = put(&store, ws, "train", &doc("Train"));
    put(&store, ws, "spend", &doc("Spend"));
    let SetOutcome::Written(a) = a else { panic!("a new page is written") };
    assert_eq!((a.slot.as_str(), a.title.as_str(), a.summary.as_str(), a.revision, a.ordinal), ("train", "Train", "s", 1, 0));
    assert_eq!(a.doc_json, doc("Train").to_json());
    assert_eq!(a.bytes as usize, a.doc_json.len());
    assert_eq!(a.actor, "manager");
    assert_eq!(a.schema_v, 1);
    let slots: Vec<String> = store.list_pages(ws).unwrap().into_iter().map(|p| p.slot).collect();
    assert_eq!(slots, ["train", "spend"], "new pages go to the end");
    assert_eq!(store.get_page(ws, "spend").unwrap().title, "Spend");
    assert!(matches!(store.get_page(ws, "nope"), Err(DomainError::NotFound)));
    assert!(matches!(store.list_pages(Uuid::now_v7()), Err(DomainError::NotFound)));
}

#[test]
fn publishing_identical_bytes_is_no_change_and_logs_nothing() {
    let (store, ws, _) = board(1);
    let first = put(&store, ws, "train", &doc("Train"));
    let SetOutcome::Written(first) = first else { panic!() };
    assert_eq!(events(&store), 1);
    let again = put(&store, ws, "train", &doc("Train"));
    assert!(matches!(again, SetOutcome::Unchanged(_)));
    assert_eq!(again.page(), &first, "not even the version moved");
    assert_eq!(events(&store), 1, "and no event");
}

#[test]
fn a_change_replaces_the_page_whole_and_moves_the_revision() {
    let (store, ws, _) = board(1);
    put(&store, ws, "train", &doc("Train"));
    let SetOutcome::Written(second) = put(&store, ws, "train", &doc("Train, later")) else { panic!() };
    assert_eq!((second.revision, second.title.as_str()), (2, "Train, later"));
    assert_eq!(second.resource_version, 2);
    assert_eq!(store.list_pages(ws).unwrap().len(), 1);
    assert_eq!(events(&store), 2);
}

#[test]
fn an_anchor_or_a_place_that_changes_is_a_change_and_keep_leaves_it() {
    let (store, ws, _) = board(1);
    let a = put(&store, ws, "train", &doc("Train"));
    let page = doc("Train");
    let write = |anchor: Anchor, ordinal: Option<u32>| {
        store.set_page(ws, &PageWrite { slot: "train", page: &page, anchor, ordinal, if_revision: None }, Actor::Manager).unwrap()
    };
    let SetOutcome::Written(anchored) = write(Anchor::Theme("t-1".into()), None) else { panic!("anchoring is a change") };
    assert_eq!((anchored.anchor_kind.as_str(), anchored.anchor.as_str(), anchored.revision), ("theme", "t-1", 2));
    assert!(matches!(write(Anchor::Keep, None), SetOutcome::Unchanged(_)), "keep leaves the anchor");
    assert_eq!(store.get_page(ws, "train").unwrap().anchor, "t-1");
    let SetOutcome::Written(moved) = write(Anchor::Keep, Some(7)) else { panic!("a place is a change") };
    assert_eq!((moved.ordinal, moved.anchor.as_str()), (7, "t-1"));
    let SetOutcome::Written(cleared) = write(Anchor::Clear, None) else { panic!() };
    assert_eq!((cleared.anchor_kind.as_str(), cleared.anchor.as_str()), ("", ""));
    assert_eq!(a.page().ordinal, 0);
}

#[test]
fn if_revision_refuses_a_write_against_a_page_that_changed() {
    let (store, ws, _) = board(1);
    let page = doc("Train");
    let with = |rev| store.set_page(ws, &PageWrite { slot: "train", page: &page, anchor: Anchor::Keep, ordinal: None, if_revision: Some(rev) }, Actor::Manager);
    assert!(matches!(with(1), Err(DomainError::ResourceConflict)), "a slot with no page is revision 0");
    assert!(with(0).is_ok());
    let newer = doc("Train, later");
    let stale = store.set_page(ws, &PageWrite { slot: "train", page: &newer, anchor: Anchor::Keep, ordinal: None, if_revision: Some(0) }, Actor::Manager);
    assert!(matches!(stale, Err(DomainError::ResourceConflict)));
    let fresh = store.set_page(ws, &PageWrite { slot: "train", page: &newer, anchor: Anchor::Keep, ordinal: None, if_revision: Some(1) }, Actor::Manager);
    assert!(fresh.is_ok());
}

#[test]
fn a_slot_must_be_a_slot() {
    let (store, ws, _) = board(1);
    let said = said(put_at(&store, ws, "Not A Slot", &doc("x"), 0));
    assert!(said.contains("lowercase letters, digits and hyphens"), "{said}");
}

#[test]
fn a_workspace_holds_twelve_pages() {
    let (store, ws, _) = board(1);
    for i in 0..page_doc::MAX_PAGES {
        put(&store, ws, &format!("p{i}"), &doc("P"));
    }
    let said = said(put_at(&store, ws, "one-more", &doc("P"), 0));
    assert_eq!(said, "A workspace has at most 12 pages. Remove one with page rm, then publish this one.");
    // A page already there can still change.
    assert!(put_at(&store, ws, "p0", &doc("P, changed"), 0).is_ok());
    store.remove_page(ws, "p3", Actor::Manager).unwrap();
    assert!(put_at(&store, ws, "one-more", &doc("P"), 0).is_ok(), "removing one makes room");
}

#[test]
fn a_theme_draws_three_pages() {
    let (store, ws, _) = board(1);
    let page = doc("P");
    let anchored = |slot: &str, theme: &str| {
        store.set_page(ws, &PageWrite { slot, page: &page, anchor: Anchor::Theme(theme.into()), ordinal: None, if_revision: None }, Actor::Manager)
    };
    for i in 0..3 {
        anchored(&format!("a{i}"), "theme-1").unwrap();
    }
    assert!(said(anchored("a3", "theme-1")).starts_with("A theme draws at most 3 pages."));
    anchored("a3", "theme-2").unwrap();
    assert!(anchored("a0", "theme-1").is_ok(), "a page already anchored there isn't counted against itself");
}

#[test]
fn more_than_thirty_changing_writes_an_hour_to_one_slot_are_refused() {
    let (store, ws, _) = board(1);
    let hour = 3_600_000;
    let t0 = 10 * hour;
    for i in 0..30 {
        put_at(&store, ws, "train", &doc(&format!("v{i}")), t0 + i).unwrap();
    }
    let said = said(put_at(&store, ws, "train", &doc("v30"), t0 + 30));
    assert_eq!(said, "This page changed 30 times in the last hour. Pages are for checkpoints, not a live log.");
    assert!(put_at(&store, ws, "spend", &doc("another slot"), t0 + 30).is_ok(), "the cap is per slot");
    assert!(put_at(&store, ws, "train", &doc("v29"), t0 + 30).is_ok(), "identical bytes are free, so a looping agent that stops changing things is fine");
    assert!(put_at(&store, ws, "train", &doc("v30"), t0 + hour + 1).is_ok(), "an hour later it's allowed again");
}

#[test]
fn removing_logs_and_a_second_removal_is_not_found() {
    let (store, ws, _) = board(1);
    put(&store, ws, "train", &doc("Train"));
    let gone = store.remove_page(ws, "train", Actor::Manager).unwrap();
    assert_eq!(gone.slot, "train");
    assert!(store.list_pages(ws).unwrap().is_empty());
    assert!(matches!(store.remove_page(ws, "train", Actor::Manager), Err(DomainError::NotFound)));
    let kinds: Vec<String> = store
        .conn()
        .prepare("SELECT kind FROM page_events ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(kinds, ["set", "remove"]);
    // The slot is free again and starts over.
    let SetOutcome::Written(again) = put(&store, ws, "train", &doc("Train")) else { panic!() };
    assert_eq!(again.revision, 1);
}

#[test]
fn the_events_keep_shapes_and_no_content() {
    let (store, ws, _) = board(1);
    put(&store, ws, "train", &doc("A Secret Title"));
    let shape: String = store.conn().query_row("SELECT shape FROM page_events", [], |r| r.get(0)).unwrap();
    assert_eq!(shape, "heading:1 table:1 list:1 ref-task:1 ref-lane:2");
    let columns: Vec<String> = store
        .conn()
        .prepare("SELECT name FROM pragma_table_info('page_events')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!columns.iter().any(|c| c.contains("doc") || c.contains("title") || c.contains("text")), "{columns:?}");
}

#[test]
fn stats_say_how_often_and_with_what_shape() {
    let (store, ws, _) = board(1);
    let hour = 3_600_000;
    put_at(&store, ws, "train", &doc("v1"), hour).unwrap();
    put_at(&store, ws, "train", &doc("v2"), 2 * hour).unwrap();
    put_at(&store, ws, "spend", &doc("v1"), 3 * hour).unwrap();
    let stats = store.page_stats(ws, 0).unwrap();
    assert_eq!(stats.iter().map(|s| (s.slot.as_str(), s.sets)).collect::<Vec<_>>(), [("train", 2), ("spend", 1)]);
    let train = &stats[0];
    assert_eq!((train.first_at, train.last_at), (hour, 2 * hour));
    assert_eq!(train.shape.get("table"), Some(&2));
    assert_eq!(train.shape.get("ref-lane"), Some(&4));
    let later = store.page_stats(ws, 3 * hour).unwrap();
    assert_eq!(later.len(), 1, "since drops the older events");
    assert_eq!(later[0].slot, "spend");
}

#[test]
fn deleting_a_workspace_takes_its_pages_and_events() {
    let (store, main, _) = board(1);
    let repo = store.get_workspace(main).unwrap().repository_id;
    let other = store.create_workspace(repo, "Other", "ot").unwrap();
    put(&store, other.id, "train", &doc("Train"));
    put(&store, main, "train", &doc("Train"));
    store.delete_workspace(other.id).unwrap();
    let rows: i64 = store.conn().query_row("SELECT count(*) FROM board_pages", [], |r| r.get(0)).unwrap();
    assert_eq!((rows, events(&store)), (1, 1), "only Main's page is left");
    assert_eq!(store.list_pages(main).unwrap().len(), 1);
}

#[test]
fn the_cards_on_a_board_are_known_by_key_in_lower_case() {
    let (store, main, tasks) = board(2);
    let keys = store.page_card_keys(main).unwrap();
    assert!(keys.contains(&tasks[0].key.to_ascii_lowercase()) && keys.contains(&tasks[1].key.to_ascii_lowercase()));
    assert_eq!(keys.len(), 2);
}

// ---- the migration ----

/// The migration is the 25th and `Welcome`, so the build before it can still
/// open the file.
#[test]
fn the_migration_is_welcome() {
    use crate::compat::Older;
    let last = &crate::migrate::MIGRATIONS[24];
    assert!(std::ptr::fn_addr_eq(last.0, migration_0025_pages as fn(&Transaction) -> rusqlite::Result<()>));
    assert_eq!(last.1, Older::Welcome);
    assert_eq!(crate::migrate::CURRENT_SCHEMA_VERSION, 25);
}

/// Nothing existing carries a column for pages, nothing points into the pages'
/// tables, and no trigger touches them.
#[test]
fn nothing_existing_gained_a_column_a_key_or_a_trigger_for_pages() {
    let store = Store::open_in_memory().unwrap();
    let conn = store.conn();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let ours = ["board_pages", "page_events"];
    for table in tables.iter().filter(|t| !ours.contains(&t.as_str())) {
        let cols: Vec<String> = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!cols.iter().any(|c| c.contains("page")), "{table} has a column for pages: {cols:?}");
        let keys: Vec<String> = conn
            .prepare(&format!("SELECT \"table\" FROM pragma_foreign_key_list('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!keys.iter().any(|t| ours.contains(&t.as_str())), "{table} points at a page table");
    }
    // The pages' own tables point at the workspace and at nothing else: above
    // all not at the plan layer.
    for table in ours {
        let keys: Vec<String> = conn
            .prepare(&format!("SELECT \"table\" FROM pragma_foreign_key_list('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(keys, ["workspaces"], "{table}");
    }
    let triggers: Vec<String> = conn
        .prepare("SELECT sql FROM sqlite_master WHERE type = 'trigger'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for sql in triggers {
        assert!(!ours.iter().any(|t| sql.contains(t)), "a trigger touches the pages: {sql}");
    }
}

// ---- the removal drill (ov-269, section 4.3) ----

/// A board with everything on it: notes, a worker, a line, the plan layer,
/// and then pages on top, one anchored to a theme.
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
    let agent = AgentRecord { harness: "claude".into(), agent_id: "a1".into(), role: AgentRole::Build, model: None, ended: false };
    let lane = store
        .create_lane(
            main,
            &NewLane { name: "mac-ux".into(), ..Default::default() },
            &[LaneCard { task_id: t[1].id, slice: String::new() }],
            Some(&agent),
            Actor::Manager,
        )
        .unwrap();
    assert_eq!(lane.name, "mac-ux");
    let queued = store
        .create_lane(main, &NewLane { name: "next-one".into(), ..Default::default() }, &[LaneCard { task_id: t[3].id, slice: String::new() }], None, Actor::Manager)
        .unwrap();
    store.set_plan(main, &[queued.id], Actor::Manager).unwrap();
    put(&store, main, "train", &doc("Train"));
    store
        .set_page(
            main,
            &PageWrite { slot: "risks", page: &doc("Risks"), anchor: Anchor::Theme(theme.id.to_string()), ordinal: None, if_revision: None },
            Actor::Manager,
        )
        .unwrap();
    (store, main, t)
}

/// Everything the board reads that a task could depend on pages for, and the
/// plan layer's whole read beside it.
fn reads(store: &Store, main: Uuid, tasks: &[Task]) -> String {
    let mut out = String::new();
    out += &format!("{:#?}\n", store.list_tasks(TaskScope::Workspace(main), None).unwrap());
    for t in tasks {
        let task = store.get_task(t.id).unwrap();
        out += &format!("{task:#?}\n{:#?}\n", store.notes_for(t.id, None).unwrap());
        out += &format!("{:#?}\n", store.task_facts(std::slice::from_ref(&task)).unwrap());
        out += &format!("{:#?}\n", store.workers_for(t.id).unwrap());
    }
    out += &format!("{:#?}\n", store.search_notes(store.get_workspace(main).unwrap().repository_id, "lane", None).unwrap());
    let mut plan = store.plan(main, i64::MIN).unwrap();
    plan.now_ms = 0;
    out += &format!("{plan:#?}\n");
    out
}

/// The drill: with the pages' tables gone, every board and plan read is the
/// same bytes and every board and plan write still works. `Err` names what
/// broke.
fn drill(store: &Store, main: Uuid, tasks: &[Task]) -> std::result::Result<(), String> {
    drill_with(store, main, tasks, &|_| String::new())
}

/// The drill, with one more read to compare: how a test stands in for a read
/// that came to depend on pages.
fn drill_with(store: &Store, main: Uuid, tasks: &[Task], extra: &dyn Fn(&Store) -> String) -> std::result::Result<(), String> {
    let before = reads(store, main, tasks) + &extra(store);
    crate::testing::drop_pages(store);
    let after = reads(store, main, tasks) + &extra(store);
    if before != after {
        return Err("a read changed when the pages went".into());
    }
    let new = store.create_task(main, "after", Actor::Manager).map_err(|e| format!("create_task: {e}"))?;
    store.set_task_status(tasks[0].id, InProgress, Actor::Manager).map_err(|e| format!("set_task_status: {e}"))?;
    store
        .add_note(tasks[0].id, NoteKind::Progress, Actor::Manager, "still writing", serde_json::json!({}))
        .map_err(|e| format!("add_note: {e}"))?;
    store
        .create_theme(main, &NewTheme { name: "After".into(), outcome: "Still works.".into() }, &[new.id], Actor::Manager)
        .map_err(|e| format!("create_theme: {e}"))?;
    let repo = store.get_workspace(main).map_err(|e| e.to_string())?.repository_id;
    let other = store.create_workspace(repo, "Other", "ot").map_err(|e| format!("create_workspace: {e}"))?;
    store.move_tasks(&[new.id], other.id, Actor::Manager).map_err(|e| format!("move_tasks: {e}"))?;
    store.move_tasks(&[new.id], main, Actor::Manager).map_err(|e| format!("move_tasks back: {e}"))?;
    store.delete_workspace(other.id).map_err(|e| format!("delete_workspace: {e}"))?;
    Ok(())
}

/// Removing the pages changes nothing the board or the plan says, and every
/// write still works.
#[test]
fn the_board_and_the_plan_are_the_same_bytes_with_the_pages_dropped() {
    let (store, main, tasks) = populated();
    assert_eq!(store.list_pages(main).unwrap().len(), 2, "the pages had something to remove");
    assert!(events(&store) >= 2);
    drill(&store, main, &tasks).unwrap();
}

/// The drill can fail: a trigger on `tasks` that writes into the pages is a
/// task depending on them, and the writes break once the tables are gone.
#[test]
fn the_drill_goes_red_when_a_task_write_depends_on_the_pages() {
    let (store, main, tasks) = populated();
    store
        .conn()
        .execute_batch(
            "CREATE TRIGGER injected AFTER UPDATE OF status ON tasks
             BEGIN UPDATE board_pages SET summary = 'touched'; END;",
        )
        .unwrap();
    let err = drill(&store, main, &tasks).unwrap_err();
    assert!(err.starts_with("set_task_status"), "{err}");
}

/// The drill can fail on a read too: a task list that joined `board_pages`
/// reads differently, or not at all, once the table is gone.
#[test]
fn the_drill_goes_red_when_a_read_depends_on_the_pages() {
    let (store, main, tasks) = populated();
    let joined = |store: &Store| -> String {
        store
            .conn()
            .query_row("SELECT count(*) FROM tasks t JOIN board_pages p ON p.workspace_id = t.workspace_id", [], |r| r.get::<_, i64>(0))
            .map(|n| n.to_string())
            .unwrap_or_else(|e| e.to_string())
    };
    let err = drill_with(&store, main, &tasks, &joined).unwrap_err();
    assert_eq!(err, "a read changed when the pages went");
}

/// And a plan read that came to name the pages fails the same drill.
#[test]
fn the_drill_goes_red_when_a_plan_read_depends_on_the_pages() {
    let (store, main, tasks) = populated();
    let themes_with_pages = |store: &Store| -> String {
        store
            .conn()
            .query_row("SELECT count(*) FROM board_themes t JOIN board_pages p ON p.anchor = lower(hex(t.id))", [], |r| r.get::<_, i64>(0))
            .map(|n| n.to_string())
            .unwrap_or_else(|e| e.to_string())
    };
    let err = drill_with(&store, main, &tasks, &themes_with_pages).unwrap_err();
    assert_eq!(err, "a read changed when the pages went");
}

/// The other direction, which is what the text anchor is for: with the plan
/// layer gone, the pages are still there, still read, and still written,
/// anchor and all.
#[test]
fn the_pages_survive_the_plan_layer_being_dropped() {
    let (store, main, _) = populated();
    let before = store.list_pages(main).unwrap();
    crate::testing::drop_plan_layer(&store);
    assert_eq!(store.list_pages(main).unwrap(), before, "pages read the same bytes");
    let anchored = store.get_page(main, "risks").unwrap();
    assert_eq!(anchored.anchor_kind, "theme");
    assert!(!anchored.anchor.is_empty(), "the anchor is kept as text, pointing at nothing now");
    put(&store, main, "risks", &doc("Risks, later"));
    put(&store, main, "new-slot", &doc("New"));
    store.remove_page(main, "train", Actor::Manager).unwrap();
    assert_eq!(store.list_pages(main).unwrap().len(), 2);
}

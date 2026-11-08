//! ov-965: the engine owns only a ticket's stage. A ticket it holds with no run (its column) keeps every other
//! record open to the orchestrator: its line, plan rank and hold, its wait, its blocks and its subagents. A card
//! with a run, and a closed ticket, keep every fence. Each case runs at `legacy` and at `released`, and no
//! write may lift a fence to get through: the schema cookie must not move.
use super::*;
use crate::models::{Actor, TaskStatus};
use crate::ticket_lines::{LineUpdate, NewTicketLine, Stage};
use crate::waits::Wait;
use crate::workers::{EndReason, LinkedBy, WorkerRecord};
use farcooler_workflow::{Command, Definition, Grant, Input};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase { Legacy, Released }
const PHASES: [Phase; 2] = [Phase::Legacy, Phase::Released];

struct Board { _tmp: tempfile::TempDir, store: Store, ws: uuid::Uuid, other: uuid::Uuid, run: uuid::Uuid, ticket: crate::models::Task, sequence: std::cell::Cell<u64> }

impl Board {
    fn apply(&self, id: &str, command: Command) -> crate::workflow::Receipt {
        let n = self.sequence.get() + 1;
        self.sequence.set(n);
        let revision = self.store.workflow_state(&self.ws.to_string()).unwrap().map_or(0, |s| s.revision);
        self.store.workflow_apply(&Input { schema: 1, id: id.into(), workspace: self.ws.to_string(), expected_revision: revision, epoch: 1,
            source: "controller".into(), source_sequence: n, now_ms: 0, grant: Grant::Controller, busy_resources: Default::default(), command }).unwrap()
    }
    fn cookie(&self) -> i64 { self.store.conn().query_row("PRAGMA schema_version", [], |r| r.get(0)).unwrap() }
    fn resting(&self) -> Vec<uuid::Uuid> {
        let conn = self.store.conn();
        let mut q = conn.prepare("SELECT task_id FROM engine_stage_tickets ORDER BY rowid").unwrap();
        q.query_map([], |r| r.get::<_, Vec<u8>>(0)).unwrap().map(|b| uuid::Uuid::from_slice(&b.unwrap()).unwrap()).collect()
    }
}

fn board(phase: Phase) -> Board {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    let store = Store::open(&path).unwrap();
    let repo = store.register_repository_for_test("Stage");
    let ws = store.ensure_main_workspace(repo).unwrap().id;
    let other = store.create_task(ws, "other", Actor::User).unwrap().id;
    let run = store.create_task(ws, "run", Actor::User).unwrap();
    let ticket = store.create_task(ws, "ticket", Actor::User).unwrap();
    let store = if phase == Phase::Released {
        drop(store);
        let store = Store::open(&path).unwrap();
        let plan = preflight(&store.conn()).unwrap();
        store.engine_quiesce(joined(&store)).unwrap();
        store.engine_commit_for_test(&plan, &proof(&store), "stage").unwrap();
        store.engine_release(&plan, joined(&store)).unwrap();
        drop(store);
        Store::open(&path).unwrap()
    } else { store };
    let b = Board { _tmp: tmp, store, ws, other, run: run.id, ticket, sequence: Default::default() };
    let tracker = Definition::tracker("owner");
    assert!(b.apply("run", Command::Enqueue { item: run.key.clone(), policy_pin: tracker.pin(), resource: "own-run".into(),
        dependencies: Default::default(), initial_candidate: Some(tracker.pin()), definition: tracker }).result.is_ok(), "the engine runs one card");
    let definition = Definition::ticket();
    let policy_pin = b.store.workflow_ticket_bundle(&definition).unwrap();
    assert!(b.apply("ticket", Command::Enqueue { item: b.ticket.key.clone(), policy_pin, resource: "own-ticket".into(),
        dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition }).result.is_ok(), "and tracks a ticket");
    let authority: String = b.store.conn().query_row("SELECT phase FROM engine_cutover_authority", [], |r| r.get(0)).unwrap();
    assert_eq!(authority, if phase == Phase::Legacy { "legacy" } else { "released" });
    assert!(b.store.task_is_tracked_ticket(b.ticket.id).unwrap());
    b
}

fn line() -> NewTicketLine { NewTicketLine { reason: "plan".into(), branch: "b".into(), worktree_id: None, worktree_path: "/p".into(), stack_parent: None } }
fn worker(id: &str) -> WorkerRecord {
    WorkerRecord { harness: "claude".into(), agent_id: id.into(), session_id: None, session_cwd: None,
        orchestrator_terminal: None, label: Some("build".into()), model: None, linked_by: LinkedBy::Orchestrator }
}

/// Lines, plan rank and a line's hold: what still stopped plan order and PR chips on a tracked card.
#[test]
fn a_ticket_at_rest_takes_its_line_rank_and_hold() {
    for phase in PHASES {
        let b = board(phase);
        let (s, t, user) = (&b.store, b.ticket.id, Actor::User);
        let cookie = b.cookie();
        s.put_on_line(t, &line(), user).unwrap_or_else(|e| panic!("{phase:?}: a ticket goes on the line: {e:?}"));
        s.set_line_order(b.ws, &[t], user).unwrap_or_else(|e| panic!("{phase:?}: and takes a plan rank: {e:?}"));
        assert_eq!(s.ticket_line(t).unwrap().plan_rank, Some(1));
        s.update_line(t, &LineUpdate { branch: Some("c".into()), pr_number: Some(Some(7)), ..Default::default() }, user)
            .unwrap_or_else(|e| panic!("{phase:?}: its line is set: {e:?}"));
        for stage in [Stage::Building, Stage::Reviewing] {
            s.update_line(t, &LineUpdate { stage: Some(stage), ..Default::default() }, user).unwrap_or_else(|e| panic!("{phase:?}: PR stage {stage:?}: {e:?}"));
        }
        s.ensure_on_line(t, user).unwrap_or_else(|e| panic!("{phase:?}: dispatch finds it on the line: {e:?}"));
        assert_eq!(b.cookie(), cookie, "{phase:?}: no fence was lifted to get through");
        assert_eq!(s.get_task(t).unwrap().status, TaskStatus::Backlog, "{phase:?}: a line moves no column");
    }
}

/// A line's hold (`held_off`) is the landing queue's, which is fenced for everyone after release; at legacy a
/// ticket's line takes it like any other.
#[test]
fn a_ticket_at_rest_takes_a_line_hold_at_legacy() {
    let b = board(Phase::Legacy);
    let (s, t) = (&b.store, b.ticket.id);
    s.put_on_line(t, &line(), Actor::User).unwrap();
    s.update_line(t, &LineUpdate { held_off: Some(true), ..Default::default() }, Actor::User).expect("a ticket's line is held");
    assert!(s.ticket_line(t).unwrap().held_off);
}

/// Waits, holds, blocks and subagents already got through by dropping the fences for one write (ov-962). The
/// guards now say the rule, so none of these touches the schema.
#[test]
fn a_ticket_at_rest_takes_waits_blocks_and_subagents_with_no_fence_lifted() {
    for phase in PHASES {
        let b = board(phase);
        let (s, t, user) = (&b.store, b.ticket.id, Actor::User);
        let cookie = b.cookie();
        s.set_wait(t, Some(Wait::Parked), user).unwrap_or_else(|e| panic!("{phase:?}: park: {e:?}"));
        s.set_wait(t, None, user).unwrap_or_else(|e| panic!("{phase:?}: clear: {e:?}"));
        let now = crate::tasks::now_millis();
        s.set_wait(t, Some(Wait::Until(now + 60_000)), user).unwrap_or_else(|e| panic!("{phase:?}: hold: {e:?}"));
        assert_eq!(s.release_due_holds(now + 120_000).unwrap().len(), 1, "{phase:?}: the hold lets go");
        s.set_block(t, b.other, Some("because")).unwrap_or_else(|e| panic!("{phase:?}: block: {e:?}"));
        s.set_block(t, b.other, Some("still")).unwrap_or_else(|e| panic!("{phase:?}: block again: {e:?}"));
        s.unblock(t, b.other).unwrap_or_else(|e| panic!("{phase:?}: unblock: {e:?}"));
        s.record_worker(t, &worker("a1"), user).unwrap_or_else(|e| panic!("{phase:?}: subagent: {e:?}"));
        s.end_worker(t, "claude", Some("a1"), EndReason::Finished, user).unwrap_or_else(|e| panic!("{phase:?}: subagent ends: {e:?}"));
        s.record_worker(t, &worker("a1"), user).unwrap_or_else(|e| panic!("{phase:?}: subagent resumes: {e:?}"));
        let agent: Vec<u8> = s.conn().query_row("SELECT agent_id FROM ticket_work WHERE task_id=?1 AND ended_at IS NULL",
            [t.as_bytes().as_slice()], |r| r.get(0)).unwrap();
        s.end_agent(uuid::Uuid::from_slice(&agent).unwrap(), crate::ticket_work::Verdict::Finished).unwrap_or_else(|e| panic!("{phase:?}: its agent ends: {e:?}"));
        assert_eq!(b.cookie(), cookie, "{phase:?}: no fence was lifted to get through");
    }
}

/// A ticket's hold leaves its column where the engine put it, so a ticket held in Todo must still let go.
#[test]
fn a_ticket_held_in_todo_lets_go() {
    for phase in PHASES {
        let b = board(phase);
        let (s, t) = (&b.store, b.ticket.id);
        assert!(b.apply("todo", Command::Move { item: b.ticket.key.clone(), to: "todo".into() }).result.is_ok());
        assert_eq!(s.get_task(t).unwrap().status, TaskStatus::Todo);
        let now = crate::tasks::now_millis();
        s.set_wait(t, Some(Wait::Until(now + 60_000)), Actor::User).unwrap();
        assert_eq!(s.get_task(t).unwrap().status, TaskStatus::Todo, "{phase:?}: the hold moves no column");
        assert_eq!(s.next_hold_due().unwrap(), Some(now + 60_000), "{phase:?}: the sweep waits for it");
        let gone = s.release_due_holds(now + 120_000).unwrap();
        assert_eq!(gone.iter().map(|(x, _)| x.id).collect::<Vec<_>>(), vec![t], "{phase:?}: it lets go");
    }
}

/// The column is the engine's alone, on a ticket and on a run; a run keeps every other fence too.
#[test]
fn the_stage_and_a_run_stay_the_engines() {
    for phase in PHASES {
        let b = board(phase);
        let (s, user) = (&b.store, Actor::User);
        assert!(s.set_task_status(b.ticket.id, TaskStatus::InProgress, user).is_err(), "{phase:?}: a hand never moves a ticket's column");
        let run = b.run;
        assert!(s.put_on_line(run, &line(), user).is_err(), "{phase:?}: a run's line is its own");
        assert!(s.set_wait(run, Some(Wait::Parked), user).is_err(), "{phase:?}: a run takes no wait");
        assert!(s.record_worker(run, &worker("a2"), user).is_err(), "{phase:?}: nor a hand subagent");
        if phase == Phase::Released {
            assert!(s.set_block(run, b.other, None).is_err(), "{phase:?}: nor a block");
            // The guards themselves, under any code path: a raw write to a run's records aborts.
            let raw = |sql: &str| s.conn().execute(sql, [run.as_bytes().as_slice()]).is_err();
            assert!(raw("UPDATE tasks SET wait_kind='parked' WHERE id=?1"), "the wait fence holds for a run");
            assert!(raw("UPDATE tasks SET status='done' WHERE id=?1"), "the stage fence holds for a run");
            let t = b.ticket.id.as_bytes().to_vec();
            assert!(s.conn().execute("UPDATE tasks SET status='done' WHERE id=?1", [&t]).is_err(), "and for a ticket");
            assert!(s.conn().execute("UPDATE tasks SET wait_kind='parked' WHERE id=?1", [&t]).is_ok(), "a ticket's wait is open");
        }
    }
}

/// The marker the guards read follows the engine's own state: a ticket at rest is on it, a run and a closed
/// ticket are not. Absence fences, so a missed sync fails closed.
#[test]
fn the_marker_follows_the_run() {
    for phase in PHASES {
        let b = board(phase);
        assert_eq!(b.resting(), vec![b.ticket.id], "{phase:?}: only the ticket at rest");
        assert!(b.apply("doing", Command::Move { item: b.ticket.key.clone(), to: "doing".into() }).result.is_ok());
        assert_eq!(b.resting(), vec![b.ticket.id], "{phase:?}: a column move keeps it at rest");
        let tracker = Definition::tracker("owner");
        assert!(b.apply("start", Command::Start { item: b.ticket.key.clone(), policy_pin: tracker.pin(), resource: "own-ticket".into(),
            dependencies: Default::default(), initial_candidate: Some(tracker.pin()), definition: tracker, models: Default::default() }).result.is_ok());
        assert!(b.resting().is_empty(), "{phase:?}: Start takes it off");
        assert!(b.store.put_on_line(b.ticket.id, &line(), Actor::User).is_err(), "{phase:?}: and its line is the run's again");
        // A ticket that closes is fenced like a run.
        let second = b.store.create_task(b.ws, "second", Actor::User).unwrap();
        let definition = Definition::ticket();
        let policy_pin = b.store.workflow_ticket_bundle(&definition).unwrap();
        assert!(b.apply("second", Command::Enqueue { item: second.key.clone(), policy_pin, resource: "own-second".into(),
            dependencies: Default::default(), initial_candidate: Some(definition.pin()), definition }).result.is_ok());
        assert_eq!(b.resting(), vec![second.id]);
        assert!(b.apply("close", Command::Move { item: second.key.clone(), to: "done".into() }).result.is_ok());
        assert!(b.resting().is_empty(), "{phase:?}: a closed ticket leaves it");
        assert!(b.store.set_block(second.id, b.other, None).is_err() || phase == Phase::Legacy, "{phase:?}: a closed ticket is fenced");
    }
}

/// The marker is the engine's own record: a write the projection makes to a run's card, closing it with an open
/// wait it can't keep, still goes through. The guards it lifts include the new wait fence.
#[test]
fn closing_a_ticket_clears_what_it_held() {
    for phase in PHASES {
        let b = board(phase);
        let (s, t, user) = (&b.store, b.ticket.id, Actor::User);
        s.set_wait(t, Some(Wait::Parked), user).unwrap();
        s.record_worker(t, &worker("a1"), user).unwrap();
        assert!(b.apply("close", Command::Move { item: b.ticket.key.clone(), to: "done".into() }).result.is_ok(), "{phase:?}: the ticket closes");
        let task = s.get_task(t).unwrap();
        assert_eq!(task.status, TaskStatus::Done);
        assert!(task.wait.is_none(), "{phase:?}: the wait went with it");
        assert!(s.workers_for(t).unwrap().iter().all(|w| w.ended_at.is_some()), "{phase:?}: and its subagent");
    }
}

/// Registry 68's catalog is pinned, and differs from schema65's in exactly the four narrowed guards, each the
/// text `engine_stage::excludes` writes. A fresh install at 68 is a known source, not `schema:unknown`.
#[test]
fn registry68_catalog_is_pinned_and_narrows_only_the_four_guards() {
    let mut c65 = Connection::open_in_memory().unwrap();
    c65.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut c65, 65);
    let m65 = Manifest::capture(&c65).unwrap();
    let store = Store::open_in_memory().unwrap();
    let plan = preflight(&store.conn()).unwrap();
    assert_eq!(plan.registry, 68);
    assert_eq!(catalog_hash(&plan.raw), SOURCE68_SCHEMA_HASH, "the composed registry-68 catalog");
    assert!(!plan.quarantines.iter().any(|q| q.resource == "schema:unknown"), "a fresh install is a known source");
    // `meta` has two equivalent spellings, which `catalog_hash` already treats as one.
    let changed: Vec<&String> = plan.raw.objects.iter().filter(|(name, value)| *name != "meta" && m65.objects.get(*name) != Some(value)).map(|(name, _)| name).collect();
    let mut four: Vec<String> = crate::engine_stage::EXCLUDES.iter().map(|(t, s, _)| format!("workflow_excludes_{t}_{s}")).collect();
    four.sort();
    assert_eq!(changed, four.iter().collect::<Vec<_>>(), "only the four guards changed since 65");
    assert!(m65.objects.keys().eq(plan.raw.objects.keys()), "no object came or went outside the engine's namespace");
    for (table, suffix, event) in crate::engine_stage::EXCLUDES {
        let name = format!("workflow_excludes_{table}_{suffix}");
        assert_eq!(m65.objects[&name].2.as_deref(), Some(crate::engine_stage::excludes(table, suffix, event, false).as_str()), "{name} before");
        assert_eq!(plan.raw.objects[&name].2.as_deref(), Some(crate::engine_stage::excludes(table, suffix, event, true).as_str()), "{name} after");
    }
}

/// ov-963's path, with a real migration: a released install at registry 67, already tracking a ticket and running
/// a card, is admitted, migrates to 68, stays released, and its ticket (only) takes its line at once.
#[test]
fn a_released_install_at_67_carries_its_tickets_forward() {
    let Board { _tmp: tmp, store, ticket, run, ws, .. } = board(Phase::Released);
    drop(store);
    let path = tmp.path().join("db.sqlite");
    roll_back_to_67(&path);
    // Under 67's guards the ticket's line is refused: what the migration must change.
    {
        let conn = Connection::open(&path).unwrap();
        assert!(conn.execute("INSERT INTO ticket_lines (task_id, stage, stage_since, created_at, resource_version) VALUES (?1,'waiting',1,1,1)",
            [ticket.id.as_bytes().as_slice()]).is_err(), "registry 67 refuses a ticket's line");
    }
    assert_eq!(startup::ordinary_admission(&path).unwrap(), startup::Admission::Existing, "a released 67 is admitted");
    let store = Store::open(&path).expect("and migrates forward");
    let version: String = store.conn().query_row("SELECT value FROM meta WHERE key='schema_version'", [], |r| r.get(0)).unwrap();
    assert_eq!(version, "68");
    let phase: String = store.conn().query_row("SELECT phase FROM engine_cutover_authority", [], |r| r.get(0)).unwrap();
    assert_eq!(phase, "released");
    let resting: Vec<Vec<u8>> = { let c = store.conn(); let mut q = c.prepare("SELECT task_id FROM engine_stage_tickets").unwrap();
        q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
    assert_eq!(resting, vec![ticket.id.as_bytes().to_vec()], "the backfill found the ticket, and not the run");
    store.put_on_line(ticket.id, &line(), Actor::User).expect("the ticket takes its line");
    store.set_line_order(ws, &[ticket.id], Actor::User).expect("and its rank");
    store.set_wait(ticket.id, Some(Wait::Parked), Actor::User).expect("and its wait");
    assert!(store.put_on_line(run, &line(), Actor::User).is_err(), "the run keeps its fence");
    assert!(store.set_task_status(ticket.id, TaskStatus::Done, Actor::User).is_err(), "and the column stays the engine's");
}

/// Put the database at `path` back exactly as registry 67 left it: its guards, its stamps, no marker. Only the
/// objects 68 changed are touched, and the result is checked against a database migrated to 67 and no further.
fn roll_back_to_67(path: &std::path::Path) {
    let mut reference = Connection::open_in_memory().unwrap();
    reference.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    crate::migrate::migrate_only_to(&mut reference, 67);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("DROP TRIGGER engine_task_wait_fence; DROP TABLE engine_stage_tickets;").unwrap();
    let mut names: Vec<String> = crate::engine_stage::EXCLUDES.iter().map(|(t, s, _)| format!("workflow_excludes_{t}_{s}")).collect();
    names.extend(["engine_fence_ticket_lines_d", "engine_fence_ticket_work_d", "engine_fence_task_blocks_d", "engine_fence_task_blocks_i",
        "engine_fence_task_blocks_u", "engine_task_execution_fence"].map(String::from));
    for name in &names {
        let sql: String = reference.query_row("SELECT sql FROM sqlite_master WHERE name=?1", [name], |r| r.get(0)).unwrap();
        conn.execute_batch(&format!("DROP TRIGGER {name}; {sql};")).unwrap();
    }
    conn.execute("UPDATE meta SET value='67' WHERE key='schema_version'", []).unwrap();
    conn.execute("UPDATE meta SET value=?1 WHERE key='compatible_down_to'", [crate::compat::floor_at(67).to_string()]).unwrap();
    let objects = |c: &Connection| { let mut o = Manifest::capture(c).unwrap().objects; o.remove("meta"); o };
    assert_eq!(objects(&conn), objects(&reference), "exactly registry 67's catalog, but for `meta`'s spelling");
    drop(conn);
}

/// The commit's own answer, then the release's: a hand-changed guard must be refused at the commit, by the exact
/// text check, before `schema:unknown` refuses the release.
fn cut_over(path: &std::path::Path) -> Result<Result<()>> {
    // Raw, as the cutover reads a source: no ordinary open has migrated it.
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;").unwrap();
    let plan = preflight(&conn)?;
    assert_eq!(plan.registry, 67);
    // What the freeze does in the same transaction as the quiesce: the source takes every later migration.
    crate::migrate::migrate(&mut conn, 67).unwrap();
    let store = Store::from_engine_cutover_connection(conn, None).unwrap();
    store.engine_quiesce(joined(&store))?;
    store.engine_commit_for_test(&plan, &proof(&store), "sixty-seven")?;
    Ok(store.engine_release(&plan, joined(&store)))
}

/// A board still at registry 67 and `legacy` cuts over to 68 through the one named delta. A guard someone
/// changed by hand is not that delta, and the release is refused.
#[test]
fn a_legacy_67_source_cuts_over_through_the_named_delta_only() {
    for tampered in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("db.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let repo = store.register_repository_for_test("Sixty-seven");
            let ws = store.ensure_main_workspace(repo).unwrap().id;
            store.create_task(ws, "history", Actor::User).unwrap();
        }
        roll_back_to_67(&path);
        if tampered {
            let conn = Connection::open(&path).unwrap();
            let sql = crate::engine_stage::excludes("ticket_lines", "insert", "INSERT", false).replace("WHEN NEW.task_id IS NOT NULL AND", "WHEN");
            conn.execute_batch(&format!("DROP TRIGGER workflow_excludes_ticket_lines_insert; {sql};")).unwrap();
        }
        let done = cut_over(&path);
        if tampered { assert!(matches!(&done, Err(Error::Refused(why)) if why.contains("source changed")), "refused at the commit: {done:?}"); }
        else { assert!(matches!(done, Ok(Ok(()))), "{done:?}"); }
        if !tampered {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.schema().unwrap().version, 68);
            assert_eq!(store.engine_authority().unwrap().phase, "released");
        }
    }
}

/// The guards themselves, under any code path: raw writes to each record a ticket at rest keeps pass, and the same
/// writes to a run's card abort. The delete fences apply only after release.
#[test]
fn the_guards_pass_a_ticket_at_rest_and_hold_a_run() {
    for phase in PHASES {
        let b = board(phase);
        let s = &b.store;
        let other = b.other.as_bytes().to_vec();
        let agent = { s.record_worker(b.other, &worker("seed"), Actor::User).unwrap();
            s.conn().query_row("SELECT agent_id FROM ticket_work WHERE task_id=?1", [&other], |r| r.get::<_, Vec<u8>>(0)).unwrap() };
        for (card, open) in [(b.ticket.id, true), (b.run, false)] {
            let id = card.as_bytes().to_vec();
            // Each statement binds only the parameters it names: the card, the other card, the agent.
            let bind = |c: &Connection, sql: &str| -> rusqlite::Result<()> {
                let mut q = c.prepare(sql)?;
                let n = q.parameter_count();
                q.execute(rusqlite::params_from_iter([&id, &other, &agent].into_iter().take(n))).map(|_| ())
            };
            let run = |sql: &str| bind(&s.conn(), sql);
            // `workflow_excludes_*` hold at every phase; the `engine_fence_*` ones only after release.
            let fenced = |gated: bool| !open && (!gated || phase == Phase::Released);
            let steps: [(&str, &str, bool); 9] = [
                ("line insert", "INSERT INTO ticket_lines (task_id, stage, stage_since, created_at, resource_version) VALUES (?1,'waiting',1,1,1)", false),
                ("line update", "UPDATE ticket_lines SET plan_rank = 3 WHERE task_id = ?1", false),
                ("work insert", "INSERT INTO ticket_work (agent_id, task_id, role, started_at, linked_by) VALUES (?3, ?1, 'build', 1, 'orchestrator')", false),
                ("work update", "UPDATE ticket_work SET ended_at = 2 WHERE task_id = ?1", false),
                ("block insert", "INSERT INTO task_blocks (task_id, blocked_by, reason) VALUES (?1, ?2, 'x')", true),
                ("block update", "UPDATE task_blocks SET reason = 'y' WHERE task_id = ?1", true),
                ("block delete", "DELETE FROM task_blocks WHERE task_id = ?1", true),
                ("work delete", "DELETE FROM ticket_work WHERE task_id = ?1", true),
                ("line delete", "DELETE FROM ticket_lines WHERE task_id = ?1", true),
            ];
            for (what, sql, gated) in steps {
                // A refused insert leaves no row for the next step to fence, so seed it past the guards.
                if !open && (sql.starts_with("UPDATE") || sql.starts_with("DELETE")) {
                    let seed = match what.split(' ').next().unwrap() {
                        "line" => "INSERT OR IGNORE INTO ticket_lines (task_id, stage, stage_since, created_at, resource_version) VALUES (?1,'waiting',1,1,1)",
                        "work" => "INSERT OR IGNORE INTO ticket_work (agent_id, task_id, role, started_at, linked_by) VALUES (?3, ?1, 'build', 1, 'orchestrator')",
                        _ => "INSERT OR IGNORE INTO task_blocks (task_id, blocked_by, reason) VALUES (?1, ?2, 'x')",
                    };
                    let c = s.conn();
                    let saved: Vec<String> = { let mut q = c.prepare("SELECT sql FROM sqlite_master WHERE type='trigger' AND (name LIKE 'workflow_excludes_%' OR name LIKE 'engine_fence_%')").unwrap();
                        q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
                    let names: Vec<String> = { let mut q = c.prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND (name LIKE 'workflow_excludes_%' OR name LIKE 'engine_fence_%')").unwrap();
                        q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
                    for n in &names { c.execute_batch(&format!("DROP TRIGGER {n}")).unwrap(); }
                    bind(&c, seed).unwrap();
                    for q in &saved { c.execute_batch(q).unwrap(); }
                }
                let done = run(sql);
                assert_eq!(done.is_err(), fenced(gated), "{phase:?} {} {what}: {done:?}", if open { "ticket" } else { "run" });
            }
        }
        // The column, on both: the engine's alone after release.
        for card in [b.ticket.id, b.run] {
            let moved = s.conn().execute("UPDATE tasks SET status='todo' WHERE id=?1", [card.as_bytes().as_slice()]);
            assert_eq!(moved.is_err(), phase == Phase::Released, "{phase:?}: a raw column write");
        }
    }
}

/// The real Prepare and Commit on a file at registry 67: `transfer::begin` lifts it to 68 inside the freeze, and
/// the closed commit maps and verifies it through the named delta.
#[test]
fn a_legacy_67_file_prepares_and_commits_through_the_real_path() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db.sqlite");
    {
        let store = Store::open(&path).unwrap();
        let repo = store.register_repository_for_test("Sixty-seven");
        let ws = store.ensure_main_workspace(repo).unwrap().id;
        store.create_task(ws, "history", Actor::User).unwrap();
    }
    roll_back_to_67(&path);
    let (store, plan) = transfer::begin(&path, join_db(&path, "a")).unwrap();
    assert_eq!(plan.registry, 67);
    assert_eq!(store.schema().unwrap().version, 68, "the freeze took registry 68");
    drop(store);
    closing::commit(&path, join_db(&path, "a")).expect("the commit conserves the source through the named delta");
    let owners = join_db(&path, "a");
    let store = startup::open_recovery(&path, &owners).unwrap();
    assert_eq!(store.engine_authority().unwrap().phase, "committed");
    let (_, plan) = transfer::bound_plan(&path, &store).unwrap();
    store.engine_verify(&plan).unwrap();
}

/// ov-965 B1: Start would keep a ticket's open subagents and blocks and then fence them, so a stopped subagent
/// could never end (the daemon read "Stopping" for good) and a block could never clear. Start is refused while
/// either is open, as Enqueue is; once they are settled it goes through, and the run's own fences are intact.
#[test]
fn start_waits_for_a_tickets_subagents_and_blocks_to_settle() {
    for phase in PHASES {
        let b = board(phase);
        let (s, t, user) = (&b.store, b.ticket.id, Actor::User);
        let tracker = Definition::tracker("owner");
        let start = |b: &Board, id: &str| b.apply(id, Command::Start { item: b.ticket.key.clone(), policy_pin: tracker.pin(), resource: "own-ticket".into(),
            dependencies: Default::default(), initial_candidate: Some(tracker.pin()), definition: tracker.clone(), models: Default::default() }).result.is_ok();
        s.record_worker(t, &worker("a1"), user).unwrap();
        assert!(!start(&b, "start-1"), "{phase:?}: an open subagent holds Start back");
        assert_eq!(b.resting(), vec![t], "{phase:?}: and the ticket stays at rest");
        s.end_worker(t, "claude", Some("a1"), EndReason::Finished, user).unwrap_or_else(|e| panic!("{phase:?}: so it can still end: {e:?}"));
        s.set_block(t, b.other, Some("because")).unwrap();
        assert!(!start(&b, "start-2"), "{phase:?}: an open block holds Start back");
        s.unblock(t, b.other).unwrap_or_else(|e| panic!("{phase:?}: so it can still clear: {e:?}"));
        assert!(start(&b, "start-3"), "{phase:?}: settled, Start goes through");
        assert!(s.record_worker(t, &worker("a2"), user).is_err(), "{phase:?}: and the run's records are fenced");
    }
}

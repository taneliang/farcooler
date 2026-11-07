use super::*;
use crate::caller_seat::Seat;
use farcooler_protocol::v1::{self as pb, Scope};
use farcooler_store::engine_cutover::{JoinedOwners, startup, transfer};

fn ask(method: &str, json: &str) -> Request {
    Request { request_id: bytes::Bytes::from_static(b"r"), method: method.into(),
        payload: Some(request::Payload::WorkflowCommand(pb::WorkflowCommand { schema: 1, json: json.into() })), ..Default::default() }
}
fn local() -> Peer { Peer { client_id: None, scope: Scope::HostAdmin } }
fn reply(r: Response) -> std::result::Result<serde_json::Value, String> {
    match r.outcome {
        Some(response::Outcome::Result(WireResult { value: Some(result::Value::WorkflowReply(w)) })) => Ok(serde_json::from_str(&w.json).unwrap()),
        other => Err(format!("{other:?}")),
    }
}
fn person(root: &std::path::Path) -> RecoveryHandler { RecoveryHandler::with_walk(root.into(), local(), Some(7), |_| Seat::Person, |_| Some(1)) }
async fn op(handler: &RecoveryHandler, json: &str) -> std::result::Result<serde_json::Value, String> { reply(handler.handle(ask("workflow.cutover", json)).await) }
/// Wait until nothing holds the database. Dropping a `Service` does not close
/// it: its background tasks hold the store, and with it the shared lock, until
/// they next run, which on a loaded machine is after the test's first Prepare
/// (ov-944: 13 of 160 runs refused "another process has this database open").
async fn released(root: &std::path::Path) {
    let db = crate::cutover_admin::database(root);
    let start = std::time::Instant::now();
    loop {
        match JoinedOwners::acquire(&db, "released") {
            Ok(free) => { drop(free); return; }
            Err(_) if start.elapsed() < std::time::Duration::from_secs(30) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            Err(e) => panic!("the dropped service never released the database: {e}"),
        }
    }
}
/// A Prepare that is expected to succeed. The database lock is released when
/// the last copy of its descriptor closes, and a test beside this one that
/// forks a child holds a copy until the child's exec, a few milliseconds
/// (ov-944: about 1 refusal in 300 runs). So ask again, for a while, until the
/// lock is free. A Prepare that fails changes nothing, so asking again is safe.
async fn prepare(handler: &RecoveryHandler) -> std::result::Result<serde_json::Value, String> {
    let start = std::time::Instant::now();
    loop {
        let said = op(handler, PREPARE).await;
        if said.is_ok() || start.elapsed() > std::time::Duration::from_secs(10) { return said; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
async fn closed_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let root = service.root_dir().to_path_buf();
    drop(service);
    released(&root).await;
    (dir, root)
}
/// The domain refusal: past the caller check, unlike the denial the seat test pins.
fn failed() -> String {
    format!("{:?}", error_response(bytes::Bytes::from_static(b"r"), DomainError::PageRefused { said: "That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see where the upgrade stands.".into() }).outcome)
}
const INSPECT: &str = r#"{"op":"Inspect"}"#;
const PREPARE: &str = r#"{"op":"Prepare","attempt":"a-1"}"#;

#[tokio::test]
async fn recovery_refuses_every_caller_but_a_person_outside_every_pane() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    transfer::hold(&db, "a-1").unwrap();
    let owner = person(&root);
    assert_eq!(op(&owner, INSPECT).await.unwrap()["artifacts"], true);
    let denied = format!("{:?}", error_response(bytes::Bytes::from_static(b"r"), DomainError::ScopeDenied { needed: "person" }).outcome);
    for seat in [(|_| Seat::Agent(None)) as SeatOf, |_| Seat::Agent(Some(uuid::Uuid::nil())), |_| Seat::Unreadable] {
        let handler = RecoveryHandler::with_walk(root.clone(), local(), Some(7), seat, |_| Some(1));
        for ask in [INSPECT, PREPARE, r#"{"op":"Recover"}"#, r#"{"op":"Release"}"#, r#"{"op":"Finish"}"#] {
            assert_eq!(op(&handler, ask).await.unwrap_err(), denied, "{ask}: a pane or agent is not the owner");
        }
    }
    let device = RecoveryHandler::with_walk(root.clone(), Peer { client_id: Some("device".into()), scope: Scope::HostAdmin }, Some(7), |_| Seat::Person, |_| Some(1));
    assert!(op(&device, INSPECT).await.is_err(), "a device is not the owner");
    let unknown = RecoveryHandler::with_walk(root.clone(), local(), None, |_| Seat::Person, |_| Some(1));
    assert!(op(&unknown, INSPECT).await.is_err(), "a caller with no pid cannot be walked");
    let gone = RecoveryHandler::with_walk(root.clone(), local(), Some(7), |_| Seat::Person, |_| None);
    assert!(op(&gone, INSPECT).await.is_err(), "nor one whose process cannot be identified");
    let held = format!("{:?}", error_response(bytes::Bytes::from_static(b"r"), DomainError::CutoverHeld).outcome);
    assert_eq!(reply(owner.handle(ask("host.get", INSPECT)).await).unwrap_err(), held, "no other method is served, and the refusal says why");
    assert!(startup::artifact_dir(&db).join("hold.json").exists(), "and nothing moved");
}
#[tokio::test]
async fn recovery_a_reused_pid_is_not_the_process_that_connected() {
    use std::sync::atomic::{AtomicI64, Ordering};
    static START: AtomicI64 = AtomicI64::new(1);
    let (_dir, root) = closed_root().await;
    let handler = RecoveryHandler::with_walk(root, local(), Some(7), |_| Seat::Person, |_| Some(START.load(Ordering::SeqCst)));
    assert!(op(&handler, INSPECT).await.is_ok());
    START.store(2, Ordering::SeqCst);
    assert!(op(&handler, INSPECT).await.is_err());
}
#[tokio::test]
async fn recovery_prepares_then_recovers_a_real_database_back_to_ordinary_service() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    let original = std::fs::read(&db).unwrap();
    transfer::hold(&db, "a-1").unwrap();
    assert!(crate::service::Service::open_in(root.clone()).await.is_err(), "ordinary startup is refused");
    let owner = person(&root);
    assert_eq!(prepare(&owner).await.unwrap()["phase"], "quiesced");
    let inspected = op(&owner, INSPECT).await.unwrap();
    assert_eq!(inspected["authority"]["phase"], "quiesced");
    assert!(inspected["ordinary_startup"].as_str().unwrap().starts_with("refused"));
    assert_eq!(inspected["baseline"]["attempt"], "a-1");
    for refused in [r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#, r#"{"op":"Finish"}"#, PREPARE] {
        assert_eq!(op(&owner, refused).await.unwrap_err(), failed(), "{refused} is refused before a commit");
    }
    assert_eq!(op(&owner, r#"{"op":"Recover"}"#).await.unwrap()["restored"], true);
    assert_eq!(std::fs::read(&db).unwrap(), original);
    assert_eq!(op(&owner, INSPECT).await.unwrap()["ordinary_startup"], "Existing");
    drop(crate::service::Service::open_in(root).await.expect("ordinary service is back"));
}
#[tokio::test]
async fn recovery_prepare_refuses_while_any_process_has_the_database_open() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    transfer::hold(&db, "a-1").unwrap();
    let held = JoinedOwners::acquire(&db, "other").unwrap();
    let refused = op(&person(&root), PREPARE).await.unwrap_err();
    assert!(refused.contains("Another process") && refused.contains("has this runner's database open. Close it") && !refused.contains("no proof issued"), "another owner blocks, named, in a sentence: {refused}");
    drop(held);
    assert!(prepare(&person(&root)).await.is_ok());
}
#[tokio::test]
async fn recovery_verify_release_and_finish_run_from_the_baseline_after_a_commit() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    transfer::hold(&db, "a-1").unwrap();
    let owner = person(&root);
    prepare(&owner).await.unwrap();
    assert!(op(&owner, r#"{"op":"Verify"}"#).await.is_err(), "not committed");
    let committed = op(&owner, r#"{"op":"Commit"}"#).await.unwrap();
    assert_eq!(committed["committed"], "a-1");
    assert!(std::path::Path::new(committed["backup"].as_str().unwrap()).join("db").is_file(), "with a closed backup");
    assert!(op(&owner, r#"{"op":"Recover"}"#).await.is_err(), "committed recovers forward only");
    assert!(op(&owner, r#"{"op":"Finish"}"#).await.is_err(), "not released");
    assert_eq!(op(&owner, r#"{"op":"Verify"}"#).await.unwrap()["verified"], "a-1");
    assert_eq!(op(&owner, r#"{"op":"Release"}"#).await.unwrap()["released"], "a-1");
    assert_eq!(op(&owner, r#"{"op":"Finish"}"#).await.unwrap()["evidence"].as_str().map(|p| p.contains("resolved-a-1")), Some(true));
    assert_eq!(op(&owner, INSPECT).await.unwrap()["ordinary_startup"], "Existing");
    assert!(op(&owner, r#"{"op":"Recover"}"#).await.is_err(), "nothing is left to recover");
}
#[tokio::test]
async fn live_hold_drains_marks_the_attempt_and_a_caller_lost_during_the_drain_marks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let db = crate::cutover_admin::database(service.root_dir());
    let refused = crate::cutover_admin::hold(&service, "a-1", || Err(crate::cutover_admin::Refused("revoked".into()))).await;
    assert!(refused.is_err());
    assert!(!startup::artifact_dir(&db).exists(), "a caller revoked during the drain leaves no mark");
    let dir = tempfile::tempdir().unwrap();
    let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let db = crate::cutover_admin::database(service.root_dir());
    let held = crate::cutover_admin::hold(&service, "a-1", || Ok(())).await.unwrap();
    assert_eq!(held["held"], "a-1");
    assert!(startup::artifact_dir(&db).join("hold.json").exists());
    let root = service.root_dir().to_path_buf();
    drop(service);
    released(&root).await;
    assert!(crate::service::Service::open_in(root.clone()).await.is_err(), "the next start is not ordinary");
    assert_eq!(prepare(&person(&root)).await.unwrap()["phase"], "quiesced");
}
#[tokio::test]
async fn live_prepare_refuses_an_agent_pane_and_holds_and_stops_for_a_person() {
    use farcooler_store::models::{TerminalRole, TerminalUpdate};
    use farcooler_transport::Handler;
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let t = svc.create_terminal(worktree.id, "cutover caller", "shell").await.unwrap();
    svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
        title: t.title, command_preset: "claude".into(), intent: t.intent,
        runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
        lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
    }).unwrap();
    svc.store.set_terminal_role(t.id, TerminalRole::Orchestrator).unwrap();
    svc.inventory.refresh().await;
    let pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == t.id).unwrap().stamp.pid as i32;
    assert_ne!(crate::caller_seat::recovery_seat(pid), Seat::Person, "recovery takes no pane for a person");
    let db = crate::cutover_admin::database(svc.root_dir());
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let factory = |pid| crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), stop.clone(), local()).with_caller_pid(pid);
    let refused = reply(factory(Some(pid)).handle(ask("workflow.cutover", PREPARE)).await);
    assert!(refused.is_err(), "an agent cannot strand the runner");
    assert!(!startup::artifact_dir(&db).exists());
    assert!(tokio::time::timeout(std::time::Duration::from_millis(400), stop.notified()).await.is_err(), "a refused caller stops nothing");
    let held = reply(factory(None).handle(ask("workflow.cutover", PREPARE)).await).unwrap();
    assert_eq!(held["held"], "a-1");
    assert!(startup::artifact_dir(&db).join("hold.json").exists());
    tokio::time::timeout(std::time::Duration::from_secs(2), stop.notified()).await.expect("the daemon stops into recovery");
}

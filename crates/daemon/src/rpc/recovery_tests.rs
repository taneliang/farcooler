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
async fn closed_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let service = crate::service::Service::open_in(dir.path().join("state")).await.unwrap();
    let root = service.root_dir().to_path_buf();
    drop(service);
    (dir, root)
}
/// The domain refusal, or the parser's for a word outside the vocabulary: either
/// way past the caller check, unlike the denial the seat test pins.
fn failed(json: &str) -> String {
    let error = if json.contains("Commit") { DomainError::NotFound } else { DomainError::OperationFailed };
    format!("{:?}", error_response(bytes::Bytes::from_static(b"r"), error).outcome)
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
    assert!(reply(owner.handle(ask("host.get", INSPECT)).await).is_err(), "no other method is served");
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
    assert_eq!(op(&owner, PREPARE).await.unwrap()["phase"], "quiesced");
    let inspected = op(&owner, INSPECT).await.unwrap();
    assert_eq!(inspected["authority"]["phase"], "quiesced");
    assert!(inspected["ordinary_startup"].as_str().unwrap().starts_with("refused"));
    assert_eq!(inspected["baseline"]["attempt"], "a-1");
    for refused in [r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#, r#"{"op":"Finish"}"#, r#"{"op":"Commit"}"#, PREPARE] {
        assert_eq!(op(&owner, refused).await.unwrap_err(), failed(refused), "{refused} is refused before a commit");
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
    assert!(op(&person(&root), PREPARE).await.is_err(), "another owner blocks; nothing is signaled");
    drop(held);
    assert!(op(&person(&root), PREPARE).await.is_ok());
}
#[tokio::test]
async fn recovery_verify_release_and_finish_run_from_the_baseline_after_a_commit() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    transfer::hold(&db, "a-1").unwrap();
    let owner = person(&root);
    op(&owner, PREPARE).await.unwrap();
    assert!(op(&owner, r#"{"op":"Verify"}"#).await.is_err(), "not committed");
    let store = startup::open_recovery(&db, &JoinedOwners::acquire(&db, "a-1").unwrap()).unwrap();
    farcooler_store::testing::exec_for_tests(&store, "UPDATE engine_cutover_authority SET phase='released', incarnation=1");
    drop(store);
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
    assert!(crate::service::Service::open_in(root.clone()).await.is_err(), "the next start is not ordinary");
    assert_eq!(op(&person(&root), PREPARE).await.unwrap()["phase"], "quiesced");
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

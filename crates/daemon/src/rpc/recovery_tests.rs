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
    format!("{:?}", error_response(bytes::Bytes::from_static(b"r"), DomainError::refused("That upgrade step was refused. Run `farcooler workflow-cutover inspect` to see where the upgrade stands.".into())).outcome)
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
    let asked = std::fs::read_to_string(startup::artifact_dir(&db).join("journal/commit-requested-1.json")).expect("the Commit names who asked");
    assert!(asked.contains("pid 7 (started 1), tty ") && asked.contains("session leader"), "{asked}");
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
/// A pane in `svc`'s main checkout, launched as `claude` with `role`, and the
/// pid of its root process: the caller a live Prepare walks.
async fn a_pane(svc: &std::sync::Arc<crate::service::Service>, repo: uuid::Uuid, role: farcooler_store::models::TerminalRole) -> (uuid::Uuid, i32) {
    use farcooler_store::models::TerminalUpdate;
    let worktree = svc.store.list_worktrees_for_repository(repo).unwrap().remove(0);
    let t = svc.create_terminal(worktree.id, "cutover caller", "shell").await.unwrap();
    svc.store.update_terminal(t.id, t.resource_version, TerminalUpdate {
        title: t.title, command_preset: "claude".into(), intent: t.intent,
        runtime_confirmed: t.runtime_confirmed, exit_code: t.exit_code, exit_signal: t.exit_signal,
        lease_generation: t.lease_generation, epoch: t.epoch, columns: t.columns, rows: t.rows,
    }).unwrap();
    svc.store.set_terminal_role(t.id, role).unwrap();
    svc.inventory.refresh().await;
    let pid = svc.inventory_snapshot().panes.iter().find(|p| p.terminal_id == t.id).unwrap().stamp.pid as i32;
    (t.id, pid)
}
/// The owner's "Allow cutover once", said from the app, to an ask on a card of
/// `pane`'s board (`cutover_grant`): the answer's id.
fn granted(svc: &std::sync::Arc<crate::service::Service>, pane: uuid::Uuid) -> uuid::Uuid {
    answered(svc, pane, "app")
}
/// That answer, said from `from`.
fn answered(svc: &std::sync::Arc<crate::service::Service>, pane: uuid::Uuid, from: &'static str) -> uuid::Uuid {
    use farcooler_store::models::{Actor, NoteKind};
    let workspace = svc.store.get_terminal(pane).unwrap().workspace_id.unwrap();
    let task = svc.store.create_task(workspace, "Data: the engine cutover", Actor::Manager).unwrap().id;
    svc.store.add_note(task, NoteKind::Question, Actor::Manager, "Run the cutover once?", serde_json::json!({ "options": [crate::cutover_grant::ALLOW] })).unwrap();
    let req = pb::TaskNoteAppend { task_id: task.as_bytes().to_vec().into(), kind: pb::TaskNoteKind::Answer as i32,
        body: crate::cutover_grant::ALLOW.into(), actor: "user".into(), ..Default::default() };
    let note = crate::task_ops::note_from(svc, &crate::watch::Watcher::new(svc.clone()), &req, Seat::Person, from).unwrap();
    uuid::Uuid::from_slice(&note.id).unwrap()
}
/// A live Prepare from `pid` (`None`: the in-process session, a person), its
/// reply, and whether it stopped the daemon.
async fn live_prepare(svc: &std::sync::Arc<crate::service::Service>, pid: Option<i32>) -> (std::result::Result<serde_json::Value, String>, bool) {
    use farcooler_transport::Handler;
    let stop = std::sync::Arc::new(tokio::sync::Notify::new());
    let factory = crate::rpc::RpcFactory::new(svc.clone(), crate::watch::Watcher::new(svc.clone()), stop.clone(), local()).with_caller_pid(pid);
    let said = reply(factory.handle(ask("workflow.cutover", PREPARE)).await);
    let wait = if said.is_ok() { 2000 } else { 400 };
    (said, tokio::time::timeout(std::time::Duration::from_millis(wait), stop.notified()).await.is_ok())
}
/// Nothing held, nobody pinned, nothing stopped.
fn untouched(svc: &crate::service::Service, (said, stopped): (std::result::Result<serde_json::Value, String>, bool), who: &str) {
    assert!(said.is_err(), "{who} cannot strand the runner: {said:?}");
    assert!(!stopped, "{who}: a refused caller stops nothing");
    assert!(!startup::artifact_dir(&crate::cutover_admin::database(svc.root_dir())).exists(), "{who}: no hold");
    assert!(!svc.root_dir().join(crate::cutover_delegate::FILE).exists(), "{who}: no pane pinned");
}

/// ov-974: a builder's pane is no orchestrator.
#[tokio::test]
async fn live_prepare_refuses_a_builder_pane() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let (pane, pid) = a_pane(&svc, repo, farcooler_store::models::TerminalRole::Agent).await;
    granted(&svc, pane);
    assert_ne!(crate::caller_seat::recovery_seat(pid), Seat::Person, "recovery takes no pane for a person");
    untouched(&svc, live_prepare(&svc, Some(pid)).await, "a builder pane");
}
/// ov-974: the cutover is the owner's, so a member's orchestrator, on the
/// login of the member who leads its workspace, is refused.
#[tokio::test]
async fn live_prepare_refuses_a_members_orchestrator() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let (pane, pid) = a_pane(&svc, repo, farcooler_store::models::TerminalRole::Orchestrator).await;
    let workspace = svc.store.get_terminal(pane).unwrap().workspace_id.unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
    let login_id = svc.store.ensure_member_login("bob").unwrap();
    svc.store.set_terminal_login(pane, &farcooler_store::claude_logins::TerminalLogin { handle: "bob".into(), login_id, config_dir: "/r/claude/bob".into(), shared: false }).unwrap();
    granted(&svc, pane);
    untouched(&svc, live_prepare(&svc, Some(pid)).await, "Bob's orchestrator");
}
/// ov-974: an orchestrator on the runner's login while Bob leads its
/// workspace is another lead's pane (`refuse_another_leads_login`).
#[tokio::test]
async fn live_prepare_refuses_an_orchestrator_on_another_leads_login() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let (pane, pid) = a_pane(&svc, repo, farcooler_store::models::TerminalRole::Orchestrator).await;
    let workspace = svc.store.get_terminal(pane).unwrap().workspace_id.unwrap();
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_workspace_lead(workspace, Some("bob")).unwrap();
    granted(&svc, pane);
    untouched(&svc, live_prepare(&svc, Some(pid)).await, "an orchestrator while Bob leads");
}
/// ov-974: the owner's own orchestrator still needs the owner's word for it:
/// none, or one typed at a CLI, which an agent can run too, holds nothing.
#[tokio::test]
async fn live_prepare_refuses_the_owners_orchestrator_without_the_owners_grant() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let (pane, pid) = a_pane(&svc, repo, farcooler_store::models::TerminalRole::Orchestrator).await;
    untouched(&svc, live_prepare(&svc, Some(pid)).await, "no grant");
    answered(&svc, pane, "cli");
    let (said, stopped) = live_prepare(&svc, Some(pid)).await;
    assert!(said.as_ref().is_err_and(|e| e.contains("allowed this cutover")), "{said:?}");
    untouched(&svc, (said, stopped), "an answer from a terminal");
}
/// ov-974: the owner's orchestrator holds, stops, and is pinned by its pane's
/// root process; a person's Prepare after it pins nobody.
#[tokio::test]
async fn live_prepare_holds_for_the_owners_orchestrator_and_pins_its_pane() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let (pane, pid) = a_pane(&svc, repo, farcooler_store::models::TerminalRole::Orchestrator).await;
    let answer = granted(&svc, pane);
    let workspace = svc.store.get_terminal(pane).unwrap().workspace_id.unwrap();
    let db = crate::cutover_admin::database(svc.root_dir());
    let (said, stopped) = live_prepare(&svc, Some(pid)).await;
    assert_eq!(said.unwrap()["held"], "a-1");
    assert!(stopped, "the daemon stops into recovery");
    let pinned: crate::cutover_delegate::Delegate = serde_json::from_slice(&std::fs::read(svc.root_dir().join(crate::cutover_delegate::FILE)).unwrap()).unwrap();
    assert_eq!((pinned.attempt.as_str(), pinned.terminal, pinned.pane_pid, pinned.grant), ("a-1", pane, pid, answer));
    assert_eq!(crate::cutover_grant::find(&svc.store, workspace, crate::step_up_rpc::now_ms()).unwrap(), None, "the Prepare spent the grant");
    assert_eq!(Some(pinned.pane_start), crate::claude_registry::process_incarnation(pid));
    assert!(startup::artifact_dir(&db).join("hold.json").exists());
}
/// A person holds and stops as before, and a pane an earlier Prepare pinned
/// is forgotten, so it never outlives the attempt it was pinned for.
#[tokio::test]
async fn live_prepare_holds_for_a_person_and_forgets_any_pinned_pane() {
    let (_dir, svc, _) = crate::test_support::fixture().await;
    std::fs::write(svc.root_dir().join(crate::cutover_delegate::FILE), "an earlier pane").unwrap();
    let (said, stopped) = live_prepare(&svc, None).await;
    assert_eq!(said.unwrap()["held"], "a-1");
    assert!(stopped, "the daemon stops into recovery");
    assert!(startup::artifact_dir(&crate::cutover_admin::database(svc.root_dir())).join("hold.json").exists());
    assert!(!svc.root_dir().join(crate::cutover_delegate::FILE).exists(), "no pane is pinned for a person");
}
/// ov-974: in recovery, a caller inside the pinned pane runs every step, and
/// Commit names it as the orchestrator acting for the owner; Finish forgets it.
#[tokio::test]
async fn recovery_runs_the_pinned_orchestrator_from_prepare_to_finish() {
    let (_dir, root) = closed_root().await;
    let db = crate::cutover_admin::database(&root);
    transfer::hold(&db, "a-1").unwrap();
    std::fs::write(root.join(crate::cutover_delegate::FILE), "pinned").unwrap();
    fn pinned(_: &std::path::Path, _: i32) -> Option<crate::cutover_delegate::Delegate> {
        Some(crate::cutover_delegate::Delegate { attempt: "a-1".into(), terminal: uuid::Uuid::nil(), workspace: uuid::Uuid::max(), pane_pid: 9, pane_start: 1, grant: uuid::Uuid::from_u128(1) })
    }
    let unpinned = RecoveryHandler::with_walk(root.clone(), local(), Some(7), |_| Seat::Unreadable, |_| Some(1));
    assert!(op(&unpinned, INSPECT).await.is_err(), "a pane nobody pinned");
    let orchestrator = RecoveryHandler::with_walk(root.clone(), local(), Some(7), |_| Seat::Unreadable, |_| Some(1)).with_delegate(pinned);
    prepare(&orchestrator).await.unwrap();
    op(&orchestrator, r#"{"op":"Commit"}"#).await.unwrap();
    let asked = std::fs::read_to_string(startup::artifact_dir(&db).join("journal/commit-requested-1.json")).unwrap();
    assert!(asked.contains(&format!("the owner's orchestrator (terminal {}, workspace {}), acting for the owner on their grant (answer {}): pid 7", uuid::Uuid::nil(), uuid::Uuid::max(), uuid::Uuid::from_u128(1))), "{asked}");
    for step in [r#"{"op":"Verify"}"#, r#"{"op":"Release"}"#, r#"{"op":"Finish"}"#] { op(&orchestrator, step).await.unwrap_or_else(|e| panic!("{step}: {e}")); }
    assert!(!root.join(crate::cutover_delegate::FILE).exists(), "Finish forgets the pane");
}

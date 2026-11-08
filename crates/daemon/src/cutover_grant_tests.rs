//! ov-974: the owner's grant, by the path a request takes (`task_ops::note_from`
//! with the seat and `said_from` the RPC layer reads). The real processes
//! behind a seat are `watch::answer_wake::answer_seat_tests`'.
use super::*;
use crate::caller_seat::Seat;
use farcooler_protocol::v1 as pb;

struct Card { _dir: crate::test_support::ScratchDir, svc: std::sync::Arc<crate::service::Service>, watcher: std::sync::Arc<crate::watch::Watcher>, workspace: Uuid, task: Uuid }

/// A card on the main board, asked "Allow cutover once?" by the orchestrator.
async fn asked_card() -> Card {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let task = svc.store.create_task(workspace, "Data: the engine cutover", Actor::Manager).unwrap().id;
    svc.store.add_note(task, NoteKind::Question, Actor::Manager, "Can your orchestrator run the cutover once today?",
        serde_json::json!({ "options": [ALLOW, "Wait for me"] })).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());
    Card { _dir: dir, svc, watcher, workspace, task }
}

fn answer(card: &Card, actor: &str, body: &str, extra: &str) -> pb::TaskNoteAppend {
    pb::TaskNoteAppend { task_id: card.task.as_bytes().to_vec().into(), kind: pb::TaskNoteKind::Answer as i32, body: body.into(),
        actor: actor.into(), extra_json: extra.into(), ..Default::default() }
}
/// The owner's answer, from `seat`, said from `from`.
fn said(card: &Card, seat: Seat, from: &'static str) -> Result<pb::TaskNote> {
    crate::task_ops::note_from(&card.svc, &card.watcher, &answer(card, "user", ALLOW, ""), seat, from)
}
fn now() -> i64 { crate::step_up_rpc::now_ms() }

#[test]
fn only_the_apps_signature_and_the_owners_phone_key_say_a_grant() {
    let local = Peer { client_id: None, scope: farcooler_protocol::v1::Scope::HostAdmin };
    let phone = Peer { client_id: Some("phone-1".into()), scope: farcooler_protocol::v1::Scope::Control };
    let app = |pid: i32| pid == 10;
    let signed = |method: &str, device: &str, browser: bool| crate::step_up::Stepped { method: method.into(), digest: [0; 32], device: device.into(), browser };
    let answer = signed("task.note", "phone-1", false);
    assert_eq!(said_from_with(&local, Some(10), None, || true, &app), "app", "the app, by its code signature");
    assert_eq!(said_from_with(&local, Some(11), None, || true, &app), "cli", "a CLI in any terminal");
    assert_eq!(said_from_with(&local, None, None, || true, &app), "cli", "the in-process session");
    assert_eq!(said_from_with(&local, Some(11), Some(&answer), || true, &app), "cli", "the socket has no device to sign");
    assert_eq!(said_from_with(&phone, Some(10), Some(&answer), || true, &app), "device", "the owner's phone, by its key");
    assert_eq!(said_from_with(&phone, Some(10), None, || true, &app), "cli", "a line named for the phone, unsigned: a preamble anyone local can write");
    assert_eq!(said_from_with(&phone, None, Some(&signed("task.note", "phone-2", false)), || true, &app), "cli", "another device's key");
    assert_eq!(said_from_with(&phone, None, Some(&signed("member.set", "phone-1", false)), || true, &app), "cli", "a step-up for something else");
    assert_eq!(said_from_with(&phone, None, Some(&signed("task.note", "phone-1", true)), || true, &app), "cli", "a browser's passkey");
    assert_eq!(said_from_with(&phone, None, Some(&answer), || false, &app), "cli", "a member's or a released device");
}

#[tokio::test]
async fn the_owners_answer_from_the_app_grants_once() {
    let card = asked_card().await;
    let note = said(&card, Seat::Person, "app").unwrap();
    let written = card.svc.store.notes_for(card.task, Some(NoteKind::Answer)).unwrap().pop().unwrap();
    assert_eq!((written.actor, written.extra[SAID_FROM].as_str()), (Actor::User, Some("app")), "the daemon's stamp");
    let grant = spend(&card.svc.store, card.workspace, "a-1", now()).expect("granted");
    assert_eq!(grant.answer.as_bytes().as_slice(), note.id.as_ref());
    assert_eq!(find(&card.svc.store, card.workspace, now()).unwrap(), None, "spent");
    assert!(spend(&card.svc.store, card.workspace, "a-2", now()).is_err(), "a spent grant admits no second Prepare");
    let record = card.svc.store.notes_for(card.task, Some(NoteKind::Progress)).unwrap();
    assert!(record.iter().any(|n| n.actor == Actor::Runner && n.body.contains("a-1")), "the spend is on the card: {record:?}");
}

#[tokio::test]
async fn the_owners_answer_from_their_phone_grants() {
    let card = asked_card().await;
    said(&card, Seat::Person, "device").unwrap();
    assert!(find(&card.svc.store, card.workspace, now()).unwrap().is_some());
}

#[tokio::test]
async fn no_answer_grants_nothing() {
    let card = asked_card().await;
    assert!(spend(&card.svc.store, card.workspace, "a-1", now()).is_err());
}

#[tokio::test]
async fn an_agents_answer_grants_nothing_whatever_it_claims() {
    let card = asked_card().await;
    // `task note --kind answer` from the orchestrator's pane: walked to an agent.
    let pane = card.svc.store.create_terminal_for_test(card.svc.store.list_worktrees_for_repository(card.svc.store.get_workspace(card.workspace).unwrap().repository_id).unwrap()[0].id, card.workspace);
    said(&card, Seat::Agent(Some(pane)), "app").unwrap();
    said(&card, Seat::Agent(None), "device").unwrap();
    // `--actor manager`, and a pane's own agent id: not a person's answer, never walked.
    for actor in ["manager".to_string(), format!("agent:{pane}")] {
        crate::task_ops::note_from(&card.svc, &card.watcher, &answer(&card, &actor, ALLOW, ""), Seat::Person, "app").unwrap();
    }
    // A caller that writes the stamp itself is refused outright.
    let forged = answer(&card, "user", ALLOW, r#"{"said_from":"app"}"#);
    assert!(matches!(crate::task_ops::note_from(&card.svc, &card.watcher, &forged, Seat::Person, "cli"), Err(DomainError::InvalidArgument { what: "extra_json" })));
    assert_eq!(find(&card.svc.store, card.workspace, now()).unwrap(), None);
}

#[tokio::test]
async fn a_persons_answer_from_a_terminal_grants_nothing() {
    let card = asked_card().await;
    said(&card, Seat::Person, "cli").unwrap();
    assert_eq!(find(&card.svc.store, card.workspace, now()).unwrap(), None, "a CLI path an agent can also run");
}

#[tokio::test]
async fn an_old_grant_has_lapsed() {
    let card = asked_card().await;
    said(&card, Seat::Person, "app").unwrap();
    assert!(find(&card.svc.store, card.workspace, now() + MAX_AGE_MS - 60_000).unwrap().is_some(), "a day less a minute");
    assert_eq!(find(&card.svc.store, card.workspace, now() + MAX_AGE_MS + 60_000).unwrap(), None, "a day and a minute");
}

#[tokio::test]
async fn a_grant_answers_an_ask_on_its_own_board() {
    let card = asked_card().await;
    let unasked = card.svc.store.create_task(card.workspace, "Data: another card", Actor::Manager).unwrap().id;
    let mut req = answer(&card, "user", ALLOW, "");
    req.task_id = unasked.as_bytes().to_vec().into();
    crate::task_ops::note_from(&card.svc, &card.watcher, &req, Seat::Person, "app").unwrap();
    crate::task_ops::note_from(&card.svc, &card.watcher, &answer(&card, "user", "Allow cutover once, after lunch", ""), Seat::Person, "app").unwrap();
    assert_eq!(find(&card.svc.store, card.workspace, now()).unwrap(), None, "no question offered it there, or it's not the option");
    let other = card.svc.store.create_workspace(card.svc.store.get_workspace(card.workspace).unwrap().repository_id, "Other", "oth").unwrap().id;
    said(&card, Seat::Person, "app").unwrap();
    assert_eq!(find(&card.svc.store, other, now()).unwrap(), None, "another board's orchestrator");
    assert!(find(&card.svc.store, card.workspace, now()).unwrap().is_some());
}

/// The owner's phone says it by signing the answer with its trusted key
/// (ov-856), through `Rpc` as its connection does: stamped `device`, and it
/// grants. The same answer unsigned from that phone's line, which a local
/// process can name in a preamble, doesn't.
#[tokio::test]
async fn the_owners_phone_grants_by_signing_its_answer() {
    use crate::rpc_people::tests::call;
    use crate::step_up::tests::{Device, nonce, runner};
    let r = runner().await;
    let root = r.svc.store.create_repository_root(r.svc.host_id, "/r", 0).unwrap();
    let repo = r.svc.store.create_repository(r.svc.host_id, root.id, "demo", "/r/demo/.git", "").unwrap();
    let workspace = r.svc.store.ensure_main_workspace(repo.id).unwrap().id;
    let task = r.svc.store.create_task(workspace, "Data: the engine cutover", Actor::Manager).unwrap().id;
    r.svc.store.add_note(task, NoteKind::Question, Actor::Manager, "Run the cutover once?", serde_json::json!({ "options": [ALLOW] })).unwrap();
    let answer = || pb::Request { method: "task.note".into(), payload: Some(pb::request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: task.as_bytes().to_vec().into(), kind: pb::TaskNoteKind::Answer as i32, body: ALLOW.into(), actor: "user".into(), ..Default::default() })), ..Default::default() };
    let phone = Device::new("phone-o", 11);
    r.trust(&phone, crate::step_up::ALG_DEVICE);
    let conn = r.conn("phone-o");

    call(&conn, answer()).await.expect("an unsigned answer is still an answer");
    assert_eq!(find(&r.svc.store, workspace, now()).unwrap(), None, "unsigned");

    let n = nonce(&conn).await;
    call(&conn, phone.step(&r.pin, answer(), &n)).await.expect("a signed answer");
    let said = r.svc.store.notes_for(task, Some(NoteKind::Answer)).unwrap().pop().unwrap();
    assert_eq!((said.actor, said.extra[SAID_FROM].as_str()), (Actor::User, Some("device")));
    assert!(find(&r.svc.store, workspace, now()).unwrap().is_some(), "the owner's phone, by its key");

    let stolen = Device::new("phone-o", 12);
    let n = nonce(&conn).await;
    assert!(call(&conn, stolen.step(&r.pin, answer(), &n)).await.is_err(), "a key the runner never trusted");
}

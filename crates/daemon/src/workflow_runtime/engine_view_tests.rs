//! ov-1486: the web's Engine view acting on real stuck runs, through dispatch. Each command is the one the
//! shared core makes (`farcooler_client::engine_page`) from the runner's own reply, so what a person's press
//! sends and what an agent's session is refused are the same bytes.
//!
//! Every refusal here was watched to go red with its check taken out (see the commit message).
use super::*;
use super::admission_tests::{factory, receipt, refused, sent, snapshot};
use super::coding_tests::{admitted_in, configured};
use super::coding_verbs_tests::{batched_push, lander};
use super::concurrency_tests::enqueue_item;
use super::seat_tests::owners_pane;
use crate::{rpc::RpcFactory, service::Service};
use farcooler_client::engine_page::{self, Kind, Page, Selection};
use farcooler_protocol::{v1::{self as pb, request, response, Scope}, workflow_commands::Action};
use farcooler_store::models::{NoteKind, TerminalRole};
use farcooler_transport::{Handler, Peer};
use std::sync::Arc;

const WHY: &str = "I checked the worktree, and every process this step owned has ended.";

/// A runner with one workspace, whose runs are kept under that workspace's id.
struct Board {
    _service_dir: crate::test_support::ScratchDir,
    _dir: tempfile::TempDir,
    svc: Arc<Service>,
    repo: Uuid,
    workspace: Uuid,
    runtime: Runtime,
}

impl Board {
    async fn new(make: impl AsyncFnOnce(&str) -> (tempfile::TempDir, Runtime)) -> Self {
        let (service_dir, svc, repo) = crate::test_support::fixture().await;
        let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
        let (dir, runtime) = make(&workspace.to_string()).await;
        Self { _service_dir: service_dir, _dir: dir, svc, repo, workspace, runtime }
    }

    /// The signed-in person's browser: the owner's device, over ssh.
    fn browser(&self) -> RpcFactory {
        self.svc.store.bind_owner_device("browser", None).unwrap();
        factory(&self.svc, &self.runtime, Peer { client_id: Some("browser".into()), scope: Scope::Control }, None)
    }

    fn pane(&self, pid: i32) -> RpcFactory {
        factory(&self.svc, &self.runtime, Peer { client_id: None, scope: Scope::Control }, Some(pid))
    }

    /// A pane of the owner's, in this workspace unless `other` is given.
    async fn agent(&self, role: TerminalRole, other: Option<Uuid>) -> RpcFactory {
        let worktree = self.svc.store.list_worktrees_for_repository(self.repo).unwrap().remove(0).id;
        let (_, pid) = owners_pane(&self.svc, worktree, other.unwrap_or(self.workspace), role).await;
        self.pane(pid)
    }

    async fn raw(&mut self, rpc: &RpcFactory) -> serde_json::Value {
        receipt(&sent(&mut self.runtime, rpc, self.workspace, &Action::Get).await.0)
    }

    async fn page(&mut self, rpc: &RpcFactory) -> (serde_json::Value, Page) {
        let raw = self.raw(rpc).await;
        let page = engine_page::page(self.workspace, &raw, None, Vec::new(), None).expect("the Engine view reads");
        (raw, page)
    }

    /// The command a person's press on `item`'s `kind` button sends, made as the web makes it.
    async fn press(&mut self, rpc: &RpcFactory, item: &str, kind: Kind) -> Action {
        let (raw, page) = self.page(rpc).await;
        let offers = page.actions.get(item).unwrap_or_else(|| panic!("nothing offered for {item}: {:?}", page.actions));
        let offer = offers.iter().find(|o| o.kind == kind)
            .unwrap_or_else(|| panic!("{kind:?} isn't offered: {:?}", offers.iter().map(|o| o.kind).collect::<Vec<_>>()));
        let selection = Selection { item: item.into(), kind, guard: offer.guard.clone(),
            reason: offer.reason_label.is_some().then(|| WHY.into()) };
        assert!(engine_page::missing_reason(&selection).is_none());
        engine_page::command(self.workspace, &raw, &selection).unwrap().expect("the row is as it was seen")
    }

    /// `action` from `rpc` is refused at dispatch or admission, and nothing is written.
    async fn is_refused(&mut self, who: &str, rpc: &RpcFactory, action: &Action) {
        let before = snapshot(&self.runtime, self.workspace);
        let (reply, _) = sent(&mut self.runtime, rpc, self.workspace, action).await;
        assert!(matches!(reply.outcome, Some(response::Outcome::Error(_))), "{who} was admitted: {reply:?}");
        assert_eq!(refused(&reply), "scope-denied", "{who}");
        assert_eq!(snapshot(&self.runtime, self.workspace), before, "{who} wrote");
    }

    async fn accepted(&mut self, rpc: &RpcFactory, kind: Kind, action: &Action) {
        let (reply, reached) = sent(&mut self.runtime, rpc, self.workspace, action).await;
        assert!(reached, "the command never reached the engine: {reply:?}");
        let outcome = engine_page::receipt_outcome(kind, &receipt(&reply));
        assert!(outcome.ok, "{outcome:?}");
    }

    fn phase(&self, item: &str) -> Phase {
        let state = self.runtime.state(&self.workspace.to_string()).unwrap().unwrap();
        state.items.get(item).map(|i| i.phase.clone()).or_else(|| state.archive.get(item).map(|a| a.phase.clone())).unwrap()
    }

    /// Every other session that isn't a person's: a builder's pane, and the orchestrator of another workspace.
    async fn strangers(&self) -> [(&'static str, RpcFactory); 2] {
        let other = self.svc.store.create_workspace(self.repo, "Other", "oth").unwrap().id;
        [("a builder's pane", self.agent(TerminalRole::Agent, None).await),
         ("another workspace's orchestrator", self.agent(TerminalRole::Orchestrator, Some(other)).await)]
    }
}

/// A run waiting at its first step.
async fn waiting(workspace: &str) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&dir.path().join("runtime")).await.unwrap();
    let mut definition = coding_definition(false);
    definition.stages[0].progression = Progression::AwaitApproval { principal: "owner".into() };
    let pin = definition.pin();
    let m = super::admission_tests::mutation;
    runtime.wire_command(workspace, "owner", Action::Publish { reason: None, notifications: None, adapter: None,
        mutation: m("p", 0, 1), expected_pin: None, definition }).unwrap();
    runtime.wire_command(workspace, "owner", Action::Enqueue { mutation: m("e", 0, 1), item: "ov-1".into(), pin }).unwrap();
    (dir, runtime)
}

/// A worktree step whose settlement was lost: it reads Uncertain, and nothing was made.
async fn lost_provision(workspace: &str) -> (tempfile::TempDir, Runtime) {
    let (dir, mut runtime) = configured(false, "pass").await;
    let definition = runtime.configured_definition().unwrap();
    super::tests::enqueue(&mut runtime, workspace, "one", definition);
    admitted_in(&runtime, workspace).await;
    runtime.pump(200).await.unwrap();
    assert_eq!(runtime.state(workspace).unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    (dir, runtime)
}

/// The same, then cancelled, with the cancellation's own receipt lost too (as `lost_cancellation` makes it).
async fn lost_cancellation(workspace: &str) -> (tempfile::TempDir, Runtime) {
    let (dir, mut runtime) = lost_provision(workspace).await;
    let state = runtime.state(workspace).unwrap().unwrap();
    let item = &state.items["one"];
    let mut mutation = super::admission_tests::mutation("cancel", state.revision, 100);
    mutation.epoch = state.epoch;
    runtime.wire_command(workspace, "owner", Action::Cancel { mutation, item: "one".into(),
        run: item.run.clone(), pin: item.pin.clone() }).unwrap();
    runtime.pump(201).await.unwrap();
    let (root, store) = (runtime.root.clone(), runtime.store.clone());
    drop(runtime);
    let runtime = Runtime::open_service(&root, store).unwrap();
    assert_eq!(runtime.state(workspace).unwrap().unwrap().items["one"].phase, Phase::Uncertain);
    (dir, runtime)
}

/// A landing that pushed to a real (local, bare) remote and lost the word that it had.
async fn lost_push(workspace: &str) -> (tempfile::TempDir, Runtime) {
    let (dir, mut runtime) = lander("pass").await;
    let definition = batched_push(&runtime, "any", 5000);
    enqueue_item(&mut runtime, workspace, "one", definition);
    for time in 2..200 {
        runtime.pump(time).await.unwrap();
        if runtime.state(workspace).unwrap().unwrap().items["one"].phase == Phase::WaitingJoin { break; }
    }
    let state = runtime.state(workspace).unwrap().unwrap();
    let controller = runtime.controller();
    let queued = runtime.submit_now(&controller, Request { id: "queue".into(), workspace: workspace.into(),
        expected_revision: state.revision, sequence: state.source_sequences.get("controller").copied().unwrap_or(0) + 1,
        command: Command::Queue { item: "one".into() } }, 1).unwrap();
    assert!(queued.result.is_ok(), "{:?}", queued.result);
    for time in 2..5 { runtime.pump(time).await.unwrap(); }
    let binding = admitted_in(&runtime, workspace).await;
    let (_hold, cancelled) = tokio::sync::watch::channel(false);
    runtime.execute_coding(workspace, &binding, cancelled).await.unwrap();
    std::fs::remove_file(runtime.verb_path(&binding)).unwrap();
    let root = runtime.root.clone();
    drop(runtime);
    let mut runtime = Runtime::open(&root).unwrap();
    runtime.pump(50).await.unwrap();
    assert!(runtime.state(workspace).unwrap().unwrap().items.values()
        .any(|i| i.group_of.is_some() && i.phase == Phase::Uncertain), "the push is lost");
    (dir, runtime)
}

#[tokio::test]
async fn ov1486_a_person_cancels_a_run_from_the_engine_view() {
    let mut board = Board::new(waiting).await;
    let browser = board.browser();
    let cancel = board.press(&browser, "ov-1", Kind::Cancel).await;
    assert!(matches!(cancel, Action::Cancel { .. }));
    for (who, rpc) in &board.strangers().await {
        board.is_refused(who, rpc, &cancel).await;
    }
    board.accepted(&browser, Kind::Cancel, &cancel).await;
    assert!(matches!(board.phase("ov-1"), Phase::Cancelled | Phase::Cancelling), "{:?}", board.phase("ov-1"));
}

#[tokio::test]
async fn ov1486_a_person_resolves_an_uncertain_step_from_the_engine_view() {
    let mut board = Board::new(lost_provision).await;
    let browser = board.browser();
    let resolve = board.press(&browser, "one", Kind::Resolve).await;
    assert!(matches!(resolve, Action::Resolve { .. }));
    for (who, rpc) in &board.strangers().await {
        board.is_refused(who, rpc, &resolve).await;
    }
    board.accepted(&browser, Kind::Resolve, &resolve).await;
    board.runtime.pump(205).await.unwrap();
    assert_eq!(board.phase("one"), Phase::Failed, "the step ended with nothing made, and can run again");
}

#[tokio::test]
async fn ov1486_a_person_resolves_a_lost_cancellation_and_no_agent_or_orchestrator_does() {
    let mut board = Board::new(lost_cancellation).await;
    let browser = board.browser();
    let resolve = board.press(&browser, "one", Kind::ResolveCancellation).await;
    assert!(matches!(&resolve, Action::ResolveCancellation { reason, .. } if reason == WHY));
    for (who, rpc) in &board.strangers().await {
        board.is_refused(who, rpc, &resolve).await;
    }
    // The workspace's own orchestrator too, and its view never offers it.
    let orchestrator = board.agent(TerminalRole::Orchestrator, None).await;
    let (_, theirs) = board.page(&orchestrator).await;
    assert!(theirs.actions["one"].iter().all(|o| o.kind != Kind::ResolveCancellation));
    board.is_refused("the workspace's orchestrator", &orchestrator, &resolve).await;
    board.accepted(&browser, Kind::ResolveCancellation, &resolve).await;
    assert_eq!(board.phase("one"), Phase::Cancelled);
}

#[tokio::test]
async fn ov1486_a_person_settles_a_lost_push_and_no_agent_or_orchestrator_does() {
    let mut board = Board::new(lost_push).await;
    let browser = board.browser();
    let (_, page) = board.page(&browser).await;
    let group = page.actions.iter().find(|(_, offers)| offers.iter().any(|o| o.kind == Kind::SettleLanded))
        .map(|(id, _)| id.clone()).expect("the stuck landing offers to be settled");
    assert!(page.view.landings.iter().any(|landing| landing.id() == group), "the landing is on the view");
    let not_landed = board.press(&browser, &group, Kind::SettleNotLanded).await;
    let landed = board.press(&browser, &group, Kind::SettleLanded).await;
    assert!(matches!(landed, Action::Settle { landed: true, .. }));
    let orchestrator = board.agent(TerminalRole::Orchestrator, None).await;
    let (_, theirs) = board.page(&orchestrator).await;
    assert!(theirs.actions.get(&group).into_iter().flatten()
        .all(|o| !matches!(o.kind, Kind::SettleLanded | Kind::SettleNotLanded)));
    let strangers = board.strangers().await;
    for action in [&landed, &not_landed] {
        for (who, rpc) in &strangers {
            board.is_refused(who, rpc, action).await;
        }
        board.is_refused("the workspace's orchestrator", &orchestrator, action).await;
    }
    // The runner reads the remote: the commit is there, so "not landed" is refused even from a person.
    let (reply, _) = sent(&mut board.runtime, &browser, board.workspace, &not_landed).await;
    let refusal = engine_page::receipt_outcome(Kind::SettleNotLanded, &receipt(&reply));
    assert!(!refusal.ok, "{refusal:?}");
    board.accepted(&browser, Kind::SettleLanded, &landed).await;
    for time in 60..200 {
        board.runtime.pump(time).await.unwrap();
        if board.phase("one") == Phase::Done { break; }
    }
    assert_eq!(board.phase("one"), Phase::Done, "the settled landing went on");
}

/// R-107 and ov-999 give a workspace's own orchestrator cancel and resolve: it supervises the engine through
/// them. ov-1486 leaves that as it is; the person-only commands above are the ones it is refused.
#[tokio::test]
async fn ov1486_the_workspaces_own_orchestrator_keeps_the_cancel_that_r107_gives_it() {
    let mut board = Board::new(waiting).await;
    let browser = board.browser();
    let cancel = board.press(&browser, "ov-1", Kind::Cancel).await;
    let orchestrator = board.agent(TerminalRole::Orchestrator, None).await;
    let (reply, reached) = sent(&mut board.runtime, &orchestrator, board.workspace, &cancel).await;
    assert!(reached);
    super::admission_tests::ok(&reply);
}

#[tokio::test]
async fn ov1486_a_press_on_a_row_that_has_moved_makes_no_command() {
    let mut board = Board::new(waiting).await;
    let browser = board.browser();
    let (raw, page) = board.page(&browser).await;
    let offer = page.actions["ov-1"].iter().find(|o| o.kind == Kind::Cancel).unwrap();
    let selection = Selection { item: "ov-1".into(), kind: Kind::Cancel, guard: offer.guard.clone(), reason: None };
    let cancel = engine_page::command(board.workspace, &raw, &selection).unwrap().unwrap();
    board.accepted(&browser, Kind::Cancel, &cancel).await;
    let after = board.raw(&browser).await;
    assert_eq!(engine_page::command(board.workspace, &after, &selection).unwrap(), None);
}

fn note(card: Uuid, body: &str) -> pb::Request {
    let mut req = farcooler_transport::request("task.note");
    req.payload = Some(request::Payload::TaskNoteAppend(pb::TaskNoteAppend {
        task_id: bytes::Bytes::copy_from_slice(card.as_bytes()), kind: pb::TaskNoteKind::Answer as i32,
        body: body.into(), extra_json: String::new(), supersedes: None,
        // What the web sends. The runner, not the caller, says who wrote it.
        actor: "user".into(),
    }));
    req
}

/// A builder's pane and the orchestrator's own pane send the very answer the Approve and Decline buttons send,
/// claiming to be the person. Neither decides the draft, and neither reaches the engine.
#[tokio::test]
async fn ov1486_an_agents_or_the_orchestrators_answer_decides_no_draft() {
    let mut board = Board::new(async |_| lander("pass").await).await;
    let config = board.runtime.coding.clone().unwrap();
    let draft = crate::landing_templates::template("ticket-push", &config, &["make check".to_string()], &[], &Default::default()).unwrap();
    let browser = board.browser();
    let (reply, _) = sent(&mut board.runtime, &browser, board.workspace, &Action::Propose {
        reason: "Gate every push on make check.".into(), definition: draft.definition, adapter: Box::new(draft.adapter) }).await;
    let filed = receipt(&reply);
    let (id, card) = (filed["draft"]["id"].as_str().unwrap().to_owned(), filed["draft"]["card"].as_str().unwrap().parse::<Uuid>().unwrap());
    let detail = detail(&board, card);
    let asked = engine_page::draft(&id, &detail);
    assert!(asked.problem.is_none() && asked.question.contains("make check"), "{asked:?}");
    let orchestrator = board.agent(TerminalRole::Orchestrator, None).await;
    let builder = board.agent(TerminalRole::Agent, None).await;
    for approve in [true, false] {
        for (who, rpc) in [("the workspace's orchestrator", &orchestrator), ("a builder's pane", &builder)] {
            let words = engine_page::decision(&id, &asked.note, approve, &self::detail(&board, card))
                .expect("the card still asks the question the person saw");
            let pending = rpc.handle(note(card, &words));
            tokio::select! {
                reply = pending => assert!(matches!(reply.outcome, Some(response::Outcome::Result(_))), "{who}: {reply:?}"),
                _ = board.runtime.inbox.recv() => panic!("{who}'s answer reached the engine as a person's"),
            }
            let kept = crate::workflow_drafts::get(&board.svc, &id).unwrap().unwrap();
            assert_eq!(kept.state, crate::workflow_drafts::State::Pending, "{who} decided the draft with {words}");
            assert_eq!(kept.answer, None, "{who}");
            let said = board.svc.store.notes_for(card, Some(NoteKind::Answer)).unwrap().pop().unwrap();
            assert_ne!(said.actor.to_string(), "user", "{who}'s answer was filed as the person's");
        }
    }
}

fn detail(board: &Board, card: Uuid) -> pb::TaskDetail {
    let notes = board.svc.store.notes_for(card, None).unwrap().iter().map(crate::task_ops::pb_note).collect();
    pb::TaskDetail { task: None, notes, blocks: Vec::new() }
}

const LONG_TITLES: [&str; 4] = [
    "Engine: land a batched push even when the target branch moves twice while the local gate is still running on the folded commit",
    "Web: keep the orchestrator’s message box usable after a pane is closed, reopened and resized on a 320-pixel phone",
    "Daemon: bound every workflow read so a board with hundreds of finished runs still answers inside one control frame",
    "Relay: refuse an enrollment whose passkey assertion names another account, and say which one in the log",
];

fn long_title(n: usize) -> String { format!("{} (part {n})", LONG_TITLES[n % LONG_TITLES.len()]) }

/// Every ticket's row on the view, wherever it stands.
fn shown(page: &serde_json::Value) -> std::collections::BTreeSet<String> {
    let rows = |of: &serde_json::Value| of["tickets"].as_array().unwrap().iter()
        .map(|t| t["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    page["view"]["stations"].as_array().unwrap().iter().chain(page["view"]["landings"].as_array().unwrap())
        .flat_map(rows).collect()
}

/// A board with 340 items whose whole state is far past one control frame: the Engine view reads it in one
/// reply that fits, and shows every ticket. With `FARCOOLER_ENGINE_FIXTURE` set, the page this real read made
/// is written there for the browser check (`services/web/test/browser/engine-check.mjs`).
#[tokio::test]
async fn ov1486_a_board_of_340_items_loads_in_one_bounded_read() {
    use farcooler_client::plan_pages::PlanPages;
    let mut board = Board::new(waiting).await;
    let key = board.workspace.to_string();
    let mut state = board.runtime.state(&key).unwrap().unwrap();
    let mut run = state.items.remove("ov-1").unwrap();
    run.phase = Phase::Running;
    let task_keys: Vec<String> = (0..340).map(|n| board.svc.store
        .create_task(board.workspace, &long_title(n), farcooler_store::models::Actor::User).unwrap().key.to_lowercase()).collect();
    let mut tracker = run.clone();
    tracker.definition = Definition::ticket_at("todo").unwrap();
    tracker.pin = tracker.definition.pin();
    tracker.policy_pin = tracker.pin.clone();
    tracker.phase = Phase::WaitingApproval;
    // Prior runs and send-backs are what make a real board's state large.
    let reason = "Please keep the earlier review’s finding in mind and fix it before this goes on. ".repeat(40);
    for (n, id) in task_keys.iter().enumerate() {
        let mut item = if n < 60 { run.clone() } else { tracker.clone() };
        item.run = digest(&(&key, id));
        item.resource = Exec::item_resource(&key, id);
        if n < 60 {
            item.sent_back = (0..3).map(|i| farcooler_workflow::SendBack { id: format!("back-{i}"), revision: 1,
                run: item.run.clone(), from: "stage-3".into(), to: "stage-1".into(), reason: reason.clone() }).collect();
        }
        match n {
            // A failed run, to close or retry.
            1 => { item.phase = Phase::Failed; item.failure = Some("The local gate failed: scripts/copy-lint.py found two straight apostrophes in services/web/src/client/engine-view.ts.".into()); }
            // A worktree step the runner can't vouch for.
            2 => { item.phase = Phase::Uncertain; item.pending = Some("effect-2".into()); }
            // A cancellation whose receipt was lost.
            3 => { item.phase = Phase::Uncertain; item.pending = Some("effect-3".into()); item.stopping = Some("effect-3a".into());
                item.run = format!("cancel:{}", item.run); }
            _ => {}
        }
        state.items.insert(id.clone(), item);
    }
    // A landing whose push was lost, with two tickets joined to it.
    let mut group = run.clone();
    let landing = Definition { stages: vec![{ let mut push = run.definition.stages[0].clone(); push.id = "push".into();
        push.kind = StageKind::Push; push }], ..run.definition.clone() };
    group.definition = landing;
    group.pin = group.definition.pin();
    group.stage = 0;
    group.phase = Phase::Uncertain;
    group.pending = Some("effect-push".into());
    group.since_ms = 1;
    group.run = digest(&(&key, "group-7-a"));
    group.group_of = Some(farcooler_workflow::GroupOf { name: "main".into(), pin: group.pin.clone() });
    for id in &task_keys[4..6] {
        let member = state.items.get_mut(id).unwrap();
        member.phase = Phase::Joined;
        member.grouped_in = Some("group-7-a".into());
        group.members.push(farcooler_workflow::Member { item: id.clone(), candidate: "c".repeat(40) });
    }
    state.items.insert("group-7-a".into(), group);
    let whole = serde_json::to_vec(&state).unwrap().len();
    assert!(whole > farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES, "the whole state must not fit a frame: {whole}");
    board.runtime.store.workflow_state_for_tests(&state).unwrap();
    let browser = board.browser();

    let started = std::time::Instant::now();
    let (reply, _) = sent(&mut board.runtime, &browser, board.workspace, &Action::Get).await;
    let took = started.elapsed();
    // The framing itself refuses a reply over the protocol's cap.
    let frame = farcooler_protocol::framing::encode(&pb::WireEnvelope { protocol_version: farcooler_protocol::PROTOCOL_VERSION,
        message_id: farcooler_protocol::ids::new_id(), body: Some(pb::wire_envelope::Body::Response(reply.clone())) })
        .expect("the reply fits one frame").len();
    let raw = receipt(&reply);
    assert!(frame <= farcooler_protocol::MAX_CONTROL_ENVELOPE_BYTES, "{frame} bytes");
    assert!(raw.get("workflow_page").is_none(), "one reply, with nothing left to fetch");
    assert!(took < farcooler_client::deadlines::ORDINARY, "{took:?}");

    let mut pages = PlanPages::overview(crate::wire::id_bytes(board.workspace));
    let plan = loop {
        let answer = browser.handle(pb::Request { method: "plan.get".into(),
            payload: Some(request::Payload::PlanGet(pages.request())), ..Default::default() }).await;
        let Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::Plan(page)), .. })) = answer.outcome
        else { panic!("plan.get: {:?}", answer.outcome) };
        if let Some(plan) = pages.push(page).unwrap() { break plan; }
    };
    // The plan names no ticket here (none is in a theme or the queue), so the titles come from the board's
    // own paged list, as `Session::engine_page` reads them.
    assert!(engine_page::lacks_titles(&raw, &plan));
    let mut tasks = pb::TaskListRequest { workspace_id: Some(crate::wire::id_bytes(board.workspace)),
        page_after: Some(Default::default()), ..Default::default() };
    let mut listed = Vec::new();
    loop {
        let answer = browser.handle(pb::Request { method: "task.list".into(),
            payload: Some(request::Payload::TaskList(tasks.clone())), ..Default::default() }).await;
        let Some(response::Outcome::Result(pb::Result { value: Some(pb::result::Value::TaskList(page)), .. })) = answer.outcome
        else { panic!("task.list: {:?}", answer.outcome) };
        listed.extend(page.items);
        let Some(next) = page.next_page_after.filter(|next| !next.is_empty()) else { break };
        tasks.page_after = Some(next);
    }
    let plan = engine_page::with_titles(plan, &listed);
    let card = Uuid::now_v7();
    let draft = engine_page::draft("0123456789ab", &drafted(card, "0123456789ab"));
    assert!(draft.problem.is_none(), "{draft:?}");
    let page = engine_page::page(board.workspace, &raw, Some(&plan), vec![draft], None).unwrap();
    let page = serde_json::to_value(page).unwrap();
    let rows = shown(&page);
    assert_eq!(page["notice"], serde_json::Value::Null, "nothing was left out");
    assert_eq!(rows.len(), 340, "every ticket has its row");
    let titled = page["view"]["stations"].as_array().unwrap().iter().chain(page["view"]["landings"].as_array().unwrap())
        .flat_map(|of| of["tickets"].as_array().unwrap()).all(|t| t["title"].as_str().unwrap().contains("(part "));
    assert!(titled, "every row has its ticket's title");
    assert!(page["actions"]["group-7-a"].as_array().unwrap().iter().any(|o| o["kind"] == "settle_landed"), "{}", page["actions"]["group-7-a"]);
    assert!(page["actions"][&task_keys[3]].as_array().unwrap().iter().any(|o| o["kind"] == "resolve_cancellation"));
    assert!(page["actions"][&task_keys[2]].as_array().unwrap().iter().any(|o| o["kind"] == "resolve"));
    eprintln!("whole state {whole} bytes; Engine read {frame} bytes in {took:?}");
    if let Some(out) = std::env::var_os("FARCOOLER_ENGINE_FIXTURE") {
        std::fs::write(out, serde_json::to_vec(&serde_json::json!({ "workspace": board.workspace, "stateBytes": whole,
            "replyBytes": frame, "page": page })).unwrap()).unwrap();
    }
}

/// A draft's card as the daemon asks it, with a real question (`compose`'s own bytes, pinned in the client).
fn drafted(card: Uuid, id: &str) -> pb::TaskDetail {
    let question = include_str!("../../../client/src/testdata/workflow-change-question.txt").trim_end().to_owned();
    let at = question.rfind("answer `approve ").expect("the approving words") + "answer `".len();
    let approve = question[at..].split('`').next().unwrap().to_owned();
    pb::TaskDetail { task: None, blocks: Vec::new(), notes: vec![pb::TaskNote {
        id: crate::wire::id_bytes(Uuid::now_v7()), task_id: crate::wire::id_bytes(card), kind: pb::TaskNoteKind::Question as i32,
        actor: "manager".into(), body: question,
        extra_json: serde_json::json!({ "workflow_draft": { "id": id }, "options": [approve, "decline"] }).to_string(),
        ..Default::default() }] }
}

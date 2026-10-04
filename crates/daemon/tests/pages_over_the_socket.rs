//! Orchestrator pages (ov-269) as a client meets them: over a real socket,
//! through the real dispatch table and scope check.
//!
//! Each test is the one that goes red when the call it is about is removed:
//! the announce, the scope, the reference check, the drill.

#[path = "support/in_process.rs"]
mod in_process;

use std::time::Duration;

use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, event, request as payload, result};
use farcooler_store::models::{Actor, NoteKind, TaskStatus};
use farcooler_transport::{ClientError, request};
use in_process::*;
use uuid::Uuid;

fn id(uuid: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(uuid.as_bytes())
}

async fn call(link: &mut Link, method: &str, p: payload::Payload) -> Result<result::Value, ClientError> {
    let mut r = request(method);
    r.required_capabilities = vec![farcooler_protocol::capability::BOARD_PAGES.into()];
    r.payload = Some(p);
    Ok(link.call(r).await?.value.expect("a value"))
}

fn doc(title: &str, refs: &[&str]) -> String {
    let items: Vec<String> = refs.iter().map(|r| format!(r#"{{"text":"x","ref":{r}}}"#)).collect();
    let list = if items.is_empty() { String::new() } else { format!(r#",{{"type":"list","items":[{}]}}"#, items.join(",")) };
    format!(r#"{{"v":1,"title":"{title}","summary":"s","blocks":[{{"type":"heading","text":"Lanes"}}{list}]}}"#)
}

fn set(workspace: Uuid, slot: &str, doc_json: String) -> pb::PageSet {
    pb::PageSet { workspace_id: id(workspace), slot: slot.into(), doc_json, actor: "manager".into(), ..Default::default() }
}

async fn put(link: &mut Link, p: pb::PageSet) -> Result<pb::PageSetResult, ClientError> {
    match call(link, "page.set", payload::Payload::PageSet(p)).await? {
        result::Value::PageSetResult(r) => Ok(r),
        other => panic!("wrong result: {other:?}"),
    }
}

async fn list(link: &mut Link, workspace: Uuid, with_docs: bool) -> Vec<pb::BoardPage> {
    let p = payload::Payload::PageList(pb::PageListRequest { workspace_id: id(workspace), with_docs });
    match call(link, "page.list", p).await.expect("page.list") {
        result::Value::BoardPageList(l) => l.pages,
        other => panic!("wrong result: {other:?}"),
    }
}

async fn refusal(r: Result<impl std::fmt::Debug, ClientError>) -> (i32, String, String) {
    match r {
        Err(ClientError::Daemon { code, what, message, .. }) => (code, what, message),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The `pages_changed` and `task_changed` events on `listener` within `window`.
async fn events(listener: &mut Link, window: Duration) -> (Vec<pb::PagesChanged>, usize) {
    let (mut pages, mut tasks) = (vec![], 0);
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Ok(e)) = tokio::time::timeout_at(deadline, listener.next_event()).await {
        match e.payload {
            Some(event::Payload::PagesChanged(p)) => pages.push(p),
            Some(event::Payload::TaskChanged(_)) => tasks += 1,
            _ => {}
        }
    }
    (pages, tasks)
}

/// A page, written, read, listed, changed and removed; a change announces
/// `pages_changed` and never `task_changed`, and identical bytes announce
/// nothing.
#[tokio::test]
async fn a_page_round_trips_and_announces_only_when_it_changes() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let mut listener = connect(&h).await;

    let first = put(&mut a, set(repo.workspace, "train", doc("Train", &[]))).await.expect("page.set");
    assert!(first.changed);
    let page = first.page.unwrap();
    assert_eq!((page.slot.as_str(), page.revision, page.actor.as_str()), ("train", 1, "manager"));
    let stored: serde_json::Value = serde_json::from_str(&page.doc_json).unwrap();
    assert_eq!(stored["title"], "Train", "the document is the normalized one");

    let same = put(&mut a, set(repo.workspace, "train", doc("Train", &[]))).await.unwrap();
    assert!(!same.changed, "identical bytes are no change");
    assert_eq!(same.page.unwrap().revision, 1);

    let second = put(&mut a, set(repo.workspace, "train", doc("Train, later", &[]))).await.unwrap();
    assert_eq!(second.page.unwrap().revision, 2);

    let get = payload::Payload::PageGet(pb::PageGetRequest { workspace_id: id(repo.workspace), slot: "train".into() });
    let result::Value::BoardPage(read) = call(&mut connect(&h).await, "page.get", get).await.unwrap() else { panic!() };
    assert_eq!(read.title, "Train, later");

    let rm = payload::Payload::PageRemove(pb::PageRemove { workspace_id: id(repo.workspace), slot: "train".into(), actor: "manager".into() });
    let result::Value::BoardPage(gone) = call(&mut a, "page.remove", rm).await.expect("page.remove") else { panic!() };
    assert_eq!(gone.slot, "train");
    assert!(list(&mut a, repo.workspace, false).await.is_empty());

    let (pages, tasks) = events(&mut listener, Duration::from_millis(600)).await;
    assert_eq!(pages.len(), 3, "two changes and a removal; the identical write said nothing");
    assert_eq!(pages.iter().map(|p| (p.revision, p.removed)).collect::<Vec<_>>(), [(1, false), (2, false), (2, true)]);
    assert!(pages.iter().all(|p| p.workspace_id == id(repo.workspace) && p.slot == "train" && p.actor == "manager"));
    assert_eq!(tasks, 0, "no page write moves a task");
}

/// A list without documents has none, and a list with them has every one.
#[tokio::test]
async fn a_list_carries_documents_only_when_asked() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    put(&mut a, set(repo.workspace, "train", doc("Train", &[]))).await.unwrap();
    put(&mut a, set(repo.workspace, "spend", doc("Spend", &[]))).await.unwrap();
    let bare = list(&mut a, repo.workspace, false).await;
    assert_eq!(bare.iter().map(|p| p.slot.as_str()).collect::<Vec<_>>(), ["train", "spend"]);
    assert!(bare.iter().all(|p| p.doc_json.is_empty() && !p.title.is_empty()));
    assert!(list(&mut a, repo.workspace, true).await.iter().all(|p| !p.doc_json.is_empty()));
}

/// A refusal names the JSON path and the limit, as a sentence the CLI shows.
#[tokio::test]
async fn a_bad_document_is_refused_with_its_path() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    let bad = r#"{"v":1,"title":"T","blocks":[{"type":"diagram","text":"x"}]}"#;
    let (code, what, message) = refusal(put(&mut a, set(repo.workspace, "train", bad.into())).await).await;
    assert_eq!((code, what.as_str()), (ErrorCode::InvalidArgument as i32, "page"));
    assert_eq!(
        message,
        "blocks[0].type: there's no block called diagram. The blocks are heading, text, stats, progress, table, list, timeline, steps and links."
    );
    let (_, _, message) = refusal(put(&mut a, set(repo.workspace, "Bad Slot", doc("T", &[]))).await).await;
    assert!(message.contains("lowercase letters, digits and hyphens"), "{message}");
    let (_, _, message) = refusal(put(&mut a, set(repo.workspace, "train", "{ oops".into())).await).await;
    assert_eq!(message, "That isn't valid JSON (line 1, column 3).");
    assert!(list(&mut a, repo.workspace, false).await.is_empty(), "nothing was written");
}

/// A card, a lane and a theme the page names must be on this board, so a typo
/// fails when it's made.
#[tokio::test]
async fn references_are_checked_against_the_board() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let task = store.create_task(repo.workspace, "Mac: a jump", Actor::User).unwrap();
    let mut a = connect(&h).await;

    let missing_card = doc("T", &[r#"{"task":"zz-99"}"#]);
    let (code, _, message) = refusal(put(&mut a, set(repo.workspace, "train", missing_card)).await).await;
    assert_eq!(code, ErrorCode::InvalidArgument as i32);
    assert_eq!(message, "blocks[1].items[0]: there's no card zz-99 on this board.");

    let missing_lane = doc("T", &[r#"{"lane":"mac-ux"}"#]);
    let (_, _, message) = refusal(put(&mut a, set(repo.workspace, "train", missing_lane.clone())).await).await;
    assert_eq!(message, "blocks[1].items[0]: there's no lane mac-ux on this board.");
    let missing_theme = doc("T", &[r#"{"theme":"Visual language"}"#]);
    let (_, _, message) = refusal(put(&mut a, set(repo.workspace, "train", missing_theme.clone())).await).await;
    assert_eq!(message, "blocks[1].items[0]: there's no theme Visual language on this board.");

    store
        .create_lane(repo.workspace, &farcooler_store::plan::NewLane { name: "Mac-UX".into(), ..Default::default() }, &[], None, Actor::Manager)
        .unwrap();
    store
        .create_theme(
            repo.workspace,
            &farcooler_store::plan::NewTheme { name: "Visual Language".into(), outcome: "One app.".into() },
            &[task.id],
            Actor::Manager,
        )
        .unwrap();
    let ok = doc(
        "T",
        &[&format!(r#"{{"task":"{}"}}"#, task.key), r#"{"lane":"mac-ux"}"#, r#"{"theme":"visual language"}"#, r#"{"page":"not-yet"}"#, r#"{"worktree":"nowhere"}"#],
    );
    assert!(put(&mut a, set(repo.workspace, "train", ok)).await.unwrap().changed, "names match without regard to case, and a page, worktree or link isn't checked");
    let upper = doc("T2", &[&format!(r#"{{"ask":"{}"}}"#, task.key.to_uppercase())]);
    assert!(put(&mut a, set(repo.workspace, "train", upper)).await.is_ok(), "a card key matches without regard to case");
}

/// A theme anchors a page by its id, as text, and only a live theme on this
/// board will do.
#[tokio::test]
async fn a_page_anchors_to_a_live_theme_and_the_anchor_can_be_cleared() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let theme = store
        .create_theme(repo.workspace, &farcooler_store::plan::NewTheme { name: "T".into(), outcome: "o".into() }, &[], Actor::Manager)
        .unwrap();
    let mut a = connect(&h).await;

    let mut p = set(repo.workspace, "risks", doc("Risks", &[]));
    p.anchor_theme_id = Some(id(Uuid::now_v7()));
    let (_, _, message) = refusal(put(&mut a, p.clone()).await).await;
    assert_eq!(message, "There's no theme with that id on this board.");

    p.anchor_theme_id = Some(id(theme.id));
    let anchored = put(&mut a, p).await.unwrap().page.unwrap();
    assert_eq!((anchored.anchor_kind.as_str(), anchored.anchor.as_str()), ("theme", theme.id.to_string().as_str()));

    let kept = put(&mut a, set(repo.workspace, "risks", doc("Risks, later", &[]))).await.unwrap().page.unwrap();
    assert_eq!(kept.anchor, anchored.anchor, "no anchor in the request leaves it");

    let mut clear = set(repo.workspace, "risks", doc("Risks, later", &[]));
    clear.anchor_theme_id = Some(bytes::Bytes::new());
    let cleared = put(&mut a, clear).await.unwrap();
    assert!(cleared.changed);
    assert_eq!(cleared.page.unwrap().anchor_kind, "");
}

/// `if_revision` refuses a write against a page that changed since it was read.
#[tokio::test]
async fn a_write_against_a_stale_revision_is_a_conflict() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    put(&mut a, set(repo.workspace, "train", doc("Train", &[]))).await.unwrap();
    put(&mut a, set(repo.workspace, "train", doc("Train, later", &[]))).await.unwrap();
    let mut stale = set(repo.workspace, "train", doc("Train, stale", &[]));
    stale.if_revision = Some(1);
    let (code, _, _) = refusal(put(&mut a, stale).await).await;
    assert_eq!(code, ErrorCode::ResourceConflict as i32);
    let mut fresh = set(repo.workspace, "train", doc("Train, fresh", &[]));
    fresh.if_revision = Some(2);
    assert!(put(&mut a, fresh).await.unwrap().changed);
}

/// Reading pages is a read; writing and removing them is control.
#[tokio::test]
async fn a_read_client_reads_pages_and_cannot_write_them() {
    let h = start(Scope::Read).await;
    let repo = a_repository(&h);
    let page = farcooler_core::page_doc::parse(&doc("Train", &[]), &farcooler_core::page_doc::Caps::default()).unwrap();
    h.service
        .store
        .set_page(
            repo.workspace,
            &farcooler_store::pages::PageWrite {
                slot: "train",
                page: &page,
                anchor: farcooler_store::pages::Anchor::Keep,
                ordinal: None,
                if_revision: None,
            },
            Actor::Manager,
        )
        .unwrap();
    let mut link = connect(&h).await;
    assert_eq!(list(&mut link, repo.workspace, true).await.len(), 1);
    let stats = payload::Payload::PageStats(pb::PageStatsRequest { workspace_id: id(repo.workspace), since_ms: 0 });
    let result::Value::PageStatsList(stats) = call(&mut link, "page.stats", stats).await.expect("page.stats") else { panic!() };
    assert_eq!((stats.slots[0].slot.as_str(), stats.slots[0].sets), ("train", 1));
    assert!(stats.slots[0].shape.iter().any(|s| s.kind == "heading" && s.count == 1));

    let denied = |r: Result<result::Value, ClientError>| match r {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32),
        other => panic!("expected a scope denial, got {other:?}"),
    };
    denied(call(&mut link, "page.set", payload::Payload::PageSet(set(repo.workspace, "train", doc("x", &[])))).await);
    let rm = payload::Payload::PageRemove(pb::PageRemove { workspace_id: id(repo.workspace), slot: "train".into(), actor: String::new() });
    denied(call(&mut link, "page.remove", rm).await);
}

/// A runner that doesn't advertise `board_pages` isn't asked: the capability is
/// in the hello, and a request that names one a runner lacks is refused as
/// "this runner can't", the code an app words as an update.
#[tokio::test]
async fn the_runner_advertises_pages_and_refuses_a_capability_it_lacks() {
    let h = start(Scope::Control).await;
    let mut a = connect(&h).await;
    assert!(a.server_hello().capabilities.iter().any(|c| c == farcooler_protocol::capability::BOARD_PAGES));
    let repo = a_repository(&h);
    let mut r = request("page.set");
    r.required_capabilities = vec!["board_pages_from_the_future".into()];
    r.payload = Some(payload::Payload::PageSet(set(repo.workspace, "train", doc("T", &[]))));
    match a.call(r).await {
        Err(ClientError::Daemon { code, .. }) => assert_eq!(code, ErrorCode::CapabilityUnsupported as i32),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// With the plan layer gone, pages still work: a lane or a theme reference has
/// nothing to be checked against and is accepted, and an anchor is refused in
/// words rather than stored pointing at nothing.
#[tokio::test]
async fn pages_work_with_the_plan_layer_removed() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let mut a = connect(&h).await;
    farcooler_store::testing::drop_plan_layer(&h.service.store);
    let with_refs = doc("T", &[r#"{"lane":"anything"}"#, r#"{"theme":"anything"}"#]);
    assert!(put(&mut a, set(repo.workspace, "train", with_refs)).await.unwrap().changed);
    let mut p = set(repo.workspace, "train", doc("T2", &[]));
    p.anchor_theme_id = Some(id(Uuid::now_v7()));
    let (_, _, message) = refusal(put(&mut a, p).await).await;
    assert_eq!(message, "This runner has no plan, so a page can't be drawn in a theme. Publish it without one.");
    let (_, _, message) = refusal(put(&mut a, set(repo.workspace, "x", doc("T", &[r#"{"task":"zz-1"}"#]))).await).await;
    assert!(message.contains("there's no card zz-1"), "cards are still checked: {message}");
}

/// Everything a board client reads, as bytes.
async fn board_bytes(link: &mut Link, repo: &Repo) -> Vec<String> {
    let mut out = vec![];
    let mut list = request("task.list");
    list.payload = Some(payload::Payload::TaskList(pb::TaskListRequest {
        repository_id: id(repo.id),
        workspace_id: Some(id(repo.workspace)),
        ..Default::default()
    }));
    let got = link.call(list).await.expect("task.list").value.expect("a value");
    let result::Value::TaskList(mut tasks) = got else { panic!() };
    // The read marks carry the runner's first-look clock, which is written once
    // and is not the board's.
    tasks.reads = None;
    out.push(format!("{tasks:#?}"));
    for t in &tasks.items {
        let mut get = request("task.get");
        get.payload = Some(payload::Payload::TaskGet(pb::TaskGetRequest { task_id: t.id.clone(), ..Default::default() }));
        let value = link.call(get).await.expect("task.get").value.expect("a value");
        let result::Value::TaskDetail(detail) = value else { panic!() };
        out.push(format!("{detail:#?}"));
    }
    let value = link.call(request("needs_you.list")).await.expect("needs_you.list").value.expect("a value");
    let result::Value::NeedsYouList(needs) = value else { panic!() };
    out.push(format!("{needs:#?}"));
    let mut rep = request("report.get");
    rep.payload = Some(payload::Payload::ReportRequest(pb::ReportRequest {
        since: 0,
        until: now_millis() + 3_600_000,
        ..Default::default()
    }));
    let value = link.call(rep).await.expect("report.get").value.expect("a value");
    let result::Value::Report(report) = value else { panic!() };
    // What the report counts, not the clock it was cut at: its durations and
    // `generated_at` move with the runner's time between the two reads.
    let json: serde_json::Value = serde_json::from_str(&report.report_json).expect("the runner's JSON");
    let waits: Vec<_> = json["notable"]["longest_waits"].as_array().unwrap().iter().map(|w| w["key"].clone()).collect();
    let t = &json["totals"];
    out.push(format!(
        "{} {} {} {} {} {waits:?}",
        t["created"], t["completed"], t["canceled"], t["decisions"]["asked"], t["acceptance"]
    ));
    let mut plan = request("plan.get");
    plan.payload = Some(payload::Payload::PlanGet(pb::PlanGetRequest { workspace_id: id(repo.workspace), include_closed: true }));
    let value = link.call(plan).await.expect("plan.get").value.expect("a value");
    let result::Value::Plan(mut plan) = value else { panic!() };
    // The runner's clock when it was read, which moves between two reads.
    plan.now_ms = 0;
    out.push(format!("{plan:#?}"));
    out
}

/// The removal drill (ov-269, section 4.3), over the wire: with the pages'
/// tables dropped, `task.list`, `task.get`, `needs_you.list`, `report.get` and
/// `plan.get` answer the same bytes, and the board's and the plan's writes
/// still work.
#[tokio::test]
async fn the_boards_and_the_plans_wire_reads_are_the_same_bytes_without_pages() {
    let h = start(Scope::Control).await;
    let repo = a_repository(&h);
    let store = &h.service.store;
    let a = store.create_task(repo.workspace, "Mac: first", Actor::User).unwrap();
    let b = store.create_task(repo.workspace, "Mac: second", Actor::User).unwrap();
    store.set_task_status(a.id, TaskStatus::InProgress, Actor::Manager).unwrap();
    store.add_note(a.id, NoteKind::Question, Actor::Manager, "Which one?", serde_json::json!({})).unwrap();
    store.set_task_status(a.id, TaskStatus::NeedsDecision, Actor::Manager).unwrap();
    store.add_note(b.id, NoteKind::Progress, Actor::Manager, "Dispatched in the mac-ux lane.", serde_json::json!({})).unwrap();
    let theme = store
        .create_theme(repo.workspace, &farcooler_store::plan::NewTheme { name: "T".into(), outcome: "o".into() }, &[a.id, b.id], Actor::Manager)
        .unwrap();

    let mut link = connect(&h).await;
    let mut anchored = set(repo.workspace, "risks", doc("Risks", &[&format!(r#"{{"task":"{}"}}"#, a.key)]));
    anchored.anchor_theme_id = Some(id(theme.id));
    put(&mut link, anchored).await.unwrap();
    put(&mut link, set(repo.workspace, "train", doc("Train", &[]))).await.unwrap();

    let before = board_bytes(&mut link, &repo).await;
    farcooler_store::testing::drop_pages(store);
    let after = board_bytes(&mut link, &repo).await;
    assert_eq!(before.len(), after.len());
    for (i, (x, y)) in before.iter().zip(&after).enumerate() {
        assert_eq!(x, y, "board read {i} changed when the pages went");
    }

    let mut create = request("task.create");
    create.target_resource_id = Some(id(repo.id));
    create.payload = Some(payload::Payload::TaskCreate(pb::TaskCreate {
        repository_id: id(repo.id),
        workspace_id: Some(id(repo.workspace)),
        title: "CLI: after".into(),
        ..Default::default()
    }));
    link.call(create).await.expect("task.create still works without pages");
    let mut theme_write = request("board_theme.create");
    theme_write.payload = Some(payload::Payload::BoardThemeCreate(pb::BoardThemeCreate {
        workspace_id: id(repo.workspace),
        name: "After".into(),
        outcome: "o".into(),
        task_ids: vec![],
        actor: "manager".into(),
    }));
    link.call(theme_write).await.expect("the plan layer still writes without pages");
}

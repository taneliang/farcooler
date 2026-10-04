//! A phone's pages, the whole way: `dispatch` with the JSON an app passes, a
//! `Session`, and the daemon's own `RpcFactory` at the scope a phone is enrolled
//! with, reading pages the store wrote.

use farcooler_core::page_doc::{self, Caps};
use farcooler_protocol::method::Method;
use farcooler_protocol::v1::Scope;
use farcooler_store::models::Actor;
use farcooler_store::pages::{Anchor, PageWrite};
use serde_json::json;

use super::phone_path_tests::a_runner;
use super::{dispatch, route::route};
use crate::session::{Session, SessionError};

fn a_page(runner: &super::phone_path_tests::Runner, slot: &str, title: &str) -> uuid::Uuid {
    let store = &runner.service.store;
    let repo = store.list_all_worktrees().unwrap().remove(0).repository_id;
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let doc = format!(r#"{{"v":1,"title":"{title}","blocks":[{{"type":"heading","text":"Lanes"}}]}}"#);
    let page = page_doc::parse(&doc, &Caps::default()).unwrap();
    store
        .set_page(main, &PageWrite { slot, page: &page, anchor: Anchor::Keep, ordinal: None, if_revision: None }, Actor::Manager)
        .unwrap();
    main
}

/// `page.list` and `page.get` answer over the real daemon, in the shape the
/// apps decode, with the document as JSON. Goes red when an arm is missing or
/// `page_json` drops a field.
#[tokio::test]
async fn a_phone_reads_pages() {
    let runner = a_runner(Scope::Read).await;
    let main = a_page(&runner, "train", "Train");
    a_page(&runner, "spend", "Spend");
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let list = dispatch(&session, "page.list", &json!({ "workspace": main.to_string() })).await.expect("page.list");
    assert_eq!(list["pages"].as_array().unwrap().len(), 2, "{list}");
    assert_eq!(list["pages"][0]["slot"], "train");
    assert_eq!(list["pages"][0]["doc"]["blocks"][0]["text"], "Lanes");
    assert_eq!(list["pages"][1]["title"], "Spend");

    let page = dispatch(&session, "page.get", &json!({ "workspace": main.to_string(), "slot": "spend" })).await.expect("page.get");
    assert_eq!((page["slot"].as_str(), page["revision"].as_u64()), (Some("spend"), Some(1)), "{page}");
    assert_eq!(page["doc"]["v"], 1);
}

/// A page read names its board and, for one page, its slot; neither is guessed.
#[tokio::test]
async fn a_page_read_names_its_board_and_slot() {
    let runner = a_runner(Scope::Read).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    assert!(matches!(dispatch(&session, "page.list", &json!({})).await, Err(SessionError::Protocol(_))));
    let board = uuid::Uuid::nil().to_string();
    assert!(matches!(dispatch(&session, "page.get", &json!({ "workspace": board })).await, Err(SessionError::Protocol(_))));
}

/// The one writer is the orchestrator: every page write has no phone route and
/// no arm, and the two reads have both.
#[tokio::test]
async fn a_phone_reads_pages_and_never_writes_them() {
    let runner = a_runner(Scope::Control).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    for method in [Method::PageList, Method::PageGet] {
        assert_eq!(route(method), Some(method.name()));
    }
    for method in [Method::PageSet, Method::PageRemove, Method::PageStats] {
        assert_eq!(route(method), None, "{} has a phone route", method.name());
        let answer = dispatch(&session, method.name(), &json!({})).await;
        assert!(
            matches!(&answer, Err(SessionError::Protocol(m)) if m == &format!("unknown method: {}", method.name())),
            "`dispatch` has an arm for {}: {answer:?}",
            method.name()
        );
    }
}

/// A page written on the runner reaches a phone as a `pages` notice naming the
/// board and the slot.
#[test]
fn a_page_notice_names_its_board_and_slot() {
    let board = uuid::Uuid::from_u128(7);
    let line = super::event_line(&crate::session::FleetEvent::Pages { workspace: board, slot: "train".into(), removed: false });
    assert_eq!(line, format!(r#"{{"event":"pages","removed":false,"slot":"train","workspace":"{board}"}}"#));
}

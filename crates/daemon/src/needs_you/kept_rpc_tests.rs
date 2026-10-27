//! A member's Keep reaches the lead's `needs_you.list` as an FYI, through
//! `Rpc`, from the rulings the store holds (ov-756).

use std::sync::Arc;

use farcooler_protocol::v1::{self as pb, NeedsYouKind, Request, Scope, request, response, result};
use farcooler_store::models::Actor;
use farcooler_store::rulings::NewRuling;
use farcooler_transport::Peer;
use uuid::Uuid;

use crate::rpc::RpcFactory;
use crate::service::Service;
use crate::wire::id_bytes;

const BOB: Option<&str> = Some("phone-b");
const ALICE: Option<&str> = Some("phone-a");
const OWNER: Option<&str> = None;

struct Runner {
    _dir: crate::test_support::ScratchDir,
    svc: Arc<Service>,
    workspace: Uuid,
}

async fn runner() -> Runner {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    svc.store.set_member("bob", "Bob", &["phone-b".to_string()]).unwrap();
    svc.store.set_member("alice", "Alice", &["phone-a".to_string()]).unwrap();
    Runner { _dir: dir, svc, workspace }
}

fn rpc(r: &Runner, device: Option<&str>) -> RpcFactory {
    let peer = Peer { client_id: device.map(str::to_string), scope: Scope::Control };
    RpcFactory::new(r.svc.clone(), crate::watch::Watcher::new(r.svc.clone()), Arc::new(tokio::sync::Notify::new()), peer)
}

async fn call(rpc: &RpcFactory, method: &str, payload: Option<request::Payload>) -> Option<result::Value> {
    call_accepting(rpc, method, payload, &[]).await
}

async fn call_accepting(rpc: &RpcFactory, method: &str, payload: Option<request::Payload>, accepts: &[&str]) -> Option<result::Value> {
    use farcooler_transport::Handler as _;
    let accepts = accepts.iter().map(|a| a.to_string()).collect();
    match rpc.handle(Request { method: method.into(), payload, accepts, ..Default::default() }).await.outcome {
        Some(response::Outcome::Result(r)) => r.value,
        other => panic!("{method}: {other:?}"),
    }
}

fn open_ruling(r: &Runner, decision: &str) -> Uuid {
    let new = NewRuling { decision: decision.into(), why: "Why.".into(), reversal: "Undo.".into(), theme_id: None };
    r.svc.store.add_ruling(r.workspace, &new, &[], Actor::Manager).unwrap().id
}

async fn keep(r: &Runner, device: Option<&str>, ruling: Uuid) {
    let set = pb::RulingSet { ruling_id: id_bytes(ruling), state: pb::BoardRulingState::Confirmed as i32, actor: "user".into(), ..Default::default() };
    call(&rpc(r, device), "ruling.set", Some(request::Payload::RulingSet(set))).await;
}

async fn keep_all(r: &Runner, device: Option<&str>) {
    let all = pb::RulingKeepAll { workspace_id: id_bytes(r.workspace), actor: "user".into() };
    call(&rpc(r, device), "ruling.keep_all", Some(request::Payload::RulingKeepAll(all))).await;
}

/// What `device`'s Needs You says as FYIs, to a client that places them.
async fn fyis(r: &Runner, device: Option<&str>) -> Vec<String> {
    fyis_for(r, device, &[farcooler_protocol::accepts::NEEDS_YOU_FYI]).await
}

async fn fyis_for(r: &Runner, device: Option<&str>, accepts: &[&str]) -> Vec<String> {
    match call_accepting(&rpc(r, device), "needs_you.list", None, accepts).await {
        Some(result::Value::NeedsYouList(l)) => l.items.into_iter().filter(|i| i.kind() == NeedsYouKind::Fyi).map(|i| i.question).collect(),
        other => panic!("{other:?}"),
    }
}

/// Bob's Keep and Keep All tell the owner, who leads; Bob's own list and
/// Alice's say nothing; the owner's own Keep tells no one; a reversal takes
/// the line away; and once Alice leads, she is told and the owner is not.
#[tokio::test]
async fn a_members_keep_tells_the_lead_and_the_leads_own_tells_no_one() {
    let r = runner().await;
    let first = open_ruling(&r, "Round half to even");
    assert!(fyis(&r, OWNER).await.is_empty(), "nothing kept yet");

    keep(&r, BOB, first).await;
    let told = fyis(&r, OWNER).await;
    assert_eq!(told, vec!["Bob kept R-1: Round half to even".to_string()]);
    assert!(fyis(&r, BOB).await.is_empty(), "Bob's own Keep tells Bob nothing");
    assert!(fyis(&r, ALICE).await.is_empty(), "Alice doesn't lead");

    open_ruling(&r, "Page nobody at night");
    open_ruling(&r, "Show currency on every line");
    keep_all(&r, BOB).await;
    let told = fyis(&r, OWNER).await;
    assert!(told.contains(&"Bob kept 2 rulings: R-3, R-2".to_string()) || told.iter().any(|t| t.starts_with("Bob kept 2 rulings")), "{told:?}");
    assert_eq!(told.len(), 2, "one line for the Keep, one for the Keep All");

    let own = open_ruling(&r, "Owner's own call");
    keep(&r, OWNER, own).await;
    assert_eq!(fyis(&r, OWNER).await.len(), 2, "the owner's Keep adds nothing");
    assert!(fyis(&r, ALICE).await.is_empty());

    r.svc.store.set_ruling(first, farcooler_store::rulings::RulingState::Reversed, None, Actor::User).unwrap();
    assert_eq!(fyis(&r, OWNER).await.len(), 1, "a reversed ruling isn't kept any more");

    r.svc.store.set_workspace_lead(r.workspace, Some("alice")).unwrap();
    assert!(fyis(&r, OWNER).await.is_empty(), "the owner no longer leads");
    assert_eq!(fyis(&r, ALICE).await.len(), 1, "Alice leads now");
    keep(&r, ALICE, open_ruling(&r, "Alice's own call")).await;
    assert_eq!(fyis(&r, ALICE).await.len(), 1, "the lead's own Keep adds nothing");
}

/// A client that doesn't name `needs_you_fyi` is sent no FYI, though the same
/// owner's list from a client that does has one: a shipped app can't hide a
/// kind it has never heard of.
#[tokio::test]
async fn a_client_that_does_not_ask_for_fyis_gets_none() {
    let r = runner().await;
    let ruling = open_ruling(&r, "Round half to even");
    keep(&r, BOB, ruling).await;
    assert_eq!(fyis(&r, OWNER).await.len(), 1, "a client that accepts them");
    assert!(fyis_for(&r, OWNER, &[]).await.is_empty(), "an older client");
    assert!(fyis_for(&r, OWNER, &["something_else"]).await.is_empty(), "a client that accepts another thing");
}

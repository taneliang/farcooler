//! A real `farcoolerd` whose `gh` answers `search prs`, and the Needs You list
//! it serves (ov-315): the open pull requests that ask for this login's review
//! appear once a board lands through pull requests, and not before; and a `gh`
//! that cannot answer leaves the kinds absent, with no error.
//!
//! Alone in its binary with the support module `gh_pr`, as the PR-stage tests
//! are, so no other test's shim answers these calls.

use std::time::{Duration, Instant};

use farcooler_protocol::capability::{LANDING, NEEDS_YOU, WAKE_ON_ANSWER, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, NeedsYouKind, request, result};
use farcooler_transport::request as request_for;

mod common;
#[path = "support/gh_pr.rs"]
mod gh_pr;
use gh_pr::*;

/// The recorded search (`test/fixtures/pr-review/search.json`): two pull
/// requests and a draft.
fn serve_search(dir: &std::path::Path) {
    std::fs::copy(fixtures().join("../pr-review/search.json"), dir.join("search-prs.json")).unwrap();
}

async fn needs_you(client: &mut SocketClient) -> Vec<pb::NeedsYouItem> {
    let result::Value::NeedsYouList(list) = call(client, request_for("needs_you.list"), NEEDS_YOU).await else {
        panic!("wrong result")
    };
    list.items
}

fn requests(items: &[pb::NeedsYouItem]) -> Vec<&pb::NeedsYouItem> {
    items.iter().filter(|i| i.kind() == NeedsYouKind::PrReview).collect()
}

/// The board chooses pull requests: `workspace.set_settings`.
async fn choose_pull_requests(client: &mut SocketClient, workspace: &pb::Workspace) {
    let mut set = request_for("workspace.set_settings");
    set.target_resource_id = Some(workspace.id.clone());
    set.payload = Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings {
        landing: Some(pb::LandingMode::PullRequests as i32),
        expected_version: Some(workspace.resource_version),
        ..Default::default()
    }));
    set.required_capabilities = vec![LANDING.into(), WAKE_ON_ANSWER.into()];
    let result::Value::Workspace(_) = call(client, set, WORKSTREAMS).await else { panic!("wrong result") };
}

async fn calls_of(dir: &std::path::Path, what: &str, at_least: usize) {
    let deadline = Instant::now() + BUDGET;
    while calls(dir).iter().filter(|c| *c == what).count() < at_least {
        assert!(Instant::now() < deadline, "the runner never ran `{what}`: {:?}", calls(dir));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn review_requests_appear_once_a_board_lands_through_pull_requests_and_not_before() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    serve_search(dir.path());

    // A runner that has not chosen pull requests asks GitHub nothing.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(requests(&needs_you(&mut client).await).is_empty());
    assert!(!calls(dir.path()).iter().any(|c| c == "search prs"), "{:?}", calls(dir.path()));

    choose_pull_requests(&mut client, &workspace).await;
    let deadline = Instant::now() + BUDGET;
    let items = loop {
        let items = needs_you(&mut client).await;
        if !requests(&items).is_empty() {
            break items;
        }
        assert!(Instant::now() < deadline, "no request ever appeared; gh was asked {:?}", calls(dir.path()));
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let found = requests(&items);
    assert_eq!(found.len(), 2, "the draft is not asked for yet: {found:#?}");
    // The oldest first: PR 213 opened on the 4th, PR 7 on the 5th.
    assert_eq!(found[0].question, "Add refund endpoint");
    assert_eq!(found[0].detail.as_deref(), Some("acme/billing #213 \u{b7} bob"));
    assert_eq!(found[0].link.as_deref(), Some("https://github.com/acme/billing/pull/213"));
    assert_eq!(found[1].link.as_deref(), Some("https://github.com/acme/ledger/pull/7"));
    assert!(found.iter().all(|i| i.id.starts_with("pr-review:") && !i.id.contains("github")));
    assert!(found[0].rank < found[1].rank);
}

/// `gh` logged out: the runner asked, and the list has no pull-request kinds
/// and no error. Not an empty answer from GitHub, so nothing claims there is
/// no review waiting.
#[tokio::test]
async fn a_gh_that_is_logged_out_leaves_the_kinds_absent_and_the_list_readable() {
    let (dir, _daemon, mut client, workspace) = a_runner().await;
    serve_search(dir.path());
    std::fs::write(dir.path().join("offline"), "").unwrap();

    choose_pull_requests(&mut client, &workspace).await;
    calls_of(dir.path(), "search prs", 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let items = needs_you(&mut client).await;
    assert!(
        items.iter().all(|i| !matches!(i.kind(), NeedsYouKind::PrReview | NeedsYouKind::PrThread)),
        "{items:#?}"
    );
}

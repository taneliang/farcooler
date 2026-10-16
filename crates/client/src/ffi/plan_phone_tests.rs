//! A phone's Plan view, the whole way (ov-274): `dispatch` with the JSON an app
//! passes, a `Session`, and the daemon's own `RpcFactory` at the scope a phone
//! is enrolled with, reading a plan the store wrote.

use farcooler_protocol::method::Method;
use farcooler_protocol::v1::Scope;
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneCard, NewLane, NewTheme, ThemeUpdate};
use serde_json::json;

use super::phone_path_tests::a_runner;
use super::{dispatch, route::route};
use crate::session::{Session, SessionError};

/// A board with one theme holding one card, and a queued lane working it.
fn a_plan(runner: &super::phone_path_tests::Runner) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
    let store = &runner.service.store;
    let repo = store.list_all_worktrees().unwrap().remove(0).repository_id;
    let main = store.ensure_main_workspace(repo).unwrap().id;
    let task = store.create_task(main, "Phones: Plan view", Actor::User).unwrap();
    let theme = store
        .create_theme(main, &NewTheme { name: "Phone parity".into(), outcome: "A phone does what a Mac does.".into() }, &[task.id], Actor::Manager)
        .unwrap();
    store
        .update_theme(
            theme.id,
            &ThemeUpdate { owner_ask: Some("Read the copy.".into()), story: Some("Built.".into()), ..Default::default() },
            Actor::Manager,
        )
        .unwrap();
    let lane = store
        .create_lane(
            main,
            &NewLane { name: "plan-phones".into(), reason: "Unblocked".into(), ..Default::default() },
            &[LaneCard { task_id: task.id, slice: String::new() }],
            None,
            Actor::Manager,
        )
        .unwrap();
    store.set_plan(main, &[lane.id], Actor::Manager).unwrap();
    (main, theme.id, lane.id)
}

/// `plan.get` answers, over the real daemon, in the shape the apps decode: the
/// theme with its ask and counts, the queued lane first in `order`, and the
/// card's key. Goes red when the arm is missing or `plan_json` drops a field.
#[tokio::test]
async fn a_phone_reads_the_plan() {
    let runner = a_runner(Scope::Read).await;
    let (main, theme, lane) = a_plan(&runner);
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    let plan = dispatch(&session, "plan.get", &json!({ "workspace": main.to_string() })).await.expect("plan.get");
    assert_eq!(plan["themes"][0]["id"], theme.to_string(), "{plan}");
    assert_eq!(plan["themes"][0]["owner_ask"], "Read the copy.");
    assert_eq!(plan["themes"][0]["counts"]["backlog"], 1);
    assert_eq!(plan["themes"][0]["cards"][0]["key"], plan["cards"][0]["key"]);
    assert_eq!(plan["lanes"][0]["state"], "queued");
    assert_eq!(plan["order"], json!([lane.to_string()]));

    let record = dispatch(&session, "plan.events", &json!({ "theme": theme.to_string() })).await.expect("plan.events");
    assert!(record["events"].as_array().is_some_and(|e| !e.is_empty()), "{record}");
    assert!(record["events"].as_array().unwrap().iter().any(|e| e["kind"] == "story"), "{record}");
    let lane_record = dispatch(&session, "plan.events", &json!({ "lane": lane.to_string() })).await.expect("lane events");
    assert!(lane_record["events"].is_array(), "{lane_record}");
}

/// A record needs one subject, and a plan needs its board; neither is guessed.
#[tokio::test]
async fn a_plan_read_names_its_subject() {
    let runner = a_runner(Scope::Read).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    for args in [json!({}), json!({ "theme": uuid::Uuid::nil().to_string(), "lane": uuid::Uuid::nil().to_string() })] {
        assert!(matches!(dispatch(&session, "plan.events", &args).await, Err(SessionError::Protocol(_))), "{args}");
    }
    assert!(matches!(dispatch(&session, "plan.get", &json!({})).await, Err(SessionError::Protocol(_))));
}

/// The layer's one writer is the orchestrator: every plan write has no phone
/// route and no arm, and the two reads have both.
#[tokio::test]
async fn a_phone_reads_the_plan_and_never_writes_it() {
    let runner = a_runner(Scope::Control).await;
    let session = Session::connect_local(&runner.socket).await.expect("connect");
    for method in [Method::PlanGet, Method::PlanEvents] {
        assert_eq!(route(method), Some(method.name()));
    }
    for method in [
        Method::PlanSet,
        Method::BoardThemeCreate,
        Method::BoardThemeUpdate,
        Method::BoardThemeCards,
        Method::LaneCreate,
        Method::LaneUpdate,
        Method::LaneCards,
        Method::LaneAgent,
    ] {
        assert_eq!(route(method), None, "{} has a phone route", method.name());
        let answer = dispatch(&session, method.name(), &json!({})).await;
        assert!(
            matches!(&answer, Err(SessionError::Protocol(m)) if m == &format!("unknown method: {}", method.name())),
            "`dispatch` has an arm for {}: {answer:?}",
            method.name()
        );
    }
}

/// A plan written on the runner reaches a phone as a `plan` notice naming the
/// board, and two in a row are one.
#[test]
fn a_plan_notice_names_its_board() {
    let board = uuid::Uuid::from_u128(7);
    let line = super::event_line(&crate::session::FleetEvent::Plan { workspace: board });
    assert_eq!(line, format!(r#"{{"event":"plan","workspace":"{board}"}}"#));
}

/// The owner's marks on a ruling, from a phone (ov-333): `ruling.keep` and
/// `ruling.keep_all` reach the runner, as the owner, and the plan reads them
/// kept. Neither can reverse or settle any other way: there's no arm for it.
#[tokio::test]
async fn a_phone_keeps_rulings_and_cannot_reverse_one() {
    use farcooler_store::rulings::{NewRuling, RulingState};
    let runner = a_runner(Scope::Control).await;
    let (main, _, _) = a_plan(&runner);
    let store = &runner.service.store;
    let new = |d: &str| NewRuling { decision: d.into(), why: "Why.".into(), reversal: "Cheap.".into(), theme_id: None };
    let one = store.add_ruling(main, &new("One"), &[], Actor::Manager).unwrap();
    let two = store.add_ruling(main, &new("Two"), &[], Actor::Manager).unwrap();
    let three = store.add_ruling(main, &new("Three"), &[], Actor::Manager).unwrap();
    let session = Session::connect_local(&runner.socket).await.expect("connect");

    assert_eq!(route(Method::RulingSet), Some("ruling.keep"));
    assert_eq!(route(Method::RulingKeepAll), Some("ruling.keep_all"));
    dispatch(&session, "ruling.keep", &json!({ "ruling": one.id.to_string() })).await.expect("keep");
    let kept = store.ruling(one.id).unwrap();
    assert_eq!((kept.state, kept.settled_by.as_deref()), (RulingState::Confirmed, Some("user")));
    assert_eq!(store.ruling(two.id).unwrap().state, RulingState::Standing, "only the one asked for");

    let all = dispatch(&session, "ruling.keep_all", &json!({ "workspace": main.to_string() })).await.expect("keep all");
    assert_eq!(all["kept"], 2, "the two still open");
    assert_eq!(store.ruling(three.id).unwrap().state, RulingState::Confirmed);

    let plan = dispatch(&session, "plan.get", &json!({ "workspace": main.to_string() })).await.expect("plan.get");
    assert!(plan["rulings"].as_array().unwrap().iter().all(|r| r["state"] == "confirmed"), "{plan}");
    assert!(plan["rulings"][0].get("reversed_sha").is_some());

    // The reverse mark is the orchestrator's: a phone has no arm for it.
    let answer = dispatch(&session, "ruling.reverse", &json!({ "ruling": two.id.to_string(), "sha": "abcd1234" })).await;
    assert!(matches!(&answer, Err(SessionError::Protocol(m)) if m == "unknown method: ruling.reverse"), "{answer:?}");
    assert!(matches!(dispatch(&session, "ruling.keep", &json!({})).await, Err(SessionError::Protocol(_))));
}

/// The two pull request verbs, from a phone (ov-317), the whole way: routed,
/// decoded from the JSON an app passes, sent as Control's and refused below it.
/// This daemon's `gh` is inert, so what is checked is the part the phone owns:
/// the route, the arguments, and the runner's own refusals reaching the app as
/// words it can switch on.
#[tokio::test]
async fn a_phone_s_pull_request_verbs_are_routed_and_refused_below_control() {
    use farcooler_protocol::v1::ErrorCode;
    assert_eq!(route(Method::PrReady), Some("pr.ready"));
    assert_eq!(route(Method::PrThreadReply), Some("pr.thread_reply"));
    let nobody = uuid::Uuid::from_u128(5).to_string();

    let read = a_runner(Scope::Read).await;
    let session = Session::connect_local(&read.socket).await.expect("connect");
    for (method, args) in [
        ("pr.ready", json!({ "lane": nobody, "pr_number": 12 })),
        ("pr.thread_reply", json!({ "task": nobody, "body": "Done.", "send": true })),
    ] {
        match dispatch(&session, method, &args).await {
            Err(SessionError::Refused { code, .. }) => assert_eq!(code, ErrorCode::ScopeDenied as i32, "{method}"),
            other => panic!("{method} below Control: {other:?}"),
        }
    }

    let control = a_runner(Scope::Control).await;
    let session = Session::connect_local(&control.socket).await.expect("connect");
    // Past the scope check: the lane and the card are not there.
    for (method, args) in [("pr.ready", json!({ "lane": nobody })), ("pr.thread_reply", json!({ "task": nobody }))] {
        match dispatch(&session, method, &args).await {
            Err(SessionError::Refused { code, .. }) => assert_eq!(code, ErrorCode::NotFound as i32, "{method}"),
            other => panic!("{method}: {other:?}"),
        }
    }
    // The arguments are named: a call without them never leaves the phone.
    assert!(matches!(dispatch(&session, "pr.ready", &json!({})).await, Err(SessionError::Protocol(m)) if m.contains("lane")));
    assert!(matches!(dispatch(&session, "pr.thread_reply", &json!({})).await, Err(SessionError::Protocol(m)) if m.contains("task")));
}

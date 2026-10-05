//! The plan's glance (ov-310): what a board with a plan sends the relay.

use super::*;
use farcooler_store::models::Actor;
use farcooler_store::plan::{LaneUpdate, NewLane, NewTheme, ThemeUpdate};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

fn lane(store: &Store, ws: Uuid, name: &str) -> Uuid {
    let new = NewLane { name: name.into(), reason: "Card text the glance never says".into(), ..NewLane::default() };
    store.create_lane(ws, &new, &[], None, Actor::Manager).unwrap().id
}

fn to(store: &Store, lane: Uuid, state: LaneState) {
    store.update_lane(lane, &LaneUpdate { state: Some(state), ..LaneUpdate::default() }, Actor::Manager).unwrap();
}

/// A needs-you item on `ws`, or on no board.
fn item(ws: Option<Uuid>) -> pb::NeedsYouItem {
    pb::NeedsYouItem { workspace_id: ws.map(crate::wire::id_bytes).unwrap_or_default(), ..Default::default() }
}

/// A board with no theme and no lane has no plan, and sends nothing.
#[tokio::test]
async fn a_board_without_a_plan_sends_no_glance() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    svc.store.ensure_main_workspace(repo).unwrap();
    assert!(planned(&svc.store, now_ms()).unwrap().is_empty());
}

/// Now is the first two live lanes past queued, in the plan's order; next
/// up is the plan's first queued lane; the count is the board's items plus
/// its themes asking the owner, the Mac's `WorkspaceNeedsYou.count`. And no
/// lane's reason, no theme's name, story or ask, reaches the wire.
#[tokio::test]
async fn a_board_with_a_plan_sends_now_next_up_and_its_needs_you_count() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let store = &svc.store;
    let [fix, review, build, fu3, fu4] =
        ["mac-fix", "mac-ux", "ov-310", "mac-fu3", "mac-fu4"].map(|name| lane(store, main.id, name));
    to(store, fix, LaneState::Building);
    to(store, fix, LaneState::Review);
    to(store, fix, LaneState::Fixing);
    to(store, review, LaneState::Building);
    to(store, review, LaneState::Review);
    to(store, build, LaneState::Building);
    store.set_plan(main.id, &[fu4, fu3], Actor::Manager).unwrap();
    let theme = |name: &str, outcome: &str| {
        let new = NewTheme { name: name.into(), outcome: outcome.into() };
        store.create_theme(main.id, &new, &[], Actor::Manager).unwrap()
    };
    let asking = theme("Visual language", "One look");
    let ask = ThemeUpdate { owner_ask: Some("Pick the accent".into()), ..ThemeUpdate::default() };
    store.update_theme(asking.id, &ask, Actor::Manager).unwrap();
    theme("Quiet theme", "Nothing asked");

    let items = [item(Some(main.id)), item(Some(main.id)), item(None), item(Some(Uuid::now_v7()))];
    let sent = boards(planned(store, now_ms()).unwrap(), &items);
    assert_eq!(
        sent,
        vec![BoardGlance {
            workspace: "Main".into(),
            needs_you: 3,
            now: vec![
                LaneGlance { name: "mac-fix".into(), state: "fixing" },
                LaneGlance { name: "mac-ux".into(), state: "review" },
            ],
            next: Some("mac-fu4".into()),
        }]
    );
    let wire = serde_json::to_string(&sent).unwrap();
    assert_eq!(
        wire,
        concat!(
            r#"[{"workspace":"Main","needsYou":3,"now":[{"name":"mac-fix","state":"fixing"},"#,
            r#"{"name":"mac-ux","state":"review"}],"next":"mac-fu4"}]"#
        )
    );
    for private in ["Card text", "Visual language", "Pick the accent", "One look"] {
        assert!(!wire.contains(private), "{private} in {wire}");
    }
}

/// A plan with nothing queued says no next up, and one where nothing is past
/// queued says no Now; a board busy now leads one that isn't.
#[tokio::test]
async fn the_busy_board_leads_and_absent_parts_stay_absent() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let ops = svc.store.create_workspace(repo, "Ops", "ops").unwrap();
    let store = &svc.store;
    let queued = lane(store, main.id, "mac-fu3");
    store.set_plan(main.id, &[queued], Actor::Manager).unwrap();
    to(store, lane(store, ops.id, "ops-1"), LaneState::Building);

    let sent = boards(planned(store, now_ms()).unwrap(), &[]);
    assert_eq!(sent.iter().map(|b| b.workspace.as_str()).collect::<Vec<_>>(), ["Ops", "Main"]);
    assert_eq!((sent[0].now.len(), sent[0].next.as_deref()), (1, None));
    assert_eq!((sent[1].now.len(), sent[1].next.as_deref()), (0, Some("mac-fu3")));
    assert!(!serde_json::to_string(&sent[0]).unwrap().contains("next"), "absent, never null");
}

/// The next notice the watcher sends, waiting out any debounce.
async fn next(taps: &mut tokio::sync::mpsc::UnboundedReceiver<crate::watch::Tapped>) -> Option<crate::watch::Tapped> {
    tokio::time::timeout(std::time::Duration::from_secs(30), taps.recv()).await.ok().flatten()
}

/// A lane that moves sends the glance on a count notice, though the count
/// stayed; the same glance again sends nothing.
#[tokio::test]
async fn a_lane_that_moves_sends_the_glance_on_a_count_notice() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let watcher = crate::watch::Watcher::new(svc.clone());
    let mut taps = watcher.tap_notices();
    tokio::time::pause();

    watcher.schedule_count_notice();
    let first = next(&mut taps).await.expect("the first count");
    assert_eq!(first.plan, Some(vec![]), "no plan is said as none");

    let ux = lane(&svc.store, main.id, "mac-ux");
    to(&svc.store, ux, LaneState::Building);
    watcher.announce_plan_changed(main.id, Actor::Manager);
    let moved = next(&mut taps).await.expect("a lane that moved is news");
    assert_eq!((moved.kind, moved.needs_you), (Some("count"), first.needs_you));
    let board = &moved.plan.as_ref().expect("a glance")[0];
    assert_eq!(board.now, vec![LaneGlance { name: "mac-ux".into(), state: "building" }]);

    watcher.announce_plan_changed(main.id, Actor::Manager);
    assert!(next(&mut taps).await.is_none(), "a glance the relay holds is not sent again");
}

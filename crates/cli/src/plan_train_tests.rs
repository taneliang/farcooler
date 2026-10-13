//! Which lanes board a train (ov-503), said before anything is sent. A child
//! of `plan_tests.rs`, whose runner it drives.

use super::*;

fn methods(link: &Runner) -> Vec<&str> {
    link.sent.iter().map(|r| r.method.as_str()).collect()
}

/// A lane still in review is refused with where it is and how to board it, and
/// nothing is sent but the read.
#[tokio::test]
async fn a_lane_in_review_does_not_board_without_force() {
    let mut link = runner();
    let err = say(&mut link, "train start integ-10 --lane mac-ux").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-ux is in review. Move it to landing when its review is clean, or add --force to board it now.");
    assert_eq!(methods(&link), ["plan.get"]);
}

/// With --force, a lane in review is moved to landing by a lane write of its
/// own, and then the train starts with it.
#[tokio::test]
async fn force_moves_a_lane_in_review_to_landing_first() {
    let mut link = runner();
    say(&mut link, "train start integ-10 --lane mac-ux --force").await.unwrap();
    assert_eq!(methods(&link), ["plan.get", "lane.update", "train.start"]);
    let Some(request::Payload::LaneUpdate(p)) = &link.sent[1].payload else { panic!() };
    assert_eq!((p.lane_id.clone(), p.state, p.reason.as_deref()), (tid(0x2002), Some(pb::LaneState::Landing as i32), Some("Boarded a train with --force.")));
}

/// A queued lane never boards, with or without --force, and neither does a
/// landed one. The refusal names the lane and nothing is moved.
#[tokio::test]
async fn a_queued_or_landed_lane_never_boards() {
    let mut link = runner();
    for cmd in ["train start integ-10 --lane mac-fu3", "train start integ-10 --lane mac-ux --lane mac-fu3 --force"] {
        let err = say(&mut link, cmd).await.unwrap_err();
        assert_eq!(err.to_string(), "mac-fu3 is queued. A train takes only lanes that are ready to land, so it waits for the next one.");
    }
    let err = say(&mut link, "train set integ-9 --add-lane fix-ac84 --force").await.unwrap_err();
    assert_eq!(err.to_string(), "fix-ac84 has already landed, so it can't board a train.");
    assert!(methods(&link).iter().all(|m| *m == "plan.get"), "{:?}", methods(&link));
}

/// --add-lane follows the rule too; a lane already on the train is not asked
/// again.
#[tokio::test]
async fn adding_a_lane_follows_the_same_rule() {
    let mut link = runner();
    let err = say(&mut link, "train set integ-9 --add-lane mac-fu3").await.unwrap_err();
    assert!(err.to_string().starts_with("mac-fu3 is queued."), "{err}");
    say(&mut link, "train set integ-9 --add-lane mac-ux").await.unwrap();
    assert_eq!(methods(&link).last(), Some(&"train.set"), "mac-ux is on integ-9 already");
    assert!(!methods(&link).contains(&"lane.update"));
}

/// A name in use, or a train that has landed, is refused before a forced lane
/// is moved to landing.
#[tokio::test]
async fn nothing_is_moved_for_a_train_that_cannot_take_it() {
    let mut link = runner();
    let err = say(&mut link, "train start INTEG-9 --lane mac-ux --force").await.unwrap_err();
    assert!(err.to_string().starts_with("That train name is already used"), "{err}");
    assert_eq!(methods(&link), ["plan.get"]);
}

/// Two trains still integrating, `integ-1` first. On `integ-1`: `go-one` and
/// `ready-one` ready to land, `slow-one` sent back for a fix, so it is held.
fn held_plan() -> pb::Plan {
    let mut plan = the_plan();
    let on = |mut l: pb::Lane, train: &str| {
        l.train = Some(train.into());
        l
    };
    plan.lanes = vec![
        on(lane(10, "go-one", pb::LaneState::Landing, &[1]), "integ-1"),
        on(lane(11, "ready-one", pb::LaneState::Landing, &[2]), "integ-1"),
        on(lane(12, "slow-one", pb::LaneState::Fixing, &[3]), "integ-1"),
        on(lane(13, "last-one", pb::LaneState::Landing, &[1]), "integ-2"),
    ];
    let train = |n: u128, name: &str, created: i64, lanes: &[u128]| pb::BoardTrain {
        id: tid(0x6000 + n),
        workspace_id: tid(0x0202),
        name: name.into(),
        title: format!("Train {n}"),
        state: pb::BoardTrainState::Integrating as i32,
        created_at: created,
        state_since: created,
        lane_ids: lanes.iter().map(|l| tid(0x2000 + l)).collect(),
        ..Default::default()
    };
    let mut first = train(1, "integ-1", NOW - 4 * HOUR, &[10, 11, 12]);
    first.phase = pb::BoardTrainPhase::WaitingForLanes as i32;
    first.phase_since = NOW - 2 * HOUR;
    first.held_by = vec![tid(0x2000 + 12)];
    plan.trains = vec![first, train(2, "integ-2", NOW - 3 * HOUR, &[13])];
    plan
}

fn sent_set(link: &Runner) -> &pb::TrainSet {
    let Some(request::Payload::TrainSet(p)) = &link.sent.last().expect("sent").payload else { panic!("not a train.set") };
    p
}

/// A held train is listed with what holds it and the verb that releases it.
#[tokio::test]
async fn a_held_train_says_what_holds_it() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train list").await.unwrap();
    assert!(out.contains("Train 1 (integ-1) · Waiting for lanes"), "{out}");
    assert!(out.contains("waiting for lanes for 2h"), "{out}");
    let hint = format!("Holding for slow-one. `{} plan train move slow-one --actor manager`", crate::runner_install::cli_name());
    assert!(out.contains(&hint), "{out}");
    assert!(!out.contains("Train 2 (integ-2) · Waiting"), "the other train is not held");
}

/// A ready lane moves to the next train in one write on it.
#[tokio::test]
async fn a_ready_lane_moves_to_the_next_train() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move go-one").await.unwrap();
    assert_eq!(out, "Moved go-one from Train 1 to Train 2.");
    assert_eq!(methods(&link), ["plan.get", "train.set"]);
    let p = sent_set(&link);
    assert_eq!((p.train_id.clone(), p.add_lane_ids.clone(), p.remove_lane_ids.len()), (tid(0x6002), vec![tid(0x2000 + 10)], 0));
}

/// The lane that holds the train is not ready, so it can't board another
/// one: it comes off, and waits.
#[tokio::test]
async fn a_lane_that_is_not_ready_comes_off_and_waits() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move slow-one").await.unwrap();
    assert_eq!(out, "Took slow-one off Train 1. It waits until it's ready to land, then boards a train.");
    let p = sent_set(&link);
    assert_eq!((p.train_id.clone(), p.add_lane_ids.len(), p.remove_lane_ids.clone()), (tid(0x6001), 0, vec![tid(0x2000 + 12)]));
}

/// With no later train, even a ready lane just comes off.
#[tokio::test]
async fn with_no_next_train_a_lane_goes_back_to_waiting() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let out = say(&mut link, "train move last-one").await.unwrap();
    assert_eq!(out, "Took last-one off Train 2. It's ready to land and boards the next train that starts.");
    assert_eq!(sent_set(&link).remove_lane_ids, vec![tid(0x2000 + 13)]);
}

/// Refusals send nothing: a train named outright takes only a ready lane, a
/// pushed train keeps its lanes, and a lane on no train has nothing to leave.
#[tokio::test]
async fn a_move_that_cannot_happen_is_refused_before_anything_is_sent() {
    let mut link = runner();
    link.plan = Some(held_plan());
    let err = say(&mut link, "train move slow-one --to integ-2").await.unwrap_err();
    assert!(err.to_string().starts_with("slow-one is fixing. A train takes only lanes that are ready to land"), "{err}");
    let err = say(&mut link, "train move go-one --to integ-1").await.unwrap_err();
    assert!(err.to_string().contains("isn't integrating"), "{err}");

    let mut pushed = held_plan();
    pushed.trains[0].state = pb::BoardTrainState::Pushed as i32;
    pushed.trains[0].pushed_sha = Some("1a1b3275".into());
    link.plan = Some(pushed);
    let err = say(&mut link, "train move go-one").await.unwrap_err();
    assert!(err.to_string().contains("has already gone to gating or CI"), "{err}");

    link.plan = Some(the_plan());
    let err = say(&mut link, "train move mac-fu3").await.unwrap_err();
    assert_eq!(err.to_string(), "mac-fu3 isn't on a train.");
    assert!(methods(&link).iter().all(|m| *m == "plan.get"), "{:?}", methods(&link));
}

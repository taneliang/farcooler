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

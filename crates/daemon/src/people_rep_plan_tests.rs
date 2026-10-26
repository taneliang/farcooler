//! The plan changes a rep makes for its member, and only those (ov-736):
//! `rep_may` lets through exactly what its gate's grammar reaches
//! (`rep_gate::PLAN_VERBS`). One test per change it refuses, so a widening
//! of any one goes red on its own.

use farcooler_protocol::v1::{self as pb, BoardRulingState, LaneState};

use super::*;

fn req(method: &str, payload: request::Payload) -> Request {
    Request { method: method.into(), payload: Some(payload), ..Default::default() }
}

fn lane_update(edit: impl FnOnce(&mut pb::LaneUpdate)) -> Request {
    let mut p = pb::LaneUpdate::default();
    edit(&mut p);
    req("lane.update", request::Payload::LaneUpdate(p))
}

fn ruling_set(state: BoardRulingState, sha: Option<&str>) -> Request {
    req("ruling.set", request::Payload::RulingSet(pb::RulingSet { state: state as i32, sha: sha.map(str::to_string), ..Default::default() }))
}

/// What the grammar reaches passes: the ranking, a theme made, set and its
/// cards, a lane's title, reason and cards and its state to queued or
/// dropped, a ruling recorded and one reversed with no commit.
#[test]
fn a_rep_makes_the_plan_changes_its_gate_reaches() {
    use request::Payload as P;
    let lane = || vec![1u8; 16].into();
    for allowed in [
        req("plan.set", P::PlanSet(pb::PlanSet { lane_ids: vec![lane()], ..Default::default() })),
        req("board_theme.create", P::BoardThemeCreate(pb::BoardThemeCreate::default())),
        req("board_theme.update", P::BoardThemeUpdate(pb::BoardThemeUpdate { next: Some("Receipts".into()), ..Default::default() })),
        req("board_theme.cards", P::BoardThemeCards(pb::BoardThemeCards::default())),
        // Its place in the ranking: the plan's order is the whole team's (ov-755).
        req("board_theme.move", P::BoardThemeMove(pb::BoardThemeMove { to: 2, ..Default::default() })),
        lane_update(|p| p.title = Some("PDF invoices".into())),
        lane_update(|p| p.reason = Some("Invoices first".into())),
        lane_update(|p| p.state = Some(LaneState::Queued as i32)),
        lane_update(|p| p.state = Some(LaneState::Dropped as i32)),
        req("lane.cards", P::LaneCards(pb::LaneCards::default())),
        req("ruling.add", P::RulingAdd(pb::RulingAdd::default())),
        ruling_set(BoardRulingState::Reversed, None),
    ] {
        assert!(rep_may(&allowed), "{}: {:?}", allowed.method, allowed.payload);
    }
}

macro_rules! refused {
    ($($name:ident: $req:expr;)*) => {$(
        #[test]
        fn $name() {
            let r: Request = $req;
            assert!(!rep_may(&r), "a rep may not: {} {:?}", r.method, r.payload);
        }
    )*};
}

refused! {
    a_rep_keeps_no_ruling: ruling_set(BoardRulingState::Confirmed, None);
    a_rep_keeps_no_ruling_with_a_commit: ruling_set(BoardRulingState::Confirmed, Some("abc123"));
    a_rep_marks_no_reversal_done: ruling_set(BoardRulingState::Reversed, Some("abc123"));
    a_rep_sets_no_ruling_standing: ruling_set(BoardRulingState::Standing, None);
    a_rep_keeps_no_rulings_at_once: req("ruling.keep_all", request::Payload::RulingKeepAll(pb::RulingKeepAll::default()));
    a_rep_starts_no_train: req("train.start", request::Payload::TrainStart(pb::TrainStart::default()));
    a_rep_sets_no_train: req("train.set", request::Payload::TrainSet(pb::TrainSet::default()));
    a_rep_sets_no_page: req("page.set", request::Payload::PageSet(pb::PageSet::default()));
    a_rep_removes_no_page: req("page.remove", request::Payload::PageRemove(pb::PageRemove::default()));
    a_rep_makes_no_lane: req("lane.create", request::Payload::LaneCreate(pb::LaneCreate::default()));
    a_rep_records_no_lane_agent: req("lane.agent", request::Payload::LaneAgentSet(pb::LaneAgentSet::default()));
    a_rep_lands_no_lane: lane_update(|p| p.state = Some(LaneState::Landing as i32));
    a_rep_marks_no_lane_landed: lane_update(|p| p.state = Some(LaneState::Landed as i32));
    a_rep_moves_no_lane_to_review: lane_update(|p| p.state = Some(LaneState::Review as i32));
    a_rep_sets_no_lane_sha: lane_update(|p| p.landed_sha = Some("abc123".into()));
    a_rep_puts_no_lane_on_a_train: lane_update(|p| p.train = Some("train-1".into()));
    a_rep_sets_no_lane_checkout: lane_update(|p| p.worktree_path = Some("/elsewhere".into()));
    a_rep_sets_no_lane_checkout_by_id: lane_update(|p| p.worktree_id = Some(vec![1u8; 16].into()));
    a_rep_sets_no_lane_branch: lane_update(|p| p.branch = Some("other".into()));
    a_rep_sets_no_lane_agent: lane_update(|p| p.agent = Some(pb::LaneAgentRecord::default()));
    a_rep_sets_no_lane_budget: lane_update(|p| p.budget_tokens = Some(5_000_000));
    a_rep_sets_no_theme_budget: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { budget_tokens: Some(1), ..Default::default() }));
    a_rep_sets_no_theme_budget_while_moving_it: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { ordinal: Some(1), budget_tokens: Some(1), ..Default::default() }));
    a_rep_moves_no_theme_through_the_general_update: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { ordinal: Some(1), ..Default::default() }));
    a_rep_sends_no_bare_theme_move: Request { method: "board_theme.move".into(), ..Default::default() };
    a_rep_empties_no_ranking: req("plan.set", request::Payload::PlanSet(pb::PlanSet::default()));
    a_rep_sends_no_bare_plan_write: Request { method: "plan.set".into(), ..Default::default() };
    a_rep_sends_no_mismatched_payload: req("plan.set", request::Payload::RulingKeepAll(pb::RulingKeepAll::default()));
}

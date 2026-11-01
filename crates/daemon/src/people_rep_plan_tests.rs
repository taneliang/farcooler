//! The plan changes a rep makes for its member, and only those (ov-736):
//! `rep_may` lets through exactly what its gate's grammar reaches
//! (`rep_gate::PLAN_VERBS`). One test per change it refuses, so a widening
//! of any one goes red on its own.

use farcooler_protocol::v1::{self as pb, BoardRulingState, TicketStage};

use super::*;

fn req(method: &str, payload: request::Payload) -> Request {
    Request { method: method.into(), payload: Some(payload), ..Default::default() }
}

fn line_set(edit: impl FnOnce(&mut pb::TicketLineSet)) -> Request {
    let mut p = pb::TicketLineSet::default();
    edit(&mut p);
    req("line.set", request::Payload::LineSet(p))
}

fn ruling_set(state: BoardRulingState, sha: Option<&str>) -> Request {
    req("ruling.set", request::Payload::RulingSet(pb::RulingSet { state: state as i32, sha: sha.map(str::to_string), ..Default::default() }))
}

/// What the grammar reaches passes: the ranking, a theme made, set and its
/// cards, a ticket's reason and its stage to waiting or dropped, a ruling
/// recorded and one reversed with no commit.
#[test]
fn a_rep_makes_the_plan_changes_its_gate_reaches() {
    use request::Payload as P;
    let ticket = || vec![1u8; 16].into();
    for allowed in [
        req("line.order", P::LineOrder(pb::LineOrder { task_ids: vec![ticket()], ..Default::default() })),
        req("board_theme.create", P::BoardThemeCreate(pb::BoardThemeCreate::default())),
        req("board_theme.update", P::BoardThemeUpdate(pb::BoardThemeUpdate { next: Some("Receipts".into()), ..Default::default() })),
        req("board_theme.cards", P::BoardThemeCards(pb::BoardThemeCards::default())),
        // Its place in the ranking: the plan's order is the whole team's (ov-755).
        req("board_theme.move", P::BoardThemeMove(pb::BoardThemeMove { to: 2, ..Default::default() })),
        line_set(|p| p.reason = Some("Invoices first".into())),
        line_set(|p| p.stage = Some(TicketStage::Waiting as i32)),
        line_set(|p| p.stage = Some(TicketStage::Dropped as i32)),
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
    a_rep_starts_no_landing: req("landing.start", request::Payload::LandingStart(pb::LandingStart::default()));
    a_rep_sets_no_landing: req("landing.set", request::Payload::LandingSet(pb::LandingSet::default()));
    a_rep_sets_no_page: req("page.set", request::Payload::PageSet(pb::PageSet::default()));
    a_rep_removes_no_page: req("page.remove", request::Payload::PageRemove(pb::PageRemove::default()));
    a_rep_records_no_agent: req("agent.record", request::Payload::AgentRecord(pb::AgentRecord::default()));
    a_rep_sets_no_agent_to_work: req("agent.work", request::Payload::AgentWork(pb::AgentWork::default()));
    a_rep_ends_no_agent: req("agent.end", request::Payload::AgentEnd(pb::AgentEnd::default()));
    a_rep_lands_no_ticket: line_set(|p| p.stage = Some(TicketStage::Landing as i32));
    a_rep_marks_no_ticket_landed: line_set(|p| p.stage = Some(TicketStage::Landed as i32));
    a_rep_moves_no_ticket_to_review: line_set(|p| p.stage = Some(TicketStage::Reviewing as i32));
    a_rep_moves_no_ticket_to_building: line_set(|p| p.stage = Some(TicketStage::Building as i32));
    a_rep_sets_no_ticket_sha: line_set(|p| p.landed_sha = Some("abc123".into()));
    a_rep_sets_no_ticket_checkout: line_set(|p| p.worktree_path = Some("/elsewhere".into()));
    a_rep_sets_no_ticket_checkout_by_id: line_set(|p| p.worktree_id = Some(vec![1u8; 16].into()));
    a_rep_sets_no_ticket_branch: line_set(|p| p.branch = Some("other".into()));
    a_rep_stacks_no_ticket: line_set(|p| p.stack_parent = Some(vec![1u8; 16].into()));
    a_rep_sets_no_ticket_pull_request: line_set(|p| p.pr_number = Some(12));
    a_rep_sets_no_ticket_budget: line_set(|p| p.budget_tokens = Some(5_000_000));
    a_rep_sets_no_theme_budget: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { budget_tokens: Some(1), ..Default::default() }));
    a_rep_sets_no_theme_budget_while_moving_it: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { ordinal: Some(1), budget_tokens: Some(1), ..Default::default() }));
    a_rep_moves_no_theme_through_the_general_update: req("board_theme.update", request::Payload::BoardThemeUpdate(pb::BoardThemeUpdate { ordinal: Some(1), ..Default::default() }));
    a_rep_sends_no_bare_theme_move: Request { method: "board_theme.move".into(), ..Default::default() };
    a_rep_empties_no_ranking: req("line.order", request::Payload::LineOrder(pb::LineOrder::default()));
    a_rep_sends_no_bare_plan_write: Request { method: "line.order".into(), ..Default::default() };
    a_rep_sends_no_mismatched_payload: req("line.order", request::Payload::RulingKeepAll(pb::RulingKeepAll::default()));
    a_rep_sends_no_retired_plan_write: req("plan.set", request::Payload::RulingKeepAll(pb::RulingKeepAll::default()));
}

//! Which pull request is whose, and what a lane says when it has several, and
//! when it can't ask GitHub at all.

use super::*;
use crate::pr_stage::fixture_prs;
use farcooler_store::plan::{Lane, LaneCard};
use farcooler_store::plan_read::LaneSpend;

fn task(n: u32) -> Uuid {
    Uuid::from_u128(0x1000 + n as u128)
}

fn card(n: u32, status: TaskStatus) -> CardRef {
    CardRef { task_id: task(n), key: format!("ov-{n}"), title: format!("Card {n}"), status }
}

fn lane(state: LaneState, branch: &str, cards: &[u32]) -> LaneView {
    LaneView {
        lane: Lane {
            id: Uuid::from_u128(0x2000),
            workspace_id: Uuid::from_u128(0x3000),
            name: "team".into(),
            state,
            reason: String::new(),
            plan_rank: None,
            worktree_id: None,
            worktree_path: String::new(),
            branch: branch.into(),
            harness: String::new(),
            model: String::new(),
            train: None,
            landed_sha: None,
            state_since: 0,
            created_at: 0,
            resource_version: 1,
        },
        cards: cards.iter().map(|n| LaneCard { task_id: task(*n), slice: String::new() }).collect(),
        agents: Vec::new(),
        fix_rounds: 0,
        spend: LaneSpend::default(),
        budget_tokens: None,
        stale: false,
    }
}

fn open_cards(ns: &[u32]) -> Vec<CardRef> {
    ns.iter().map(|n| card(*n, TaskStatus::InProgress)).collect()
}

fn heard() -> Reads {
    Reads { known: true, prs: fixture_prs() }
}

fn kind_of(s: &LaneStages, n: u32) -> Option<Kind> {
    s.cards.get(&task(n)).map(|s| s.kind)
}

/// The rolled-up lane stage across two card PRs: the owner's ruling (one PR per
/// card) and the least advanced of them.
#[test]
fn a_lane_with_two_card_prs_shows_each_and_the_least_advanced() {
    let s = lane_stages(&lane(LaneState::Review, "team", &[3, 8]), &open_cards(&[3, 8]), &heard());
    assert_eq!(kind_of(&s, 3), Some(Kind::WaitingOnReviewer), "ov-3's own PR");
    assert_eq!(kind_of(&s, 8), Some(Kind::Approved), "ov-8's own PR");
    let rolled = s.lane.expect("a lane stage");
    assert_eq!((rolled.kind, rolled.pr_number), (Kind::WaitingOnReviewer, 103), "the slower card sets it");

    let swapped = lane_stages(&lane(LaneState::Landing, "team", &[8, 6]), &open_cards(&[8, 6]), &heard());
    assert_eq!(swapped.lane.unwrap().kind, Kind::ApprovedChecksRunning);
}

#[test]
fn a_card_with_no_pr_yet_holds_the_lane_at_building() {
    let s = lane_stages(&lane(LaneState::Review, "team", &[8, 30]), &open_cards(&[8, 30]), &heard());
    assert_eq!(kind_of(&s, 30), Some(Kind::Building));
    assert_eq!(s.lane.unwrap().kind, Kind::Building);
}

/// A finished card with no PR isn't waiting on one.
#[test]
fn a_finished_card_with_no_pr_does_not_hold_the_lane() {
    let cards = vec![card(8, TaskStatus::InProgress), card(30, TaskStatus::Done)];
    let s = lane_stages(&lane(LaneState::Review, "team", &[8, 30]), &cards, &heard());
    assert_eq!(kind_of(&s, 30), None);
    assert_eq!(s.lane.unwrap().kind, Kind::Approved);
}

/// `ov-1` is not in `ov-12/pr-stage`, and a key counts however the title or
/// branch cases it.
#[test]
fn a_key_names_a_pr_only_as_a_whole_word() {
    assert!(mentions("ov-12/pr-stage", "ov-12"));
    assert!(!mentions("ov-12/pr-stage", "ov-1"));
    assert!(!mentions("xov-1/pr-stage", "ov-1"));
    assert!(mentions("OV-1: Lanes show their stage", "ov-1"));
    assert!(mentions("Lanes show their stage (ov-1)", "ov-1"));
    assert!(!mentions("anything", ""));
    // Through the fixture: card 1 gets PR 101 and card 12 PR 112, never crossed.
    let prs = fixture_prs();
    assert_eq!(pr_of(&prs, "ov-1", "", false).unwrap().status.number, 101);
    assert_eq!(pr_of(&prs, "ov-12", "", false).unwrap().status.number, 112);
}

/// A lane's one card takes the PR on the lane's own branch when nothing names it.
#[test]
fn a_lane_s_only_card_takes_the_lane_s_branch_pr() {
    let mut prs = fixture_prs();
    prs[0].head_ref = "mac-ux".into();
    prs[0].title = "Mac fixes".into();
    let reads = Reads { known: true, prs };
    let s = lane_stages(&lane(LaneState::Review, "mac-ux", &[40]), &open_cards(&[40]), &reads);
    assert_eq!(s.lane.unwrap().pr_number, 101);
}

/// With several cards the lane's branch holds all of their work, so it says
/// nothing of any one: the card with no PR of its own reads no PR, and the lane
/// is only as far along as that card.
#[test]
fn a_card_with_no_pr_does_not_take_a_sibling_s_through_the_lane_s_branch() {
    let mut prs = fixture_prs();
    // Card 8's PR is on the lane's own branch; card 40 has none.
    prs.iter_mut().find(|p| p.status.number == 108).unwrap().head_ref = "mac-ux".into();
    let reads = Reads { known: true, prs };
    let s = lane_stages(&lane(LaneState::Review, "mac-ux", &[8, 40]), &open_cards(&[8, 40]), &reads);
    assert_eq!(kind_of(&s, 8), Some(Kind::Approved));
    assert_eq!(kind_of(&s, 40), Some(Kind::Building), "its own PR is not yet open");
    assert_eq!(s.lane.unwrap().kind, Kind::Building, "the lane is as far along as its slowest card");
}

/// A PR that cites a card is not that card's PR: only its head branch, or a
/// title that opens with its key, claims one.
#[test]
fn a_pr_that_only_mentions_a_card_does_not_claim_it() {
    let prs = fixture_prs();
    // PR 123 is titled "ov-20: A follow-up that cites ov-10 and ov-3".
    assert_eq!(pr_of(&prs, "ov-10", "", false).unwrap().status.number, 110, "the merged PR keeps its card");
    assert_eq!(pr_of(&prs, "ov-3", "", false).unwrap().status.number, 103);
    assert_eq!(pr_of(&prs, "ov-20", "", false).unwrap().status.number, 123);
    let s = lane_stages(&lane(LaneState::Landed, "x", &[10]), &open_cards(&[10]), &Reads { known: true, prs });
    assert_eq!(s.lane.unwrap().label(), "Merged");
}

#[test]
fn a_title_claims_a_card_only_when_it_opens_with_the_key() {
    assert!(title_starts_with("ov-312: lanes show their stage", "ov-312"));
    assert!(title_starts_with("[ov-312] lanes", "ov-312"));
    assert!(title_starts_with("OV-312 lanes", "ov-312"));
    assert!(title_starts_with("ov-312", "ov-312"));
    assert!(!title_starts_with("ov-3120: other", "ov-312"));
    assert!(!title_starts_with("ov-31: other", "ov-312"));
    assert!(!title_starts_with("Lanes show their stage (ov-312)", "ov-312"));
}

/// A revert of a card's PR, and a fork's PR on a branch of the same name, are
/// not the card's own.
#[test]
fn a_revert_and_a_fork_are_not_a_card_s_pr() {
    let prs = fixture_prs();
    // 121 reverts card 8's PR, 122 is a fork's PR for card 9: each is newer and
    // open, and each would win if it counted.
    assert_eq!(pr_of(&prs, "ov-8", "", false).unwrap().status.number, 108);
    assert_eq!(pr_of(&prs, "ov-9", "", false).unwrap().status.number, 109);
}

/// Two open PRs claiming one card: the newest wins.
#[test]
fn the_newest_of_two_open_prs_wins() {
    let prs = fixture_prs();
    // 104 and 125 both claim card 4.
    assert_eq!(pr_of(&prs, "ov-4", "", false).unwrap().status.number, 125);
}

/// The newest live PR on a card wins over an earlier closed attempt.
#[test]
fn a_live_pr_beats_an_abandoned_one_on_the_same_card() {
    let mut prs = fixture_prs();
    let mut old = prs[7].clone();
    old.status.number = 90;
    old.status.state = PrState::Closed;
    prs.push(old);
    assert_eq!(pr_of(&prs, "ov-8", "", false).unwrap().status.number, 108);
}

/// The distinction the whole feature rests on: a runner that could not ask
/// GitHub, and one that asked and heard no PR, say different things.
#[test]
fn unknown_is_not_no_pr() {
    let unheard = Reads { known: false, prs: Vec::new() };
    let none = Reads { known: true, prs: Vec::new() };
    let cards = open_cards(&[1]);

    // A lane in review: cannot ask is Unknown; asked and none is silence.
    let review = lane(LaneState::Review, "team", &[1]);
    assert_eq!(lane_stages(&review, &cards, &unheard).lane.unwrap().kind, Kind::Unknown);
    assert_eq!(lane_stages(&review, &cards, &none).lane, None);

    // A building lane: cannot ask is silence (no PR is expected yet); asked and
    // none is Building.
    let building = lane(LaneState::Building, "team", &[1]);
    assert_eq!(lane_stages(&building, &cards, &unheard).lane, None);
    assert_eq!(lane_stages(&building, &cards, &none).lane.unwrap().kind, Kind::Building);

    // The two never share an answer on a lane in landing either.
    let landing = lane(LaneState::Landing, "team", &[1]);
    assert_eq!(lane_stages(&landing, &cards, &unheard).cards[&task(1)].label(), "PR state unknown");
    assert!(lane_stages(&landing, &cards, &none).cards.is_empty());
}

#[test]
fn a_cache_that_never_answered_is_not_an_empty_list() {
    // `Reads::of` keeps the two apart through `pr_answer_is_known`; a default
    // read is "could not ask".
    assert!(!Reads::default().known);
}

#[test]
fn a_lane_not_started_has_no_stage() {
    let s = lane_stages(&lane(LaneState::Queued, "team", &[8]), &open_cards(&[8]), &heard());
    assert_eq!(s, LaneStages::default());
}

#[test]
fn a_landed_lane_shows_its_merged_pr() {
    let s = lane_stages(&lane(LaneState::Landed, "team", &[10]), &open_cards(&[10]), &heard());
    assert_eq!(s.lane.unwrap().label(), "Merged");
    // And with nothing known it says nothing rather than Unknown.
    let s = lane_stages(&lane(LaneState::Landed, "team", &[10]), &open_cards(&[10]), &Reads::default());
    assert_eq!(s.lane, None);
}

#[test]
fn the_wire_carries_the_words_and_the_facts() {
    let s = lane_stages(&lane(LaneState::Review, "team", &[3]), &open_cards(&[3]), &heard()).lane.unwrap();
    let wire = pb_stage(&s);
    assert_eq!(wire.kind, pb::PrStageKind::WaitingOnReviewer as i32);
    assert_eq!((wire.label.as_str(), wire.reviewers.as_slice(), wire.pr_number), ("Waiting on alice", &["alice".to_string()][..], 103));
    assert_eq!(wire.pr_url, "https://github.example/o/r/pull/103");
}

//! Which pull request is whose, what a ticket says of it, and what it says when
//! it can't ask GitHub at all.

use super::*;
use crate::pr_stage::fixture_prs;
use farcooler_store::plan_read::Spend;
use farcooler_store::ticket_lines::TicketLine;
use farcooler_store::ticket_work::{LinkedBy, TicketWork};

fn task(n: u32) -> Uuid {
    Uuid::from_u128(0x1000 + n as u128)
}

fn card(n: u32, status: TaskStatus) -> CardRef {
    CardRef { task_id: task(n), key: format!("ov-{n}"), title: format!("Card {n}"), status }
}

/// Ticket `n` on the line in `stage`, on `branch`.
fn ticket(stage: LineStage, branch: &str, n: u32) -> LineView {
    LineView {
        line: TicketLine {
            task_id: task(n),
            stage,
            reason: String::new(),
            branch: branch.into(),
            worktree_id: None,
            worktree_path: String::new(),
            stack_parent: None,
            same_branch: None,
            pr_number: None,
            landed_sha: None,
            plan_rank: None,
            stage_since: 0,
            held_off: false,
            created_at: 0,
            resource_version: 1,
        },
        work: Vec::new(),
        fix_rounds: 0,
        spend: Spend::default(),
        budget_tokens: None,
        stale: false,
        landing: None,
    }
}

fn open_cards(ns: &[u32]) -> Vec<CardRef> {
    ns.iter().map(|n| card(*n, TaskStatus::InProgress)).collect()
}

fn heard() -> Reads {
    Reads { known: true, prs: fixture_prs() }
}

fn stage_of(line: &LineView, cards: &[CardRef], reads: &Reads) -> Option<Kind> {
    line_stage(line, cards, reads).map(|s| s.kind)
}

/// Each ticket reads its own pull request: one per ticket, the owner's ruling.
#[test]
fn each_ticket_shows_its_own_pr() {
    let cards = open_cards(&[3, 8, 6]);
    assert_eq!(stage_of(&ticket(LineStage::Reviewing, "a", 3), &cards, &heard()), Some(Kind::WaitingOnReviewer));
    assert_eq!(stage_of(&ticket(LineStage::Reviewing, "b", 8), &cards, &heard()), Some(Kind::Approved));
    assert_eq!(stage_of(&ticket(LineStage::Landing, "c", 6), &cards, &heard()), Some(Kind::ApprovedChecksRunning));
}

#[test]
fn a_building_ticket_with_no_pr_yet_reads_building() {
    assert_eq!(stage_of(&ticket(LineStage::Building, "team", 30), &open_cards(&[30]), &heard()), Some(Kind::Building));
}

/// A finished card with no PR isn't waiting on one.
#[test]
fn a_finished_card_with_no_pr_has_no_stage() {
    let cards = vec![card(30, TaskStatus::Done)];
    assert_eq!(stage_of(&ticket(LineStage::Reviewing, "team", 30), &cards, &heard()), None);
}

/// A draft pull request reads as a fix round while a fix is open on the ticket,
/// and as the agent's review otherwise.
#[test]
fn an_open_fix_reads_as_fixing() {
    let mut line = ticket(LineStage::Building, "team", 1);
    assert_eq!(stage_of(&line, &open_cards(&[1]), &heard()), Some(Kind::Building));
    line.work.push(TicketWork {
        agent_id: Uuid::from_u128(0x4000),
        task_id: task(1),
        role: WorkRole::Fix,
        started_at: 0,
        ended_at: None,
        verdict: None,
        linked_by: LinkedBy::Orchestrator,
    });
    assert_eq!(line_phase(&line), Phase::Fixing);
    assert_eq!(stage_of(&line, &open_cards(&[1]), &heard()), Some(Kind::Fixing));
    assert_eq!(stage_of(&ticket(LineStage::Reviewing, "team", 1), &open_cards(&[1]), &heard()), Some(Kind::AgentReview));
}

/// `ov-1` is not in `ov-12/pr-stage`, and a key counts however the title or
/// branch cases it.
#[test]
fn a_key_names_a_pr_only_as_a_whole_word() {
    assert!(mentions("ov-12/pr-stage", "ov-12"));
    assert!(!mentions("ov-12/pr-stage", "ov-1"));
    assert!(!mentions("xov-1/pr-stage", "ov-1"));
    assert!(mentions("OV-1: Tickets show their stage", "ov-1"));
    assert!(mentions("Tickets show their stage (ov-1)", "ov-1"));
    assert!(!mentions("anything", ""));
    // Through the fixture: card 1 gets PR 101 and card 12 PR 112, never crossed.
    let prs = fixture_prs();
    assert_eq!(pr_of(&prs, "ov-1", "", false).unwrap().status.number, 101);
    assert_eq!(pr_of(&prs, "ov-12", "", false).unwrap().status.number, 112);
}

/// A ticket takes the PR on its own branch when nothing names it.
#[test]
fn a_ticket_takes_its_own_branch_s_pr() {
    let mut prs = fixture_prs();
    prs[0].head_ref = "mac-ux".into();
    prs[0].title = "Mac fixes".into();
    let reads = Reads { known: true, prs };
    let s = line_stage(&ticket(LineStage::Reviewing, "mac-ux", 40), &open_cards(&[40]), &reads).unwrap();
    assert_eq!(s.pr_number, 101);
}

/// A ticket carried over from a lane of several cards shares that branch, so
/// the branch says nothing of it alone: a sibling's PR on it is not its own.
#[test]
fn a_shared_branch_does_not_give_a_ticket_a_sibling_s_pr() {
    let mut prs = fixture_prs();
    // Card 8's PR is on the shared branch; card 40 has none.
    prs.iter_mut().find(|p| p.status.number == 108).unwrap().head_ref = "mac-ux".into();
    let reads = Reads { known: true, prs };
    let mut line = ticket(LineStage::Building, "mac-ux", 40);
    line.line.same_branch = Some(Uuid::from_u128(0x2000));
    assert_eq!(stage_of(&line, &open_cards(&[8, 40]), &reads), Some(Kind::Building), "its own PR is not yet open");
    assert!(pr_of_line(&line, &open_cards(&[8, 40]), &reads).is_none());
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
    let s = line_stage(&ticket(LineStage::Landed, "x", 10), &open_cards(&[10]), &Reads { known: true, prs });
    assert_eq!(s.unwrap().label(), "Merged");
}

#[test]
fn a_title_claims_a_card_only_when_it_opens_with_the_key() {
    assert!(title_starts_with("ov-312: tickets show their stage", "ov-312"));
    assert!(title_starts_with("[ov-312] tickets", "ov-312"));
    assert!(title_starts_with("OV-312 tickets", "ov-312"));
    assert!(title_starts_with("ov-312", "ov-312"));
    assert!(!title_starts_with("ov-3120: other", "ov-312"));
    assert!(!title_starts_with("ov-31: other", "ov-312"));
    assert!(!title_starts_with("Tickets show their stage (ov-312)", "ov-312"));
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

    // A ticket in review: cannot ask is Unknown; asked and none is silence.
    let review = ticket(LineStage::Reviewing, "team", 1);
    assert_eq!(stage_of(&review, &cards, &unheard), Some(Kind::Unknown));
    assert_eq!(stage_of(&review, &cards, &none), None);

    // A building ticket: cannot ask is silence (no PR is expected yet); asked
    // and none is Building.
    let building = ticket(LineStage::Building, "team", 1);
    assert_eq!(stage_of(&building, &cards, &unheard), None);
    assert_eq!(stage_of(&building, &cards, &none), Some(Kind::Building));

    // The two never share an answer on a ticket in landing either.
    let landing = ticket(LineStage::Landing, "team", 1);
    assert_eq!(line_stage(&landing, &cards, &unheard).unwrap().label(), "PR state unknown");
    assert_eq!(line_stage(&landing, &cards, &none), None);
}

#[test]
fn a_cache_that_never_answered_is_not_an_empty_list() {
    // `Reads::of` keeps the two apart through `pr_answer_is_known`; a default
    // read is "could not ask".
    assert!(!Reads::default().known);
}

#[test]
fn a_ticket_not_started_has_no_stage() {
    assert_eq!(line_stage(&ticket(LineStage::Waiting, "team", 8), &open_cards(&[8]), &heard()), None);
}

#[test]
fn a_landed_ticket_shows_its_merged_pr() {
    let s = line_stage(&ticket(LineStage::Landed, "team", 10), &open_cards(&[10]), &heard());
    assert_eq!(s.unwrap().label(), "Merged");
    // And with nothing known it says nothing rather than Unknown.
    assert_eq!(line_stage(&ticket(LineStage::Landed, "team", 10), &open_cards(&[10]), &Reads::default()), None);
}

#[test]
fn the_wire_carries_the_words_and_the_facts() {
    let s = line_stage(&ticket(LineStage::Reviewing, "team", 3), &open_cards(&[3]), &heard()).unwrap();
    let wire = pb_stage(&s);
    assert_eq!(wire.kind, pb::PrStageKind::WaitingOnReviewer as i32);
    assert_eq!((wire.label.as_str(), wire.reviewers.as_slice(), wire.pr_number), ("Waiting on alice", &["alice".to_string()][..], 103));
    assert_eq!(wire.pr_url, "https://github.example/o/r/pull/103");
}

/// Only GitHub's own revert form is noise: `Revert "..."` or a `revert-<n>-`
/// branch. A card's own PR titled "Revert the inbox color" is the card's.
#[test]
fn only_githubs_revert_form_is_a_revert() {
    let mut prs = fixture_prs();
    let at = prs.iter().position(|p| p.status.number == 108).unwrap();
    prs[at].title = "ov-8: Revert the inbox color".into();
    assert_eq!(pr_of(&prs, "ov-8", "", false).unwrap().status.number, 108);
    prs[at].title = "Revert the inbox color".into();
    prs[at].head_ref = "ov-8/inbox".into();
    assert_eq!(pr_of(&prs, "ov-8", "", false).unwrap().status.number, 108, "head names the card");
    // And the real forms are noise.
    let mut revert = prs[0].clone();
    revert.title = "Revert \"ov-8: x\"".into();
    revert.head_ref = "someone/ov-8".into();
    assert!(is_noise(&revert));
    revert.title = "ov-8: undo".into();
    revert.head_ref = "revert-108-ov-8-x".into();
    assert!(is_noise(&revert));
    revert.head_ref = "revert-the-color-ov-8".into();
    assert!(!is_noise(&revert), "a branch that only starts with the word");
}

/// An update that leaves a ticket's stage alone, or moves it out of the live
/// stages, does not enter one.
#[test]
fn only_a_move_into_review_or_landing_enters() {
    use LineStage::*;
    assert!(enters_active(Building, Some(Reviewing)));
    assert!(enters_active(NeedsYou, Some(Reviewing)));
    assert!(enters_active(Reviewing, Some(Landing)));
    assert!(!enters_active(Reviewing, Some(Reviewing)), "the stage it already had");
    assert!(!enters_active(Reviewing, None), "no stage at all");
    assert!(!enters_active(Reviewing, Some(Landed)));
    assert!(!enters_active(Waiting, Some(Building)));
}

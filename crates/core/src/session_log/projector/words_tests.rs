//! The words a row carries (ov-1062): what the apps used to ask for, one call each.

use super::fixtures::*;
use super::rows::*;
use super::words::{agent_type, is_notice, short};

#[test]
fn the_words_are_said_once() {
    assert_eq!(agent_type("general-purpose"), "General purpose");
    assert_eq!(agent_type("web_search"), "Web search");
    assert_eq!(agent_type(""), "Agent");
    assert_eq!(agent_type("éclair_x"), "Éclair x");
    assert_eq!([short(-5), short(0), short(42_000), short(65_000), short(3_723_000), short(7_384_000)], ["0:00", "0:00", "0:42", "1:05", "1:02:03", "2:03:04"]);
    assert!(is_notice(TurnOrigin::Notification) && is_notice(TurnOrigin::System));
    assert!(!is_notice(TurnOrigin::Typed) && !is_notice(TurnOrigin::Queued) && !is_notice(TurnOrigin::Scheduled));
}

#[test]
fn a_subagent_row_carries_its_type_words_on_the_wire() {
    let p = fold(QUEUED_NOTIFICATION);
    let row = p.row("sub:toolu_bgq").unwrap();
    let RowKind::Subagent(sub) = &row.kind else { panic!("a subagent") };
    assert_eq!(sub.type_words, agent_type(&sub.agent_type));
    let wire = serde_json::to_value(row).unwrap();
    assert_eq!(wire["kind"]["Subagent"]["type_words"], sub.type_words);
}

#[test]
fn a_turn_row_says_whether_it_is_a_notice_and_how_long_it_took() {
    let p = fold(META_PROMPT);
    let wire = serde_json::to_value(p.row("turn:p1").unwrap()).unwrap();
    assert_eq!(wire["kind"]["Turn"]["took"], "0:02");
    assert!(wire["kind"]["Turn"].get("notice").is_none(), "false is left off the wire");
    let notices: Vec<_> = turns(&fold(HARNESS_TURNS)).iter().map(|(_, t)| (t.origin, t.notice)).collect();
    assert!(notices.iter().any(|&(_, notice)| notice), "{notices:?}");
    assert!(notices.iter().all(|&(origin, notice)| notice == is_notice(origin)), "{notices:?}");
    let wire = serde_json::to_value(turns(&fold(HARNESS_TURNS)).iter().find(|(_, t)| t.notice).unwrap().0).unwrap();
    assert_eq!(wire["kind"]["Turn"]["notice"], true);
}

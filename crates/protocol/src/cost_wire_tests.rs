//! The wire for cost on the plan (ov-307): experimental, additive, removable
//! with the layer. Budgets ride the two updates that exist, so the capability
//! owns no method.

use prost::Message;
use prost_types::FileDescriptorSet;

use crate::capability;
use crate::method::Method;
use crate::v1;

fn number(m: &str, f: &str) -> i32 {
    let set = FileDescriptorSet::decode(&include_bytes!(concat!(env!("OUT_DIR"), "/farcooler_descriptor.bin"))[..])
        .expect("the build writes a descriptor");
    let file = set.file.into_iter().find(|f| f.package() == "farcooler.v1").expect("farcooler.v1");
    file.message_type
        .iter()
        .find(|x| x.name() == m)
        .and_then(|x| x.field.iter().find(|x| x.name() == f))
        .unwrap_or_else(|| panic!("{m} has no field {f}"))
        .number()
}

/// The new fields take the next free tags and never move.
#[test]
fn cost_holds_its_tags() {
    assert_eq!(number("BoardThemeView", "spend"), 4, "the field before them");
    assert_eq!(number("BoardThemeView", "budget_tokens"), 5);
    assert_eq!(number("BoardThemeView", "trend_tokens"), 6);
    assert_eq!(number("Lane", "stale"), 21);
    assert_eq!(number("Lane", "budget_tokens"), 22);
    assert_eq!(number("Plan", "board_counts"), 10);
    assert_eq!(number("Plan", "cost"), 11);
    assert_eq!(number("BoardThemeUpdate", "budget_tokens"), 10);
    assert_eq!(number("LaneUpdate", "budget_tokens"), 11);
}

/// Advertised, and owning no method: nothing but the two updates carries it.
#[test]
fn cost_is_an_advertised_capability_with_no_method_of_its_own() {
    assert_eq!(capability::BOARD_COST, "board_cost");
    assert!(capability::ALL.contains(&capability::BOARD_COST), "the daemon would not advertise it");
    assert_eq!(Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_COST).count(), 0);
}

/// An app built before cost reads a plan that carries it as the plan it always
/// read: the new fields are skipped, nothing else moves.
#[test]
fn a_plan_with_cost_reads_the_same_to_an_older_app() {
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldPlan {
        #[prost(int64, tag = "1")]
        now_ms: i64,
    }
    let plan = v1::Plan {
        now_ms: 42,
        cost: Some(v1::PlanCost {
            week_tokens: 9,
            compare: vec![v1::HarnessModelCost { harness: "claude".into(), cards: 3, ..Default::default() }],
            compare_held_back: 1,
        }),
        themes: vec![v1::BoardThemeView { budget_tokens: Some(5), trend_tokens: vec![1; 7], ..Default::default() }],
        ..Default::default()
    };
    let old = OldPlan::decode(plan.encode_to_vec().as_slice()).expect("an older app decodes it");
    assert_eq!(old, OldPlan { now_ms: 42 });
    let back = v1::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!((back.cost.unwrap().compare[0].cards, back.themes[0].trend_tokens.len()), (3, 7));
}

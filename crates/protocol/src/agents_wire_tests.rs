//! The wire for the agents on the plan (ov-503): one repeated field on
//! `Plan`, behind `board_agents`, which owns no method.

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

/// The field takes the next free tag on `Plan` and never moves.
#[test]
fn the_agents_hold_their_tag() {
    assert_eq!(number("Plan", "cost"), 11, "the field before it");
    assert_eq!(number("Plan", "agents"), 12);
    assert_eq!(number("PlanAgent", "spend"), 16);
}

/// Advertised, and owning no method: only `plan.get` carries it.
#[test]
fn agents_is_an_advertised_capability_with_no_method_of_its_own() {
    assert_eq!(capability::BOARD_AGENTS_LIST, "board_agents");
    assert!(capability::ALL.contains(&capability::BOARD_AGENTS_LIST), "the daemon would not advertise it");
    assert_eq!(Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_AGENTS_LIST).count(), 0);
}

/// An app built before the agents reads a plan that carries them as the plan
/// it always read, and a newer one reads every field back.
#[test]
fn a_plan_with_agents_reads_the_same_to_an_older_app() {
    #[derive(Clone, PartialEq, prost::Message)]
    struct OldPlan {
        #[prost(int64, tag = "1")]
        now_ms: i64,
        #[prost(message, optional, tag = "11")]
        cost: Option<v1::PlanCost>,
    }
    let agent = v1::PlanAgent {
        id: "agent:a1".into(),
        role: v1::PlanAgentRole::Review as i32,
        harness: "claude".into(),
        model: "claude-opus-5".into(),
        label: "Design ov-503".into(),
        lane_id: Some(bytes::Bytes::from_static(&[1; 16])),
        lane_name: "team-dir".into(),
        working: true,
        started_at: 5,
        last_at: 9,
        spend: Some(v1::LaneSpend { input_tokens: 7, cost_micros: Some(3), ..Default::default() }),
        ..Default::default()
    };
    let plan = v1::Plan { now_ms: 42, agents: vec![agent.clone()], ..Default::default() };
    let old = OldPlan::decode(plan.encode_to_vec().as_slice()).expect("an older app decodes it");
    assert_eq!(old, OldPlan { now_ms: 42, cost: None });
    assert_eq!(v1::Plan::decode(plan.encode_to_vec().as_slice()).unwrap().agents, [agent]);
}

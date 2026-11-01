//! The wire for the line, agents and landings (ov-872): the fields `Plan`
//! carries from 16 on, and the methods `board_line` owns, in their own block.

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

const LINE_METHODS: [&str; 7] =
    ["line.set", "line.order", "agent.record", "agent.work", "agent.end", "landing.start", "landing.set"];

/// The plan's new fields, and the requests and results, hold their tags: a
/// block of their own from 600, past every lane and train tag, which stay
/// reserved.
#[test]
fn the_line_holds_its_tags() {
    for (i, name) in ["lines", "landings", "agents", "agents_spend", "suggested_reviewers"].iter().enumerate() {
        assert_eq!(number("Plan", name), 16 + i as i32, "{name}");
    }
    let requests = ["line_set", "line_order", "agent_record", "agent_work", "agent_end", "landing_start", "landing_set"];
    for (i, name) in requests.iter().enumerate() {
        assert_eq!(number("Request", name), 600 + i as i32, "{name}");
    }
    for (i, name) in ["ticket_line", "agent", "landing"].iter().enumerate() {
        assert_eq!(number("Result", name), 600 + i as i32, "{name}");
    }
    assert_eq!(number("TerminalCreate", "agent"), 7);
    assert_eq!(number("PrReady", "task_id"), 3);
}

/// One advertised capability owns every method of the line, and the lane and
/// train words are retired, never advertised again.
#[test]
fn the_line_is_one_advertised_capability_owning_every_method() {
    assert_eq!(capability::BOARD_LINE, "board_line");
    assert!(capability::ALL.contains(&capability::BOARD_LINE), "the daemon would not advertise it");
    for method in LINE_METHODS {
        assert_eq!(capability::for_method(method), Some(capability::BOARD_LINE), "{method}");
    }
    let owned = Method::ALL.iter().filter(|m| m.capability() == capability::BOARD_LINE).count();
    assert_eq!(owned, LINE_METHODS.len(), "a method the line owns that this test does not name");
    for retired in ["board_trains", "board_train_phase", "board_agents", "terminal_lane"] {
        assert!(capability::RETIRED.contains(&retired), "{retired}");
    }
    for method in ["lane.create", "lane.update", "lane.cards", "lane.agent", "plan.set", "train.start", "train.set"] {
        assert!(capability::RETIRED_METHODS.contains(&method), "{method}");
        assert_eq!(capability::for_method(method), None, "{method} is no method now");
    }
}

/// A plan carrying the line reads back whole: its tickets, agents with their
/// tickets and what they are doing now, and landings.
#[test]
fn a_plan_with_the_line_reads_back_whole() {
    let task = bytes::Bytes::from_static(&[1; 16]);
    let agent = v1::Agent {
        id: bytes::Bytes::from_static(&[2; 16]),
        role: v1::AgentRole::Reviewer as i32,
        harness: "claude".into(),
        model: "claude-opus-5".into(),
        title: "Review ov-872".into(),
        subagent_id: "a1".into(),
        tickets: vec![v1::AgentTicket { task_id: task.clone(), role: v1::WorkRole::Review as i32, started_at: 3 }],
        working: true,
        doing_now: "Reading crates/store/src/landings.rs".into(),
        last_at: 9,
        spend: Some(v1::Spend { input_tokens: 7, cost_micros: Some(3), ..Default::default() }),
        ..Default::default()
    };
    let line = v1::TicketLine { task_id: task.clone(), stage: v1::TicketStage::Reviewing as i32, landing: Some(4), ..Default::default() };
    let landing = v1::Landing { number: 4, task_ids: vec![task], phase: v1::LandingPhase::GatingLocally as i32, ..Default::default() };
    let plan = v1::Plan { now_ms: 42, lines: vec![line.clone()], agents: vec![agent.clone()], landings: vec![landing.clone()], ..Default::default() };
    let back = v1::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert_eq!((back.lines, back.agents, back.landings), (vec![line], vec![agent], vec![landing]));
}

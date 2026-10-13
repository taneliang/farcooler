//! `farcooler plan agents` (ov-503), against a runner that answers `plan.get`
//! with a plan carrying agents.

use farcooler_transport::ClientError;
use uuid::Uuid;

use super::*;
use crate::id_bytes;

const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;

fn spend(input: u64, micros: Option<i64>) -> Option<pb::LaneSpend> {
    Some(pb::LaneSpend { input_tokens: input, cost_micros: micros, runs: 1, ..Default::default() })
}

fn agent(id: &str, role: pb::PlanAgentRole, working: bool, last_ago: i64, spend: Option<pb::LaneSpend>) -> pb::PlanAgent {
    pb::PlanAgent {
        id: id.into(),
        role: role as i32,
        harness: "claude".into(),
        model: "opus".into(),
        working,
        started_at: NOW - 4 * 60 * MINUTE,
        last_at: NOW - last_ago,
        spend,
        ..Default::default()
    }
}

/// Five agents: an orchestrator, a lane's builder, a train's integrator, a
/// card's subagent with a description, and a subagent nothing names.
fn the_agents() -> Vec<pb::PlanAgent> {
    vec![
        agent("pane:o", pb::PlanAgentRole::Orchestrator, true, 0, spend(2_100_000, Some(14_200_000))),
        pb::PlanAgent { lane_id: Some(id_bytes(Uuid::from_u128(7))), lane_name: "hero-shot".into(), ..agent("pane:b", pb::PlanAgentRole::Build, true, MINUTE, spend(380_000, Some(1_100_000))) },
        pb::PlanAgent { train_id: Some(id_bytes(Uuid::from_u128(8))), train_title: "Train 86".into(), ..agent("agent:i", pb::PlanAgentRole::Integrator, true, 2 * MINUTE, spend(1_400_000, Some(6_300_000))) },
        pb::PlanAgent { task_id: Some(id_bytes(Uuid::from_u128(9))), task_key: "ov-12".into(), label: "Design ov-503".into(), ..agent("agent:c", pb::PlanAgentRole::Subagent, false, 60 * MINUTE, spend(220_000, Some(600_000))) },
        agent("agent:ghost", pb::PlanAgentRole::Subagent, false, 120 * MINUTE, spend(10_000, None)),
    ]
}

struct Link {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
}

fn link() -> Link {
    Link { capabilities: [capability::BOARD_PLAN, capability::BOARD_AGENTS_LIST].map(String::from).to_vec(), sent: vec![] }
}

impl DispatchLink for Link {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        self.sent.push(req);
        Ok(pb::Result {
            value: Some(pb::result::Value::Plan(pb::Plan { now_ms: NOW, agents: the_agents(), ..Default::default() })),
        })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

/// The list reads in the order the runner sent, says what each works for, and
/// puts tokens before dollars; the one with no record is still there.
#[tokio::test]
async fn the_agents_are_listed_by_what_they_work_for() {
    let mut l = link();
    let text = agents(&mut l, id_bytes(Uuid::from_u128(1)), false).await.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "3 working · 5 agents in the last 24 hours · 4.1M tokens", "{text}");
    assert_eq!(lines[1], "  \u{25cf} Orchestrator · opus  The plan  working  2.1M tokens · about $14.20 API-equivalent");
    assert!(lines[2].contains("Builder · opus  Lane hero-shot  working  380K tokens"), "{text}");
    assert!(lines[3].contains("Integrator · opus  Train 86"), "{text}");
    assert_eq!(
        lines[4],
        "  \u{25cb} Subagent · opus \u{201c}Design ov-503\u{201d}  Card ov-12  done 1h ago  220K tokens · about $0.60 API-equivalent"
    );
    assert_eq!(lines[5], "  \u{25cb} Subagent · opus  Not on a lane  done 2h ago  10K tokens", "unpriced shows tokens only");
    assert!(!lines[0].contains('$'), "a total with an unpriced agent in it shows no dollars: {}", lines[0]);
    let r = &l.sent[0];
    assert_eq!(r.method, "plan.get");
}

/// `--json` carries every agent, with ids and the join, null where nothing joins.
#[tokio::test]
async fn the_json_names_the_join() {
    let mut l = link();
    let json: Value = serde_json::from_str(&agents(&mut l, id_bytes(Uuid::from_u128(1)), true).await.unwrap()).unwrap();
    let list = json["agents"].as_array().unwrap();
    assert_eq!(list.len(), 5);
    assert_eq!((list[0]["role"].as_str(), list[0]["working"].as_bool()), (Some("orchestrator"), Some(true)));
    assert_eq!(list[1]["lane"]["name"], "hero-shot");
    assert_eq!(list[2]["train"]["title"], "Train 86");
    assert_eq!((list[3]["card"]["key"].as_str(), list[3]["label"].as_str()), (Some("ov-12"), Some("Design ov-503")));
    assert!(list[4]["lane"].is_null() && list[4]["train"].is_null() && list[4]["card"].is_null());
    assert_eq!(list[0]["spend"]["input_tokens"], 2_100_000);
    assert!(list[4]["spend"]["cost_micros"].is_null());
}

/// A runner with the plan and no agents list is told before anything is sent.
#[tokio::test]
async fn a_runner_without_the_list_is_told_before_anything_is_sent() {
    let mut l = link();
    l.capabilities.retain(|c| c != capability::BOARD_AGENTS_LIST);
    let err = agents(&mut l, id_bytes(Uuid::from_u128(1)), false).await.unwrap_err();
    assert_eq!(err.to_string(), "This runner needs an update to list agents.");
    assert!(l.sent.is_empty());
}

/// The verb parses under `plan`, and an empty day says so.
#[test]
fn the_verb_parses_and_an_empty_day_says_so() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        plan: super::super::PlanArgs,
    }
    let cli = Cli::try_parse_from(["plan", "agents"]).expect("`plan agents` parses");
    assert!(matches!(cli.plan.cmd, Some(super::super::PlanCmd::Agents)));
    assert_eq!(agents_text(&pb::Plan::default()), "No agents have worked in the last 24 hours.");
}

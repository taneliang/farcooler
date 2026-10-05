use super::*;
use prost::Message;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

fn id(n: u128) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(Uuid::from_u128(n).as_bytes())
}

fn lane(n: u128, name: &str, state: pb::LaneState, cards: &[u128]) -> pb::Lane {
    pb::Lane {
        id: id(0x2000 + n),
        name: name.into(),
        state: state as i32,
        state_since: NOW - 10 * 60_000,
        cards: cards.iter().map(|c| pb::LaneCard { task_id: id(0x1000 + c), slice: String::new() }).collect(),
        spend: Some(Default::default()),
        ..Default::default()
    }
}

/// The plan the CLI's own fixture test builds (`plan_json_is_the_shape_the_mac_reads`),
/// so the same file holds both.
fn the_plan() -> pb::Plan {
    let mut queued = lane(1, "mac-fu3", pb::LaneState::Queued, &[2]);
    queued.plan_rank = Some(1);
    queued.reason = "Frees the Mac slot".into();
    let mut review = lane(2, "mac-ux", pb::LaneState::Review, &[1]);
    review.train = Some("integ-9".into());
    review.reason = "Five small Mac fixes, batched".into();
    review.worktree_path = ".claude/worktrees/mac-ux".into();
    review.branch = "mac-ux".into();
    review.harness = "claude".into();
    review.model = "opus".into();
    review.fix_rounds = 1;
    review.cards[0].slice = "Mac".into();
    review.spend = Some(pb::LaneSpend {
        input_tokens: 300_000,
        output_tokens: 170_000,
        cost_micros: Some(31_000_000),
        runs: 2,
        unmeasured_agents: 1,
        shared_agents: 1,
        ..Default::default()
    });
    review.agents = vec![
        pb::LaneAgent {
            harness: "claude".into(),
            agent_id: "a1".into(),
            role: pb::LaneAgentRole::Build as i32,
            model: "opus".into(),
            started_at: NOW - 3 * HOUR,
            ended_at: Some(NOW - HOUR),
        },
        pb::LaneAgent {
            harness: "claude".into(),
            agent_id: "a2".into(),
            role: pb::LaneAgentRole::Review as i32,
            model: "sonnet".into(),
            started_at: NOW - 25 * 60_000,
            ended_at: None,
        },
    ];
    let mut landed = lane(3, "fix-ac84", pb::LaneState::Landed, &[3]);
    landed.state_since = NOW - HOUR;
    landed.landed_sha = Some("4d3c8cb1e2".into());
    let task = |n: u128, key: &str, status: pb::TaskStatus| pb::PlanCard {
        task_id: id(0x1000 + n),
        key: key.into(),
        title: format!("Title of {key}"),
        status: status as i32,
    };
    pb::Plan {
        now_ms: NOW,
        themes: vec![pb::BoardThemeView {
            theme: Some(pb::BoardTheme {
                id: id(0x3001),
                name: "Visual language".into(),
                state: pb::BoardThemeState::Active as i32,
                outcome: "Every Mac surface reads as one app.".into(),
                story: "Tokens are on main.".into(),
                story_at: NOW - HOUR,
                next: "Frosted window plane".into(),
                owner_ask: "Should the sidebar tint follow the terminal theme?".into(),
                ..Default::default()
            }),
            task_ids: vec![id(0x1001), id(0x1002), id(0x1003)],
            counts: Some(pb::PlanStatusCounts { done: 1, backlog: 2, ..Default::default() }),
            spend: Some(pb::LaneSpend {
                input_tokens: 235_000,
                output_tokens: 85_000,
                cost_micros: Some(15_500_000),
                runs: 2,
                ..Default::default()
            }),
        }],
        order: vec![queued.id.clone()],
        lanes: vec![queued, review, landed],
        cards: vec![
            task(1, "ov-1", pb::TaskStatus::InProgress),
            task(2, "ov-2", pb::TaskStatus::Backlog),
            task(3, "ov-3", pb::TaskStatus::Done),
        ],
        coverage: vec![
            pb::PlanCoverage { task_id: id(0x1001), live: 1, landed: 0 },
            pb::PlanCoverage { task_id: id(0x1002), live: 1, landed: 0 },
            pb::PlanCoverage { task_id: id(0x1003), live: 0, landed: 1 },
        ],
        // The CLI test's two rulings (ov-304): R-2 standing, R-1 confirmed.
        rulings: vec![
            pb::BoardRuling {
                id: id(0x5002),
                workspace_id: id(0x0202),
                number: 2,
                decision: "The inbox is amber.".into(),
                why: "It's the one attention color, so the inbox reads as needing you.".into(),
                reversal: "One token; every surface follows.".into(),
                task_ids: vec![id(0x1001)],
                task_keys: vec!["ov-1".into()],
                theme_id: Some(id(0x3001)),
                state: pb::BoardRulingState::Standing as i32,
                actor: "manager".into(),
                created_at: NOW - HOUR,
                resource_version: 1,
                ..Default::default()
            },
            pb::BoardRuling {
                id: id(0x5001),
                workspace_id: id(0x0202),
                number: 1,
                decision: "Unread stays on the phones.".into(),
                why: "The owner reads there first.".into(),
                reversal: "A setting and two screens.".into(),
                state: pb::BoardRulingState::Confirmed as i32,
                note: "Keep it.".into(),
                actor: "manager".into(),
                created_at: NOW - 5 * HOUR,
                settled_by: Some("manager".into()),
                settled_at: Some(NOW - 2 * HOUR),
                resource_version: 2,
                ..Default::default()
            },
        ],
        trains: trains().0,
        ci: trains().1,
        board_counts: Some(pb::PlanStatusCounts { backlog: 4, in_progress: 2, in_review: 3, done: 11, ..Default::default() }),
    }
}

/// A train (ov-309): integ-9, red, carrying mac-ux, and the runner's CI read
/// of its SHA. The client's `plan_json` test builds the same, so one fixture
/// holds both.
fn trains() -> (Vec<pb::BoardTrain>, Vec<pb::BoardCiRead>) {
    let job = |name: &str, state: &str| pb::BoardCiJob { name: name.into(), state: state.into(), url: String::new() };
    let train = pb::BoardTrain {
        id: id(0x6001),
        workspace_id: id(0x0202),
        name: "integ-9".into(),
        base: "origin/main".into(),
        pushed_sha: Some("c85bf83d".into()),
        state: pb::BoardTrainState::Red as i32,
        state_since: NOW - 20 * 60_000,
        actor: "manager".into(),
        created_at: NOW - 2 * HOUR,
        landed_at: None,
        lane_ids: vec![id(0x2002)],
        ci_subject: "sha:c85bf83d".into(),
        resource_version: 3,
    };
    let read = pb::BoardCiRead {
        subject: "sha:c85bf83d".into(),
        sha: "c85bf83dce46a6b71d7312afc623899ae7914658".into(),
        status: pb::BoardCiStatus::Failed as i32,
        url: "https://github.com/taneliang/farcooler/actions/runs/37275435256".into(),
        jobs: vec![job("CI / Swift (shared + macOS)", "failed"), job("CI / Android", "passed"), job("Canary", "passed")],
        fetched_at: NOW - 60_000,
        changed_at: NOW - 20 * 60_000,
        asked_at: NOW - 60_000,
    };
    (vec![train], vec![read])
}

fn fixture() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/plan.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("test/fixtures/plan.json is missing")).unwrap()
}

/// **The wire reads into the fixture the apps decode.** The plan is encoded to
/// protobuf bytes and decoded again, as the runner's answer reaches this
/// client, and what `plan_json` makes of it is `test/fixtures/plan.json`: the
/// file the CLI's test holds `farcooler plan --json` to, and that AgentKit's and
/// Android's decoders read. A key renamed here, or a field dropped, goes red.
#[test]
fn the_wire_reads_into_the_fixture_the_apps_decode() {
    let bytes = the_plan().encode_to_vec();
    let over_the_wire = pb::Plan::decode(bytes.as_slice()).unwrap();
    assert_eq!(plan_json(&over_the_wire), fixture());
}

/// A record's events keep their order and their `extra`; an `extra` that
/// isn't JSON is null rather than a failed read.
#[test]
fn a_record_reads_as_events() {
    let list = pb::PlanEventList {
        events: vec![
            pb::PlanEvent {
                at: 5,
                actor: "manager".into(),
                kind: "story".into(),
                body: "Before.".into(),
                extra_json: r#"{"to":"After."}"#.into(),
                ..Default::default()
            },
            pb::PlanEvent { at: 9, kind: "cards".into(), body: "Added ov-1.".into(), extra_json: "nope".into(), ..Default::default() },
        ],
    };
    let wire = pb::PlanEventList::decode(list.encode_to_vec().as_slice()).unwrap();
    let json = events_json(&wire);
    assert_eq!(json["events"][0]["extra"]["to"], "After.");
    assert_eq!(json["events"][0]["body"], "Before.");
    assert_eq!(json["events"][1]["extra"], Value::Null);
    assert_eq!(json["events"][1]["at"], 9);
}

/// A card the plan doesn't list is named by its short id, not left blank.
#[test]
fn an_unlisted_card_is_named_by_its_short_id() {
    let mut plan = the_plan();
    plan.cards.clear();
    let json = plan_json(&plan);
    assert_eq!(json["themes"][0]["cards"][0]["key"], "00001001");
}

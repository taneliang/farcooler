use super::*;
use prost::Message;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

fn id(n: u128) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(Uuid::from_u128(n).as_bytes())
}

fn line(n: u128, stage: pb::TicketStage) -> pb::TicketLine {
    pb::TicketLine {
        task_id: id(0x1000 + n),
        stage: stage as i32,
        stage_since: NOW - 10 * 60_000,
        created_at: NOW - 5 * HOUR,
        resource_version: 1,
        spend: Some(Default::default()),
        ..Default::default()
    }
}

fn work(agent: u128, task: u128, role: pb::WorkRole, started_at: i64) -> pb::TicketWork {
    pb::TicketWork { agent_id: id(0x7000 + agent), task_id: id(0x1000 + task), role: role as i32, started_at, ..Default::default() }
}

/// The plan the CLI's own fixture test builds, so the same file holds both: a
/// ticket waiting next up, one in review after a fix round, one landing on
/// Landing 9 (its CI red), and one landed; an orchestrator, a builder working
/// two tickets and a reviewer.
fn the_plan() -> pb::Plan {
    let mut waiting = line(2, pb::TicketStage::Waiting);
    waiting.plan_rank = Some(1);
    waiting.reason = "Frees the Mac slot".into();
    let mut review = line(1, pb::TicketStage::Reviewing);
    review.reason = "Five small Mac fixes, batched".into();
    review.worktree_path = ".claude/worktrees/ov-1".into();
    review.branch = "ov-1-mac-polish".into();
    review.fix_rounds = 1;
    review.spend = Some(pb::Spend {
        input_tokens: 300_000,
        output_tokens: 170_000,
        cost_micros: Some(31_000_000),
        runs: 2,
        unmeasured_agents: 1,
        shared_agents: 1,
        ..Default::default()
    });
    review.budget_tokens = Some(500_000);
    review.work = vec![
        pb::TicketWork { ended_at: Some(NOW - HOUR), verdict: "finished".into(), ..work(2, 1, pb::WorkRole::Build, NOW - 3 * HOUR) },
        work(3, 1, pb::WorkRole::Review, NOW - 25 * 60_000),
    ];
    let mut landing = line(4, pb::TicketStage::Landing);
    landing.branch = "ov-4-glance".into();
    landing.stack_parent = Some(id(0x1001));
    landing.landing = Some(9);
    landing.pr_number = Some(31);
    landing.work = vec![work(2, 4, pb::WorkRole::Build, NOW - 2 * HOUR)];
    let mut landed = line(3, pb::TicketStage::Landed);
    landed.stage_since = NOW - HOUR;
    landed.landed_sha = Some("4d3c8cb1e2".into());
    let task = |n: u128, key: &str, status: pb::TaskStatus| pb::PlanCard {
        task_id: id(0x1000 + n),
        key: key.into(),
        title: format!("Title of {key}"),
        status: status as i32,
    };
    let agent = |n: u128, role: pb::AgentRole| pb::Agent {
        id: id(0x7000 + n),
        role: role as i32,
        harness: "claude".into(),
        started_at: NOW - 4 * HOUR,
        ..Default::default()
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
            spend: Some(pb::Spend {
                input_tokens: 235_000,
                output_tokens: 85_000,
                cost_micros: Some(15_500_000),
                runs: 2,
                ..Default::default()
            }),
            budget_tokens: Some(250_000),
            trend_tokens: vec![0, 0, 0, 40_000, 120_000, 0, 160_000],
            last_moved_at: Some(NOW - HOUR),
        }],
        lines: vec![waiting, review, landing, landed],
        cards: vec![
            task(1, "ov-1", pb::TaskStatus::InReview),
            task(2, "ov-2", pb::TaskStatus::Backlog),
            task(3, "ov-3", pb::TaskStatus::InProgress),
            task(4, "ov-4", pb::TaskStatus::InProgress),
            task(5, "ov-5", pb::TaskStatus::InProgress),
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
        landings: landings().0,
        ci: landings().1,
        board_counts: Some(pb::PlanStatusCounts { backlog: 4, in_progress: 2, in_review: 3, done: 11, ..Default::default() }),
        cost: Some(pb::PlanCost {
            week_tokens: 34_200_000,
            compare: vec![
                pb::HarnessModelCost {
                    harness: "claude".into(),
                    model: "opus".into(),
                    card_share_milli: 5_000,
                    tokens: 7_500_000,
                    cost_micros: Some(12_500_000),
                },
                pb::HarnessModelCost {
                    harness: "codex".into(),
                    model: "gpt-5.6".into(),
                    card_share_milli: 3_400,
                    tokens: 1_020_000,
                    cost_micros: None,
                },
            ],
            compare_held_back: 2,
            in_flight_tokens: 1_200_000,
            in_flight_cost_micros: None,
            week: vec![
                pb::WeekSpend { harness: "claude".into(), model: "opus".into(), tokens: 28_000_000, cost_micros: Some(41_200_000) },
                pb::WeekSpend { harness: "codex".into(), model: "gpt-5.6".into(), tokens: 6_200_000, cost_micros: Some(3_800_000) },
            ],
            week_cost_micros: Some(45_000_000),
            week_unsplit_cache_write_tokens: 0,
        }),
        agents: vec![
            pb::Agent {
                model: "opus".into(),
                terminal_id: Some(id(0x8001)),
                working: true,
                doing_now: "2/5 · Reviewing the plan".into(),
                last_at: NOW,
                spend: Some(pb::Spend {
                    input_tokens: 2_000_000, output_tokens: 100_000, cost_micros: Some(14_200_000), runs: 4, ..Default::default()
                }),
                ..agent(1, pb::AgentRole::Orchestrator)
            },
            pb::Agent {
                model: "sonnet".into(),
                title: "Review ov-1".into(),
                subagent_id: "a3".into(),
                parent_terminal_id: Some(id(0x8001)),
                started_at: NOW - 25 * 60_000,
                tickets: vec![pb::AgentTicket { task_id: id(0x1001), role: pb::WorkRole::Review as i32, started_at: NOW - 25 * 60_000 }],
                working: true,
                doing_now: "Reading PlanColumn.swift".into(),
                last_at: NOW - 5 * 60_000,
                spend: Some(pb::Spend { input_tokens: 200_000, runs: 1, ..Default::default() }),
                ..agent(3, pb::AgentRole::Reviewer)
            },
            pb::Agent {
                harness: "codex".into(),
                model: "gpt-5.6".into(),
                title: "Build the glance".into(),
                terminal_id: Some(id(0x8002)),
                stop_asked_at: Some(NOW - 60_000),
                tickets: vec![pb::AgentTicket { task_id: id(0x1004), role: pb::WorkRole::Build as i32, started_at: NOW - 2 * HOUR }],
                working: false,
                last_at: NOW - 2 * 60_000,
                spend: Some(pb::Spend { unmeasured_agents: 1, ..Default::default() }),
                ..agent(2, pb::AgentRole::Builder)
            },
        ],
        // What the day cost, three agents that are gone included.
        agents_spend: Some(pb::Spend {
            input_tokens: 2_900_000, output_tokens: 100_000, cost_micros: Some(20_000_000), runs: 9, ..Default::default()
        }),
        review_waits: vec![],
        suggested_reviewers: vec![],
    }
}

/// Landing 9 (ov-309, ov-872): red, carrying ov-4, with a lander, and the
/// runner's CI read of its SHA; and Landing 8, landed with ov-3.
fn landings() -> (Vec<pb::Landing>, Vec<pb::BoardCiRead>) {
    let job = |name: &str, state: &str| pb::BoardCiJob { name: name.into(), state: state.into(), url: String::new() };
    let red = pb::Landing {
        id: id(0x6001),
        workspace_id: id(0x0202),
        number: 9,
        route: pb::LandingRoute::Push as i32,
        base: "origin/main".into(),
        pushed_sha: Some("c85bf83d".into()),
        state: pb::LandingState::Red as i32,
        state_since: NOW - 20 * 60_000,
        lander_id: Some(id(0x7004)),
        collecting: false,
        actor: "runner".into(),
        created_at: NOW - 2 * HOUR,
        landed_at: None,
        resource_version: 3,
        task_ids: vec![id(0x1004)],
        ci_subject: "sha:c85bf83d".into(),
        title: "Landing 9".into(),
        summary: "Title of ov-4".into(),
        phase: pb::LandingPhase::CiFailed as i32,
        phase_since: NOW - 20 * 60_000,
        spend: Some(pb::Spend { input_tokens: 90_000, output_tokens: 30_000, cost_micros: Some(4_000_000), runs: 1, ..Default::default() }),
    };
    let landed = pb::Landing {
        id: id(0x6002),
        number: 8,
        route: pb::LandingRoute::Push as i32,
        pushed_sha: Some("4d3c8cb1e2".into()),
        state: pb::LandingState::Landed as i32,
        state_since: NOW - HOUR,
        landed_at: Some(NOW - HOUR),
        actor: "runner".into(),
        task_ids: vec![id(0x1003)],
        ci_subject: "sha:4d3c8cb1e2".into(),
        title: "Agents message the plan".into(),
        summary: "Title of ov-3".into(),
        phase: pb::LandingPhase::Landed as i32,
        phase_since: NOW - HOUR,
        ..Default::default()
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
    (vec![red, landed], vec![read])
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
///
/// `FARCOOLER_WRITE_FIXTURES=1` writes the file from the plan first, for a
/// change to the shape that is meant.
#[test]
fn the_wire_reads_into_the_fixture_the_apps_decode() {
    let bytes = the_plan().encode_to_vec();
    let over_the_wire = pb::Plan::decode(bytes.as_slice()).unwrap();
    let json = plan_json(&over_the_wire);
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/plan.json");
        std::fs::write(path, serde_json::to_string_pretty(&json).unwrap() + "\n").unwrap();
    }
    assert_eq!(json, fixture());
}

/// The reconciliation's flags: an open card whose ticket landed, and a card in
/// progress that isn't on the line.
#[test]
fn the_flags_name_a_landed_open_card_and_one_off_the_line() {
    let json = plan_json(&the_plan());
    let keys = |flag: &str| json[flag].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(keys("landed_not_closed"), ["ov-3"]);
    assert_eq!(keys("off_the_line"), ["ov-5"]);
}

/// A day with no spend still arrives as an object: clients tell a runner that lists only the agents that
/// exist from an older one by the field's presence (ov-518), so a zero must not read as absent.
#[test]
fn a_day_with_no_spend_still_sends_the_object() {
    let mut plan = the_plan();
    plan.agents_spend = Some(pb::Spend::default());
    let over_the_wire = pb::Plan::decode(plan.encode_to_vec().as_slice()).unwrap();
    assert!(over_the_wire.agents_spend.is_some(), "prost dropped an empty submessage");
    assert!(plan_json(&over_the_wire)["agents_spend"].is_object());
    plan.agents_spend = None;
    assert!(plan_json(&plan)["agents_spend"].is_null(), "an older runner sends none");
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

/// A ticket's pull request stage (ov-312) reaches the phones' JSON, and a
/// ticket the runner said nothing of keeps the shape the fixture holds, with no
/// `pr_stage`.
#[test]
fn a_ticket_s_pr_stage_reaches_the_json_and_is_left_out_when_unsaid() {
    let mut plan = the_plan();
    let at = plan.lines.iter().position(|l| l.task_id == id(0x1004)).unwrap();
    plan.lines[at].pr_stage = Some(pb::PrStage {
        kind: pb::PrStageKind::Queued as i32,
        label: "Queued, Position 2".into(),
        queue_position: 2,
        pr_number: 31,
        ..Default::default()
    });
    let json = plan_json(&plan);
    assert_eq!(json["lines"][at]["pr_stage"]["label"], "Queued, Position 2");
    assert_eq!(json["lines"][at]["pr_stage"]["kind"], "queued");
    assert!(json["lines"][(at + 1) % plan.lines.len()].get("pr_stage").is_none());
}

/// The waits appear in the JSON only when the runner sent some (ov-579), so the
/// fixture the Mac decodes keeps its shape.
#[test]
fn review_waits_appear_only_when_sent_and_an_absent_median_is_null() {
    let mut plan = pb::Plan::default();
    assert!(plan_json(&plan).get("review_waits").is_none());
    plan.review_waits = vec![pb::ReviewerWait { reviewer: "alice".into(), prs: 2, median_wait_ms: None, samples: 0 }];
    let json = plan_json(&plan);
    assert_eq!(json["review_waits"][0]["prs"], 2);
    assert!(json["review_waits"][0]["median_wait_ms"].is_null());
}

#[test]
fn suggested_reviewers_appear_only_when_sent() {
    let mut plan = pb::Plan::default();
    assert!(plan_json(&plan).get("suggested_reviewers").is_none());
    plan.suggested_reviewers = vec![pb::TicketReviewers { task_id: id(0x1001), pr: 31, reviewers: vec!["alice".into()], sources: vec!["CODEOWNERS".into()] }];
    let json = plan_json(&plan);
    assert_eq!((json["suggested_reviewers"][0]["pr"].as_u64(), json["suggested_reviewers"][0]["reviewers"][0].as_str()), (Some(31), Some("alice")));
}

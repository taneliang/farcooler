//! The report's spend over a seeded store (ov-195): turns recorded as ov-194
//! records them, read back through `report.get` as a client gets it.
//!
//! The period is 2026-09-01 to 2026-09-03, UTC. The tasks are filed today,
//! after it, so the board alone would leave every one of them out: only
//! their agents' turns bring them in.

use farcooler_core::usage::{TokenCounts, estimate};
use farcooler_protocol::v1 as pb;
use farcooler_store::models::Actor;
use farcooler_store::usage::{NewTurn, Surface, TurnKind, TurnModel};
use uuid::Uuid;

use super::*;

const SINCE: i64 = 1_788_220_800_000; // 2026-09-01T00:00Z
const UNTIL: i64 = 1_788_393_600_000; // 2026-09-03T00:00Z
const SEP_1_10H: i64 = 1_788_256_800_000;
const SEP_1_23H: i64 = 1_788_303_600_000;
const SEP_2_15H: i64 = 1_788_361_200_000;
const AUG_31_23H: i64 = 1_788_217_200_000;

fn tokens(input: u64, output: u64, cache_read: u64) -> TokenCounts {
    TokenCounts { input, output, cache_read, ..Default::default() }
}

struct Turn<'a> {
    key: &'a str,
    task: Option<Uuid>,
    workspace: Uuid,
    harness: &'a str,
    ended_at: i64,
    active_ms: i64,
    models: Vec<TurnModel>,
}

fn record(store: &farcooler_store::Store, repository: Uuid, t: Turn) {
    let turn = NewTurn {
        key: t.key.into(),
        terminal_id: None,
        worktree_id: None,
        repository_id: Some(repository),
        workspace_id: Some(t.workspace),
        task_id: t.task,
        harness: t.harness.into(),
        surface: Surface::Chat,
        started_at: Some(t.ended_at - t.active_ms),
        ended_at: t.ended_at,
        active_ms: Some(t.active_ms),
        usage: "reported",
        models: t.models,
        kind: TurnKind::Turn,
    };
    assert!(store.record_turn(&turn).unwrap());
}

fn ask(workspace: Option<Uuid>, utc_offset_minutes: i32) -> pb::ReportRequest {
    pb::ReportRequest {
        since: SINCE,
        until: UNTIL,
        repository_id: None,
        workspace_id: workspace.map(|w| bytes::Bytes::copy_from_slice(w.as_bytes())),
        utc_offset_minutes,
    }
}

fn report(store: &farcooler_store::Store, req: &pb::ReportRequest) -> Report {
    let wire = serve(store, req, UNTIL + 1).unwrap();
    serde_json::from_str(&wire.report_json).unwrap()
}

#[tokio::test]
async fn the_report_counts_what_agents_spent_by_task_harness_model_and_day() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let store = &svc.store;
    let main = store.ensure_main_workspace(repo).unwrap();
    let other = store.create_workspace(repo, "billing", "bil").unwrap();
    let docs = store.create_task(main.id, "Docs: the spend section", Actor::User).unwrap();
    let ship = store.create_task(main.id, "CLI: ship it", Actor::User).unwrap();
    let elsewhere = store.create_task(other.id, "Billing: invoices", Actor::User).unwrap();

    // docs: two claude turns, one reported ($0.50), one estimated.
    let estimated = tokens(1_000, 200, 10_000);
    let estimated_micros = estimate(Some("claude-opus-5"), &estimated).unwrap();
    record(store, repo, Turn {
        key: "claude:1", task: Some(docs.id), workspace: main.id, harness: "claude", ended_at: SEP_1_10H,
        active_ms: 600_000,
        models: vec![TurnModel::priced(Some("claude-opus-5".into()), tokens(100, 50, 2_000), Some(500_000))],
    });
    record(store, repo, Turn {
        key: "claude-log:2", task: Some(docs.id), workspace: main.id, harness: "claude", ended_at: SEP_1_23H,
        active_ms: 300_000,
        models: vec![TurnModel::priced(Some("claude-opus-5".into()), estimated, None)],
    });
    // ship: a codex turn nobody can price.
    record(store, repo, Turn {
        key: "codex:3", task: Some(ship.id), workspace: main.id, harness: "codex", ended_at: SEP_2_15H,
        active_ms: 3_600_000,
        models: vec![TurnModel::priced(Some("gpt-5.5".into()), tokens(40_000, 2_000, 0), None)],
    });
    // A turn on no task, in the workspace.
    record(store, repo, Turn {
        key: "claude:4", task: None, workspace: main.id, harness: "claude", ended_at: SEP_2_15H,
        active_ms: 60_000,
        models: vec![TurnModel::priced(Some("claude-opus-5".into()), tokens(10, 10, 0), Some(20_000))],
    });
    // Before the period, and in another workspace: neither counts below.
    record(store, repo, Turn {
        key: "claude:5", task: Some(docs.id), workspace: main.id, harness: "claude", ended_at: AUG_31_23H,
        active_ms: 1, models: vec![TurnModel::priced(Some("claude-opus-5".into()), tokens(9, 9, 9), Some(9))],
    });
    record(store, repo, Turn {
        key: "claude:6", task: Some(elsewhere.id), workspace: other.id, harness: "claude", ended_at: SEP_1_10H,
        active_ms: 1, models: vec![TurnModel::priced(Some("claude-opus-5".into()), tokens(7, 7, 7), Some(7))],
    });

    let r = report(store, &ask(Some(main.id), 0));
    let spend = r.spend.as_ref().expect("the period has spend");

    let t = &spend.total;
    assert_eq!(t.turns, 4);
    assert_eq!(t.active_ms, 600_000 + 300_000 + 3_600_000 + 60_000);
    assert_eq!(t.input_tokens, 100 + 1_000 + 40_000 + 10);
    assert_eq!(t.cache_read_tokens, 2_000 + 10_000);
    assert_eq!(t.cost_reported_micros, 500_000 + 20_000);
    assert_eq!(t.cost_estimated_micros, estimated_micros);
    assert_eq!(t.unpriced_tokens, 42_000);
    assert!(t.cost_line().ends_with("· API-equivalent, partly estimated, partly not reported"), "{}", t.cost_line());

    // By task: the most dollars first; turns on no task are their own line.
    let names: Vec<&str> = spend.by_task.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec![docs.key.as_str(), "No task", ship.key.as_str()]);
    assert_eq!(spend.by_task[0].title.as_deref(), Some("Docs: the spend section"));
    assert_eq!(spend.by_task[0].spend.turns, 2);
    assert_eq!(spend.by_task[2].spend.cost_line(), "Not reported");
    assert_eq!(spend.other_tasks, 0);

    let by = |lines: &[spend::SpendLine]| -> Vec<(String, u64)> {
        lines.iter().map(|l| (l.name.clone(), l.spend.total_tokens())).collect()
    };
    assert_eq!(
        by(&spend.by_harness),
        vec![("claude".into(), 2_150 + 11_200 + 20), ("codex".into(), 42_000)],
        "claude spent dollars; codex's are unknown, so it comes second"
    );
    assert_eq!(by(&spend.by_model), vec![("claude-opus-5".into(), 13_370), ("gpt-5.5".into(), 42_000)]);
    assert_eq!(spend.period_unit, "day");
    assert_eq!(by(&spend.by_period), vec![("2026-09-01".into(), 13_350), ("2026-09-02".into(), 42_020)]);

    // Each task's share reaches the tallies: the totals, and its area's.
    let usage = r.totals.usage.expect("the tallies carry it");
    assert_eq!(usage.turns, Some(3), "the turn on no task is in the spend, not on the board");
    assert_eq!(usage.cost_reported_micros, Some(500_000));
    assert_eq!(usage.cost_estimated_micros, Some(estimated_micros));
    assert_eq!(usage.unpriced_tokens, Some(42_000));
    let docs_area = r.by_area.iter().find(|g| g.name == "Docs").expect("docs is in, by its spend alone");
    assert_eq!(docs_area.tally.usage.and_then(|u| u.agent_ms), Some(900_000));

    // Days are the client's: two hours east, 23:00Z on the 1st is the 2nd.
    let east = report(store, &ask(Some(main.id), 120));
    let days = by(&east.spend.unwrap().by_period);
    assert_eq!(days, vec![("2026-09-01".into(), 2_150), ("2026-09-02".into(), 11_200 + 42_020)]);
}

#[tokio::test]
async fn a_period_with_no_turns_has_no_spend() {
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let main = svc.store.ensure_main_workspace(repo).unwrap();
    let r = report(&svc.store, &ask(Some(main.id), 0));
    assert_eq!(r.spend, None);
    assert_eq!(r.totals.usage, None);
    let json: serde_json::Value = serde_json::from_str(&serve(&svc.store, &ask(None, 0), UNTIL).unwrap().report_json).unwrap();
    assert_eq!(json["spend"], serde_json::Value::Null);
}

#[test]
fn a_longer_period_is_cut_into_weeks_then_months() {
    const DAY: i64 = 86_400_000;
    let unit = |days: i64| spend::period_unit(Period { since: 0, until: days * DAY });
    assert_eq!(unit(14), farcooler_store::usage::GroupBy::Day);
    assert_eq!(unit(15), farcooler_store::usage::GroupBy::Week);
    assert_eq!(unit(92), farcooler_store::usage::GroupBy::Week);
    assert_eq!(unit(93), farcooler_store::usage::GroupBy::Month);
}

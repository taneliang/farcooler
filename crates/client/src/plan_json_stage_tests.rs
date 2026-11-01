//! The pull request stage on the plan's JSON, in the bytes the Mac decodes
//! (ov-316): `test/fixtures/plan-stages.json` is this output, and AgentKit's
//! `PlanPrStageTests` decode the same file through the real decoder.

use super::*;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000_000;

fn id(n: u128) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(Uuid::from_u128(n).as_bytes())
}

fn stage(kind: pb::PrStageKind, label: &str) -> pb::PrStage {
    pb::PrStage { kind: kind as i32, label: label.into(), checks: pb::CheckState::Unknown as i32, ..Default::default() }
}

/// One ticket in review per stage kind (`ov-N`), with the facts each kind
/// carries, and one that was never told a stage.
fn the_plan() -> pb::Plan {
    use pb::PrStageKind as K;
    let mut waiting = stage(K::WaitingOnReviewer, "Waiting on alice and 2 others");
    waiting.reviewers = vec!["alice".into(), "bob".into(), "org/web".into()];
    waiting.pr_number = 31;
    waiting.pr_url = "https://github.com/acme/app/pull/31".into();
    waiting.unresolved_threads = Some(2);
    let mut long = stage(K::WaitingOnReviewer, "Waiting on maximilian-wolfgang-hohenzollern-schwarzenberg");
    long.reviewers = vec!["maximilian-wolfgang-hohenzollern-schwarzenberg".into()];
    long.pr_number = 32;
    let mut queued = stage(K::Queued, "Queued, position 2");
    queued.queue_position = 2;
    let mut failing = stage(K::ApprovedChecksFailing, "Approved \u{b7} checks failing");
    failing.checks = pb::CheckState::Failing as i32;
    failing.pr_number = 33;
    let stages = vec![
        stage(K::Building, "Building"),
        stage(K::AgentReview, "Agent review"),
        stage(K::Fixing, "Fixing"),
        waiting,
        long,
        stage(K::ChangesRequested, "Changes requested"),
        stage(K::ApprovedChecksRunning, "Approved \u{b7} checks running"),
        failing,
        stage(K::ApprovedConflicts, "Approved \u{b7} conflicts"),
        stage(K::Approved, "Approved"),
        queued,
        stage(K::Merged, "Merged"),
        stage(K::Closed, "Closed"),
        stage(K::Unknown, "PR state unknown"),
    ];
    let mut plan = pb::Plan { now_ms: NOW, ..Default::default() };
    for (i, s) in stages.into_iter().enumerate() {
        let n = i as u128 + 1;
        plan.cards.push(pb::PlanCard { task_id: id(0x1000 + n), key: format!("ov-{n}"), title: format!("Card {n}"), ..Default::default() });
        plan.lines.push(pb::TicketLine {
            task_id: id(0x1000 + n),
            stage: pb::TicketStage::Reviewing as i32,
            stage_since: NOW - 600_000,
            pr_stage: Some(s),
            spend: Some(Default::default()),
            ..Default::default()
        });
    }
    // A runner that said nothing: no `pr_stage` key at all.
    plan.cards.push(pb::PlanCard { task_id: id(0x1100), key: "ov-256".into(), title: "Silent".into(), ..Default::default() });
    plan.lines.push(pb::TicketLine {
        task_id: id(0x1100),
        stage: pb::TicketStage::Reviewing as i32,
        stage_since: NOW - 600_000,
        spend: Some(Default::default()),
        ..Default::default()
    });
    plan
}

/// **The stage reaches the Mac in the shape its decoder reads.** Rewrite the
/// file with `FARCOOLER_WRITE_FIXTURES=1`; the Mac's tests then decode it.
#[test]
fn plan_stages_json_is_the_shape_the_mac_reads() {
    let out = serde_json::to_string_pretty(&plan_json(&the_plan())).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/plan-stages.json");
    if std::env::var_os("FARCOOLER_WRITE_FIXTURES").is_some() {
        std::fs::write(&path, &out).unwrap();
    }
    let fixture = std::fs::read_to_string(&path).expect("test/fixtures/plan-stages.json is missing");
    assert_eq!(out, fixture, "the plan's stages no longer match test/fixtures/plan-stages.json, which the Mac decodes");
}

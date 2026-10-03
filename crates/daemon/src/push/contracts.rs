//! The runner's `/v1/notify`, `/v1/heartbeat` and `/v1/notify/retire` bodies
//! against the shared contract fixtures in `test/fixtures/contracts/notify/`
//! and `runner/`, which the relay's suite posts to its own
//! route (see `test/fixtures/contracts/README.md`).
//!
//! The tests in `push.rs` assert this side's spelling, and the relay's suite asserts
//! its own. Neither fails when only one side renames a key. This one does:
//! every fixture must be exactly what `wire_body` writes, so a renamed or
//! dropped key here fails here, and a fixture changed to match fails the relay.
//!
//! `FARCOOLER_WRITE_CONTRACTS=1` rewrites the fixtures from this code instead
//! of comparing. Only for a deliberate change, reviewed in the diff.

use super::*;
use farcooler_core::trace::{Sample, Trace};

/// The moment every fixture is sampled at: 2026-10-03 09:30:00 UTC, in
/// seconds. The relay's suite pins its clock to the same instant, so a
/// trace anchor and an ask's end are valid there.
const NOW_S: i64 = 1_791_019_800;
const NOW_MS: i64 = NOW_S * 1000;
/// A real runner's install id is a UUIDv7.
const INSTALL: &str = "0199a8f2-4c1e-7b3a-9d0e-5f6a7b8c9d0e";
/// The `version` the fixtures carry. The build stamp differs per build, so
/// it is checked for being this build's and then compared as this.
const VERSION: &str = "2026.10.3-canary.412+d35970c9";

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/contracts/notify")
}

/// What a run that has been going 23 minutes looks like: output and code
/// in most buckets, one commit.
fn trace() -> Trace {
    let mut trace = Trace::new();
    for minute in (0..23).rev() {
        let at = NOW_S - minute * 60;
        trace.record(at, Sample::output(40 + (minute as u32 % 5) * 12));
        if minute % 3 == 0 {
            trace.record(at, Sample::code(9 + minute as u32));
        }
    }
    trace.record(NOW_S - 4 * 60, Sample::commits(1));
    trace
}

fn written() -> Vec<(&'static str, serde_json::Value)> {
    let runner = crate::service::stable_host_id(INSTALL).to_string();
    let notice_id = format!("t:{runner}:ov-90");
    let options = vec!["pdfkit".to_string(), "pdf.js".to_string()];
    let trace = trace();
    let bytes = trace.encode(NOW_S);
    let ask = WireAsk::new(
        "hook-ask-0199a8f3-1b2c-7d4e-8f50-6a7b8c9d0e1f",
        Some("Bash"),
        std::time::UNIX_EPOCH + std::time::Duration::from_millis((NOW_MS + 9 * 60 * 1000) as u64),
    )
    .expect("a valid ask");
    let started_at = Some(NOW_MS - 23 * 60 * 1000);

    let outgoing: Vec<(&'static str, Outgoing)> = vec![
        ("agent-working", Outgoing {
            title: "claude",
            subtitle: "3/7 · Designing test matrix",
            status: "working",
            label: "claude",
            terminal: Some("term-01999a8f2c4e"),
            workspace: Some("Billing"),
            needs_you: Some(1),
            install: Some(INSTALL),
            started_at,
            insertions: Some(142),
            deletions: Some(37),
            commits: Some(1),
            trace: &bytes,
            trace_anchor: trace.anchor(NOW_S),
            ..Outgoing::default()
        }),
        ("agent-blocked", Outgoing {
            title: "claude needs you",
            subtitle: "auth-refactor — Do you want to run git push --force-with-lease?",
            status: "blocked",
            label: "claude",
            terminal: Some("term-01999a8f2c4e"),
            workspace: Some("Billing"),
            needs_you: Some(2),
            install: Some(INSTALL),
            ask: Some(&ask),
            started_at,
            insertions: Some(142),
            deletions: Some(37),
            commits: Some(1),
            trace: &bytes,
            trace_anchor: trace.anchor(NOW_S),
            ..Outgoing::default()
        }),
        ("agent-blocked-quiet", Outgoing {
            title: "codex needs you",
            subtitle: "ov-90 — Waiting for your answer",
            status: "blocked",
            label: "codex",
            terminal: Some("term-01999a90aa10"),
            needs_you: Some(2),
            install: Some(INSTALL),
            started_at: Some(NOW_MS - 5 * 60 * 1000),
            alert: false,
            ..Outgoing::default()
        }),
        ("agent-done-failed", Outgoing {
            title: "codex failed",
            subtitle: "pdf-export — Its last turn didn’t finish",
            status: "done",
            failed: true,
            label: "codex",
            terminal: Some("term-01999a90aa10"),
            needs_you: Some(0),
            install: Some(INSTALL),
            insertions: Some(12),
            deletions: Some(0),
            commits: Some(0),
            ..Outgoing::default()
        }),
        ("ask", Outgoing {
            kind: Some("ask"),
            terminal: Some("term-01999a8f2c4e"),
            needs_you: Some(2),
            install: Some(INSTALL),
            ask: Some(&ask),
            ..Outgoing::default()
        }),
        ("count", Outgoing { kind: Some("count"), needs_you: Some(0), install: Some(INSTALL), ..Outgoing::default() }),
        ("decision-legacy", Outgoing {
            kind: Some("decision"),
            title: "ov-90 Pick a PDF library",
            subtitle: "Needs your decision · Which PDF library should export use?",
            task: Some("ov-90"),
            workspace: Some("Billing"),
            needs_you: Some(1),
            install: Some(INSTALL),
            notice_id: Some(&notice_id),
            event: Some("decision"),
            level: Some("time-sensitive"),
            options: &options,
            ..Outgoing::default()
        }),
        ("task-decision", Outgoing {
            kind: Some("task"),
            title: "ov-90 Pick a PDF library",
            subtitle: "Needs your decision · Which PDF library should export use?",
            task: Some("ov-90"),
            workspace: Some("Billing"),
            needs_you: Some(1),
            install: Some(INSTALL),
            notice_id: Some(&notice_id),
            event: Some("decision"),
            level: Some("time-sensitive"),
            options: &options,
            ..Outgoing::default()
        }),
        ("task-review", Outgoing {
            kind: Some("task"),
            title: "ov-90 Pick a PDF library",
            subtitle: "Moved to In Review · 3 files changed",
            task: Some("ov-90"),
            workspace: Some("Billing"),
            needs_you: Some(1),
            install: Some(INSTALL),
            notice_id: Some(&notice_id),
            event: Some("review"),
            level: Some("active"),
            // A review never carries buttons, whatever the caller hands in.
            options: &options,
            ..Outgoing::default()
        }),
    ];
    outgoing
        .into_iter()
        .map(|(name, o)| {
            let mut body = serde_json::to_value(wire_body(&o).expect("a body")).expect("serialize");
            assert_eq!(body["version"], farcooler_protocol::BUILD, "{name}: the version is the build stamp");
            body["version"] = VERSION.into();
            (name, body)
        })
        .collect()
}

/// Whether to rewrite the fixtures rather than compare. Refused under CI:
/// a producer that rewrote its fixture there would pass by definition.
fn rewriting() -> bool {
    let asked = std::env::var_os("FARCOOLER_WRITE_CONTRACTS").is_some();
    assert!(
        !(asked && std::env::var("CI").is_ok_and(|v| v == "true")),
        "FARCOOLER_WRITE_CONTRACTS is set under CI, which would make this test pass by rewriting its fixtures"
    );
    asked
}

/// Every fixture in `dir` is exactly what `written` holds, and no other
/// fixture is there, which nothing would keep in step.
fn compare(dir: &std::path::Path, written: &[(&'static str, serde_json::Value)], producer: &str) {
    if rewriting() {
        for (name, body) in written {
            let text = serde_json::to_string_pretty(body).expect("pretty") + "\n";
            std::fs::write(dir.join(format!("{name}.json")), text).expect("write a fixture");
        }
    }
    for (name, body) in written {
        let path = dir.join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&text).expect("the fixture is JSON");
        assert_eq!(
            &fixture,
            body,
            "{} is not what {producer} writes. If the change is deliberate, rerun with \
             FARCOOLER_WRITE_CONTRACTS=1 and the relay's suite will say whether it still reads it. \
             Written:\n{}",
            path.display(),
            serde_json::to_string_pretty(body).unwrap_or_default()
        );
    }
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .expect("the fixtures")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json"))
        .collect();
    on_disk.sort();
    let mut names: Vec<String> = written.iter().map(|(n, _)| format!("{n}.json")).collect();
    names.sort();
    assert_eq!(on_disk, names, "a fixture in {} with no producer", dir.display());
}

#[test]
fn every_notify_fixture_is_what_wire_body_writes() {
    compare(&dir(), &written(), "wire_body");
}

/// The runner's other three bodies: its heartbeat, its withdrawal on
/// unpairing, and the cards it retires. The relay's suite posts each, and
/// reads the heartbeat back out of `/v1/pulse` into the pulse fixture.
#[test]
fn every_runner_fixture_is_what_the_runner_sends() {
    let mut beat = serde_json::to_value(beat_body(Some(INSTALL))).expect("serialize");
    assert_eq!(beat["version"], farcooler_protocol::BUILD, "the version is the build stamp");
    assert_eq!(beat["name"], runner_name(), "the name is this computer's");
    beat["version"] = VERSION.into();
    beat["name"] = "Studio".into();
    let terminals = ["term-01999a8f2c4e".to_string(), "term-01999a90aa10".to_string()];
    let retire = serde_json::to_value(Retirement { terminals: &terminals }).expect("serialize");
    let written = [("heartbeat", beat), ("withdraw", withdraw_body(Some(INSTALL))), ("retire", retire)];
    let runner = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/contracts/runner");
    compare(&runner, &written, "the runner");
}

//! Changing the process (docs/engine-pipeline.md 7): a publish applies at once to queued, held and working
//! tokens; a removed station retires with only its translation; crossed work never re-enters its boundary.
#[path = "pipeline_support/mod.rs"]
mod support;
use std::collections::BTreeMap;
use farcooler_workflow::pipeline::*;
use serde_json::{json, Value};
use support::*;

fn adapters() -> Value {
    json!({ "files": { "version": "test/files", "config": {} }, "cms": { "version": "test/cms", "config": {} } })
}
/// research → outline → write → check → publish (outward, once) → measure → archive.
fn before() -> Value {
    json!({ "schema": 3, "name": "articles", "adapters": adapters(), "intake": [ { "op": "files.board_column", "to": "research" } ],
        "stations": [
            { "id": "research", "kind": "Agent", "op": "files.research", "pool": "agents" },
            { "id": "outline", "kind": "Agent", "op": "files.outline", "pool": "agents" },
            { "id": "write", "kind": "Agent", "op": "files.write", "pool": "agents" },
            { "id": "check", "kind": "Agent", "op": "files.fact_check", "pool": "agents", "routes": { "rejected": "write" } },
            { "id": "publish", "kind": "Command", "op": "cms.publish" },
            { "id": "measure", "kind": "Wait", "op": "cms.stats", "routes": { "measured": "archive" } },
            { "id": "archive", "kind": "Command", "op": "files.archive" } ] })
}
/// `outline` renamed `plan`; `check` replaced by `review`, a station the publish adds.
fn after_review() -> Value {
    let mut v = before();
    v["stations"][1]["id"] = json!("plan");
    v["stations"][3] = json!({ "id": "review", "kind": "Agent", "op": "files.fact_check", "pool": "agents", "routes": { "rejected": "plan" } });
    v
}
/// `publish` replaced by `publish2`, another operation on the same boundary `cms:issue`.
fn after_mirror() -> Value {
    let mut v = before();
    v["stations"][4] = json!({ "id": "publish2", "kind": "Command", "op": "cms.publish_mirror" });
    v
}
fn publish(r: &mut Run, layout: Value, moves: &[(&str, Move)], after: &[(&str, After)]) -> Result<Vec<Effect>, Refusal> {
    r.orch(Command::Publish { layout: support::layout(layout),
        moves: moves.iter().map(|(k, m)| (k.to_string(), m.clone())).collect(),
        after: after.iter().map(|(k, a)| (k.to_string(), a.clone())).collect(), reason: "change the process".into() })
}
fn through(r: &mut Run, key: &str, stations: usize) {
    r.intake(key, None, 0);
    for _ in 0..stations { r.answer(&tok(key), "passed"); }
}

#[test]
fn a_publish_applies_at_once_to_queued_held_and_working_tokens() {
    let mut r = Run::new(&[("agents", 8)], before());
    through(&mut r, "w", 3);
    through(&mut r, "w2", 3);
    assert_eq!(r.station("w").as_deref(), Some("check"));
    through(&mut r, "h", 2);
    r.orch(Command::Hold { subject: tok("h"), station: "write".into(), reason: "wait for the source".into() }).unwrap();
    r.stopped(&tok("h"));
    r.orch(Command::HoldStation { station: "outline".into(), reason: "pause".into() }).unwrap();
    through(&mut r, "q", 1);
    assert_eq!(r.queue("outline"), vec!["q"]);

    let check = Move::Replace { to: "review".into(), labels: BTreeMap::from([("rejected".to_string(), "write".to_string())]), boundary: None };
    // A removed station with work and no move would strand it.
    assert_eq!(publish(&mut r, after_review(), &[("outline", Move::Rename { to: "plan".into() })], &[]), Err(Refusal::Stranded));
    // A rename keeps kind and op; anything else is a replacement.
    let mut wrong = after_review();
    wrong["stations"][1]["op"] = json!("files.research");
    assert_eq!(publish(&mut r, wrong, &[("outline", Move::Rename { to: "plan".into() }), ("check", check.clone())], &[]),
        Err(Refusal::Definition));

    publish(&mut r, after_review(), &[("outline", Move::Rename { to: "plan".into() }), ("check", check)], &[]).unwrap();
    // Queued at a renamed station: re-keyed in place, its history too, its station's hold kept.
    assert_eq!(r.queue("plan"), vec!["q"]);
    assert!(r.p.tokens["q"].visits.iter().all(|v| v.station != "outline"));
    assert!(r.p.stations["plan"].held.is_some());
    // Held at a station that still exists: unchanged.
    assert!(matches!(r.at("h"), Location::Held { station, .. } if station == "write"));
    // Working at the removed station: the step is not interrupted; the station retires with its translation.
    assert!(matches!(r.at("w"), Location::Working { station, .. } if station == "check"));
    let retired = &r.p.retired["check"];
    assert_eq!((retired.to.as_str(), retired.working.len()), ("review", 2));
    assert!(!r.p.stations.contains_key("check"));
    // Its `passed` arrives at `to` and runs the new station; it doesn't jump past it.
    r.answer(&tok("w"), "passed");
    assert!(matches!(r.at("w"), Location::Working { station, .. } if station == "review"));
    // A listed label follows the translation; the empty retired station is deleted.
    r.answer(&tok("w2"), "rejected");
    assert_eq!(r.station("w2").as_deref(), Some("write"));
    assert!(r.p.retired.is_empty());
    // The new layout's routes apply from here on: review's `rejected` now goes to plan.
    r.answer(&tok("w"), "rejected");
    assert_eq!(r.station("w").as_deref(), Some("plan"));
    r.orch(Command::ResumeStation { station: "plan".into() }).unwrap();
    assert!(matches!(r.at("q"), Location::Working { station, .. } if station == "plan"));
    r.assert_replays();
}

#[test]
fn crossed_work_resumes_after_its_boundary_never_at_a_replacement_on_it() {
    let mut r = Run::new(&[("agents", 8)], before());
    through(&mut r, "x", 4);
    assert!(matches!(r.at("x"), Location::Working { station, .. } if station == "publish"));
    r.orch(Command::HoldStation { station: "publish".into(), reason: "pause".into() }).unwrap();
    through(&mut r, "y", 4);
    assert_eq!(r.queue("publish"), vec!["y"]);
    let mv = |b: Option<BoundaryMove>| ("publish", Move::Replace { to: "publish2".into(), labels: BTreeMap::new(), boundary: b });
    // Replacing an outward station must say whether its boundary carries on.
    assert_eq!(publish(&mut r, after_mirror(), &[mv(None)], &[]), Err(Refusal::Definition));
    // `after` may not itself lead back to the boundary.
    assert_eq!(publish(&mut r, after_mirror(), &[mv(Some(BoundaryMove::Keep))], &[("cms:issue", After::Station("write".into()))]),
        Err(Refusal::Definition));
    publish(&mut r, after_mirror(), &[mv(Some(BoundaryMove::Keep))], &[("cms:issue", After::Station("measure".into()))]).unwrap();
    // y never crossed: it moves to the replacement at once.
    assert!(matches!(r.at("y"), Location::Working { station, .. } if station == "publish2"));
    // x's publish finishes in the retired station and crosses; the translation names publish2, a station on
    // the boundary it just crossed, so the migration rule sends it to `after` instead: no second publish.
    r.answer(&tok("x"), "passed");
    assert_eq!(r.p.tokens["x"].crossings.len(), 1);
    assert!(matches!(r.at("x"), Location::Working { station, .. } if station == "measure"));
    for to in ["publish2", "write", "research"] {
        assert_eq!(r.orch(Command::Send { token: "x".into(), station: "measure".into(), to: to.into(), reason: "again".into() }),
            Err(Refusal::Crossed), "{to} leads back to cms:issue");
    }
    r.answer(&tok("x"), "measured");
    r.answer(&tok("x"), "passed");
    assert_eq!(r.closed("x").closed, Closure::Done);
    r.assert_replays();
}

#[test]
fn with_no_continuation_crossed_work_holds_for_the_orchestrator() {
    let mut r = Run::new(&[("agents", 8)], before());
    through(&mut r, "x", 4);
    publish(&mut r, after_mirror(), &[("publish", Move::Replace { to: "publish2".into(), labels: BTreeMap::new(),
        boundary: Some(BoundaryMove::Keep) })], &[("cms:issue", After::Hold)]).unwrap();
    r.answer(&tok("x"), "passed");
    match r.at("x") {
        Location::Held { station, hold } => {
            assert_eq!(station, "publish2");
            assert_eq!(hold.kind, HoldKind::Crossed { boundary: "cms:issue".into() });
        }
        other => panic!("crossed work with no continuation holds: {other:?}"),
    }
    // A `Crossed` hold has only checked exits.
    assert_eq!(r.orch(Command::Release { subject: tok("x"), station: "publish2".into(), to: None, reason: None }), Err(Refusal::Crossed));
    assert_eq!(r.orch(Command::Release { subject: tok("x"), station: "publish2".into(), to: Some("write".into()), reason: None }), Err(Refusal::Crossed));
    r.orch(Command::Release { subject: tok("x"), station: "publish2".into(), to: Some("measure".into()), reason: None }).unwrap();
    assert_eq!(r.station("x").as_deref(), Some("measure"));
    r.assert_replays();
}

#[test]
fn the_same_inputs_give_the_same_state() {
    let script = |r: &mut Run| {
        through(r, "a", 2);
        through(r, "b", 4);
        r.orch(Command::Prioritize { token: "a".into(), priority: 3 }).unwrap();
        publish(r, after_mirror(), &[("publish", Move::Replace { to: "publish2".into(), labels: BTreeMap::new(),
            boundary: Some(BoundaryMove::Keep) })], &[("cms:issue", After::Station("measure".into()))]).unwrap();
        r.answer(&tok("b"), "passed");
    };
    let mut one = Run::new(&[("agents", 2)], before());
    let mut two = Run::new(&[("agents", 2)], before());
    script(&mut one);
    script(&mut two);
    assert_eq!(one.p, two.p);
    assert_eq!(one.effects, two.effects);
    one.assert_replays();
}

//! The personal-assistant (8.3) and research-and-content (8.4) walkthroughs of docs/engine-pipeline.md, end to
//! end through the pipeline reducer, against fixture adapters that know nothing of code.
#[path = "pipeline_support/mod.rs"]
mod support;
use farcooler_workflow::pipeline::*;
use support::*;

fn next_window(now: i64, every: i64, at: i64) -> i64 { ((now - at).div_euclid(every) + 1) * every + at }
fn decide(run: &mut Run, by: Option<&str>, s: &Subject, station: &str, verdict: &str, suspects: &[&str]) {
    let command = Command::Decide { subject: s.clone(), station: station.into(), verdict: verdict.into(),
        suspects: suspects.iter().map(|k| k.to_string()).collect(), reason: Some(format!("{verdict} by the decider")) };
    match by { Some(name) => run.person(name, command), None => run.orch(command) }.unwrap();
}

#[test]
fn assistant_triage_draft_approve_send_rounds_follow_up_and_digest() {
    let mut r = Run::new(&[("agents", 2), ("work", 2)], assistant());
    let m1 = tok("m1");
    r.intake("m1", None, 0);
    assert_eq!(r.station("m1").as_deref(), Some("triage"));
    assert_eq!(r.effect_for(&m1).model.as_deref(), Some("claude:sonnet"));
    // Round 1: triage `reply` → draft → approve (the person) → send, a crossing of mail:send.
    r.answer(&m1, "reply");
    r.answer(&m1, "passed");
    assert!(matches!(r.at("m1"), Location::Awaiting { station, .. } if station == "approve"));
    assert_eq!(r.orch(Command::Decide { subject: m1.clone(), station: "approve".into(), verdict: "approved".into(),
        suspects: vec![], reason: None }), Err(Refusal::Unauthorized), "only the deciders decide");
    decide(&mut r, Some("me"), &m1, "approve", "approved", &[]);
    assert_eq!(r.effect_for(&m1).idempotency, "m1:mail:send:1");
    r.answer(&m1, "passed");
    assert_eq!(r.p.tokens["m1"].crossings.len(), 1);
    assert_eq!(r.station("m1").as_deref(), Some("await-reply"));
    assert!(matches!(r.effect_for(&m1).kind, EffectKind::Probe(_)));
    // A reply arrives: round 2 begins by an ordinary route, with a fresh draft and a fresh idempotency key.
    r.answer(&m1, "replied");
    assert_eq!(r.station("m1").as_deref(), Some("triage"));
    r.answer(&m1, "reply");
    r.answer(&m1, "passed");
    decide(&mut r, Some("me"), &m1, "approve", "approved", &[]);
    assert_eq!(r.effect_for(&m1).idempotency, "m1:mail:send:2");
    r.answer(&m1, "passed");
    assert_eq!(r.p.tokens["m1"].crossings.iter().map(|c| c.round).collect::<Vec<_>>(), vec![1, 2]);
    // No reply in three days: `timeout` → follow-up → approve, which the orchestrator skips with a reason.
    r.now += 3 * DAY;
    r.advance();
    assert_eq!(r.station("m1").as_deref(), Some("follow-up"));
    r.answer(&m1, "passed");
    r.orch(Command::Skip { subject: m1.clone(), station: "approve".into(), reason: "routine follow-up".into() }).unwrap();
    let skipped = r.p.tokens["m1"].visits.iter().rev().find(|v| v.station == "approve").unwrap().clone();
    assert_eq!((skipped.ended.as_deref(), skipped.reason.as_deref()), (Some("skipped"), Some("routine follow-up")));
    assert_eq!(r.effect_for(&m1).idempotency, "m1:mail:send:3");
    r.answer(&m1, "passed");
    r.answer(&m1, "replied");
    r.answer(&m1, "ignore");
    assert_eq!(r.station("m1").as_deref(), Some("archive"));
    r.answer(&m1, "passed");
    let closed = r.closed("m1");
    assert_eq!(closed.closed, Closure::Done);
    assert_eq!(closed.crossings.iter().map(|c| (c.boundary.as_str(), c.round)).collect::<Vec<_>>(),
        vec![("mail:send", 1), ("mail:send", 2), ("mail:send", 3)]);

    // A stale draft can't go out again: sending m2 straight back to `send` after a crossing holds it.
    let m2 = tok("m2");
    r.intake("m2", None, 0);
    r.answer(&m2, "reply");
    r.answer(&m2, "passed");
    decide(&mut r, Some("me"), &m2, "approve", "approved", &[]);
    r.answer(&m2, "passed");
    r.orch(Command::Send { token: "m2".into(), station: "await-reply".into(), to: "send".into(), reason: "send it again".into() }).unwrap();
    match r.at("m2") {
        Location::Held { station, hold } => {
            assert_eq!(station, "send");
            assert!(hold.reason.contains("Evidence"), "{}", hold.reason);
        }
        other => panic!("a stale draft must hold, not send: {other:?}"),
    }
    assert_eq!(r.p.tokens["m2"].crossings.len(), 1);

    // `schedule` goes through the calendar adapter to draft: two adapters in one workflow.
    let m5 = tok("m5");
    r.intake("m5", None, 0);
    r.answer(&m5, "schedule");
    assert_eq!(r.station("m5").as_deref(), Some("propose-times"));
    r.answer(&m5, "passed");
    assert_eq!(r.station("m5").as_deref(), Some("draft"));

    // FYI threads wait at the digest until the next 08:00 window, then travel it as one batch.
    for k in ["m3", "m4"] { r.intake(k, None, 0); r.answer(&tok(k), "fyi"); }
    r.advance();
    assert_eq!(r.queue("digest"), vec!["m3", "m4"], "nothing forms before the window");
    r.now = next_window(r.now, DAY, 8 * HOUR);
    r.advance();
    let batch = Subject::Batch(r.batch_of("m3"));
    assert_eq!(r.batch_of("m4"), batch.key());
    assert_eq!(r.effect_for(&batch).members, vec!["m3", "m4"]);
    r.answer(&batch, "passed");
    assert_eq!(r.location(&batch).station(), Some("digest.deliver"));
    r.answer(&batch, "passed");
    assert!(!r.p.batches.contains_key(batch.key()), "the finished batch is archived");
    assert_eq!(r.p.archive[batch.key()].batch.as_ref().unwrap().members, vec!["m3", "m4"]);
    for k in ["m3", "m4"] {
        assert_eq!(r.station(k).as_deref(), Some("archive"));
        r.answer(&tok(k), "passed");
        assert_eq!(r.closed(k).crossings[0].boundary, "mail:digest");
    }
    r.assert_replays();
}

#[test]
fn content_research_pick_write_fact_check_and_a_weekly_issue() {
    let mut r = Run::new(&[("agents", 3), ("work", 1)], content());
    for k in ["a1", "a2", "a3"] { r.intake(k, None, 0); }
    for k in ["a1", "a2", "a3"] {
        let s = tok(k);
        r.answer(&s, "passed");
        r.answer(&s, "passed");
        assert!(matches!(r.at(k), Location::Awaiting { station, .. } if station == "pick"));
        decide(&mut r, None, &s, "pick", "approved", &[]);
        r.answer(&s, "passed");
        assert_eq!(r.station(k).as_deref(), Some("fact-check"));
    }
    // fact-check rejects a1 once: back to write with the reason, then it passes.
    r.answer(&tok("a1"), "rejected");
    assert_eq!(r.station("a1").as_deref(), Some("write"));
    r.answer(&tok("a1"), "passed");
    for k in ["a2", "a3", "a1"] { r.answer(&tok(k), "passed"); }
    assert_eq!(r.queue("issue"), vec!["a2", "a3", "a1"]);
    // Friday: the week's articles form one issue.
    r.now = next_window(r.now, 7 * DAY, 0);
    r.advance();
    let issue = Subject::Batch(r.batch_of("a1"));
    r.answer(&issue, "passed");
    r.answer(&issue, "passed");
    assert!(matches!(r.location(&issue), Location::Awaiting { station, .. } if station == "issue.editor"));
    // The editor rejects the issue, marking a2: return_suspects sends a2 back to write, the rest re-enter editor.
    decide(&mut r, Some("editor"), &issue, "issue.editor", "rejected", &["a2"]);
    assert_eq!(r.station("a2").as_deref(), Some("write"));
    assert!(r.p.tokens["a2"].brief.as_deref().unwrap().contains("returned"));
    assert_eq!(r.p.batches[issue.key()].members.iter().map(|m| m.token.as_str()).collect::<Vec<_>>(), vec!["a3", "a1"]);
    assert!(matches!(r.location(&issue), Location::Awaiting { station, .. } if station == "issue.editor"));
    decide(&mut r, None, &issue, "issue.editor", "approved", &[]);
    assert_eq!(r.effect_for(&issue).idempotency, format!("{}:cms:issue:1", issue.key()));
    r.answer(&issue, "passed");
    // Published: the crossing is on the batch and on every member; a crossed batch can't be pulled from.
    for k in ["a1", "a3"] { assert_eq!(r.p.tokens[k].crossings[0].boundary, "cms:issue"); }
    assert_eq!(r.orch(Command::Pull { token: "a1".into(), reason: "late fix".into() }), Err(Refusal::Crossed));
    r.answer(&issue, "measured");
    for k in ["a3", "a1"] {
        assert_eq!(r.station(k).as_deref(), Some("archive"));
        r.answer(&tok(k), "passed");
        let closed = r.closed(k);
        assert_eq!((closed.closed.clone(), closed.crossings.len()), (Closure::Done, 1));
    }
    // a2, rewritten, waits for next week's issue.
    r.answer(&tok("a2"), "passed");
    r.answer(&tok("a2"), "passed");
    assert_eq!(r.queue("issue"), vec!["a2"]);
    r.assert_replays();
}

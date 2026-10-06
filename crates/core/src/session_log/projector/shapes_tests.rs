//! The fold against the shapes real transcripts take, one fixture per finding
//! of the first review (`fixtures/shapes/`, synthetic).

use super::fixtures::*;
use super::rows::*;
use super::{Projection, SessionProjector};

fn queued(p: &Projection) -> Vec<(&str, QueuedState)> {
    p.rows().iter().filter_map(|r| match &r.kind { RowKind::Queued(q) => Some((q.text.as_str(), q.state)), _ => None }).collect()
}

#[test]
fn an_is_meta_prompt_with_its_own_prompt_id_is_a_turn_and_a_companion_is_not() {
    let p = fold(META_PROMPT);
    assert_eq!(turns(&p).len(), 2, "the heartbeat is a turn; the image companion is not");
    let heartbeat = turn(&p, "turn:p2");
    assert_eq!(heartbeat.origin, TurnOrigin::System);
    assert_eq!(heartbeat.duration_ms, Some(500));
    assert_eq!(turn(&p, "turn:p1").duration_ms, Some(2000), "the heartbeat's turn_duration is not the last turn's");
    let reply = prose(&p).into_iter().find(|(_, t)| t.text == "Nothing new on the board.").unwrap();
    assert_eq!(reply.0.turn.as_deref(), Some("turn:p2"));
}

#[test]
fn a_notification_absorbed_mid_turn_ends_its_background_agent() {
    let p = fold(QUEUED_NOTIFICATION);
    let sub = sub(&p, "sub:toolu_bgq");
    assert_eq!(sub.status, SubagentState::Completed);
    assert_eq!(sub.ended_ms, Some(ms("11:00:12.000")));
    assert_eq!(turn(&p, "turn:p1").background_running, 0);
    assert!(queued(&p).is_empty(), "claude's own notification is no person's message");
}

#[test]
fn a_dequeue_sends_the_message_its_prompt_was_not_the_oldest_one() {
    let mut p = Projection::new();
    for line in QUEUE.lines() {
        p.fold_line(line.as_bytes());
        if line.contains("\"promptId\":\"p2\"") {
            assert_eq!(
                queued(&p),
                [("Also update the docs.", QueuedState::Sent), ("Then run the tests.", QueuedState::Sent)],
                "the first was absorbed mid-turn, and the dequeue sent the newest, not the peer's message ahead of it"
            );
        }
    }
    assert_eq!(
        queued(&p),
        [("Also update the docs.", QueuedState::Sent), ("Then run the tests.", QueuedState::Sent), ("One more idea.", QueuedState::Withdrawn)],
        "no row for the empty enqueue or the peer's message, and popAll hands the last back"
    );
    assert_eq!(turn(&p, "turn:p3").origin, TurnOrigin::System);
}

#[test]
fn an_api_error_fails_the_turn_and_unknown_records_are_one_gap_per_turn() {
    let p = fold(ERRORS_AND_GAPS);
    let t = turn(&p, "turn:p1");
    assert_eq!(t.outcome, Some(TurnOutcome::Failed { detail: "API Error: 529 Overloaded".into() }), "a prompt quoting a command tag mid-text is still a prompt");
    assert_eq!(t.duration_ms, Some(3000));
    let said: Vec<&str> = prose(&p).iter().map(|(_, s)| s.text.as_str()).collect();
    assert_eq!(said, ["Working on it.", "Still going."]);
    let gaps: Vec<&Gap> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Gap(g) => Some(g), _ => None }).collect();
    assert_eq!(gaps, [&Gap { reason: GapReason::Unknown("x-new-record".into()), count: 3 }], "frame-link and the ledger are silent");
    let notices: Vec<&str> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Notice(n) => Some(n.text.as_str()), _ => None }).collect();
    assert_eq!(notices, ["/model"], "one notice per command, none for its output");
}

#[test]
fn a_rewrite_or_a_resume_into_a_session_already_shown_adds_no_rows() {
    let dir = Scratch::new("resume");
    let a = dir.path().join("a.jsonl");
    let b = dir.path().join("b.jsonl");
    std::fs::write(&a, EDITS).unwrap();
    std::fs::write(&b, CLEARED_AFTER).unwrap();
    let mut session = SessionProjector::open(a.clone());
    session.poll();
    session.rebind(b.clone());
    session.poll();
    let ids: Vec<String> = session.projection().rows().iter().map(|r| r.id.clone()).collect();
    // `/resume` back into A: its lines are read again from the start.
    session.rebind(a.clone());
    session.poll();
    let again: Vec<String> = session.projection().rows().iter().map(|r| r.id.clone()).collect();
    assert_eq!(again, ids, "nothing duplicated");
    // A rewrite of the same bytes under a new inode is the same story.
    let tmp = dir.path().join("a2.jsonl");
    std::fs::write(&tmp, EDITS).unwrap();
    std::fs::rename(&tmp, &a).unwrap();
    session.poll();
    let rows = session.projection().rows();
    let non_gap = rows.iter().filter(|r| !matches!(r.kind, RowKind::Gap(_))).count();
    assert_eq!(non_gap, ids.len(), "only the rewrite's own gap");
    // And re-reading A's prompts did not take the transcript back into A: a
    // reply with no prompt before it is not filed under A's turns.
    session.projection_mut().fold_line(br#"{"type":"assistant","uuid":"zz","message":{"content":[{"type":"text","text":"after"}]}}"#);
    let after = session.projection().row("prose:zz:0").unwrap();
    assert!(!matches!(after.turn.as_deref(), Some("turn:p1" | "turn:p2")), "{after:?}");
}

#[test]
fn a_subagents_own_tool_hooks_stay_out_of_the_main_turn() {
    let mut p = fold(EDITS);
    let before = p.rows().len();
    let hook = serde_json::json!({"prompt_id":"p2","agent_id":"a1","tool_use_id":"toolu_sub","tool_name":"Read","tool_input":{"file_path":"/x"}});
    p.hook("PreToolUse", &hook, 1);
    p.hook("PostToolUse", &hook, 2);
    assert_eq!(p.rows().len(), before);
}

#[test]
fn either_record_of_a_mid_turn_notification_ends_the_agent_alone() {
    // Claude writes both, but a reader must not need both: the attachment
    // and the enqueue are each one claude.rs's `notified` reads by itself.
    for (dropped, kept) in [("\"queue-operation\"", "attachment"), ("\"queued_command\"", "queue-operation")] {
        let text: String = QUEUED_NOTIFICATION.lines().filter(|l| !l.contains(dropped)).map(|l| format!("{l}\n")).collect();
        let p = fold(&text);
        assert_eq!(sub(&p, "sub:toolu_bgq").status, SubagentState::Completed, "by the {kept} alone");
    }
}

#[test]
fn a_second_turn_duration_does_not_retime_a_timed_turn() {
    let late = r#"{"type":"system","subtype":"turn_duration","durationMs":99999,"timestamp":"2026-10-06T10:00:04.000Z"}"#;
    let p = fold(&format!("{META_PROMPT}{late}\n"));
    assert_eq!(turn(&p, "turn:p2").duration_ms, Some(500));
}

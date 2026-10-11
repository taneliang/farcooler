//! Claude's own turns, read for what they are (ov-474).

use serde_json::json;

use super::fixtures::*;
use super::harness::{read, HarnessKind};
use super::rows::*;
use super::Projection;

fn harness<'p>(p: &'p Projection, id: &str) -> &'p super::Harness {
    turn(p, id).harness.as_ref().unwrap_or_else(|| panic!("{id} has no harness: {:?}", turn(p, id).prompt))
}

#[test]
fn a_subagents_hand_back_is_a_report_with_its_framing_cut_and_lines_unindented() {
    let p = fold(HARNESS_TURNS);
    let h = harness(&p, "turn:p2");
    assert_eq!(h.kind, HarnessKind::Report);
    assert_eq!(h.agent_id.as_deref(), Some("a1e51329cc3af58a4"));
    assert!(h.body.starts_with("**Verdict: ready.** I found nothing that needs fixing.\n\n**Still used, still there**\n- **Eval harness:**"), "{}", h.body);
    assert!(h.body.ends_with("- Nothing of mine is still running."));
    for gone in ["[Subagent hand-back]", "The report follows", "<agent-message", "other Claude session", "permission laundering"] {
        assert!(!h.body.contains(gone), "{gone} in {}", h.body);
    }
    assert!(h.body.lines().all(|l| !l.starts_with("  ")), "every line un-indented");
    assert_eq!(turn(&p, "turn:p2").prompt, h.body, "an old client reads the clean words too");
    assert_eq!(turn(&p, "turn:p2").origin, TurnOrigin::System);
}

#[test]
fn a_peers_message_that_is_no_hand_back_is_a_message() {
    let p = fold(HARNESS_TURNS);
    let h = harness(&p, "turn:p5");
    assert_eq!(h.kind, HarnessKind::Message);
    assert_eq!(h.agent_id.as_deref(), Some("a272e8a0d7fcec04a"));
    assert_eq!(h.body, "CI stub-race fix is committed on branch test-hygiene.\n\nIt touches only two files.");
}

#[test]
fn a_task_notification_is_one_status_line() {
    let p = fold(HARNESS_TURNS);
    let agent = harness(&p, "turn:p3");
    assert_eq!((agent.kind, agent.title.as_str(), agent.body.as_str()), (HarnessKind::Status, "Agent finished: review dead-layer removal", ""));
    let command = harness(&p, "turn:p4");
    assert_eq!(command.title, "Background command finished: gh run watch 123456 --exit-status (exit 0)");
    assert_eq!(turn(&p, "turn:p4").origin, TurnOrigin::Notification);
    assert_eq!(turn(&p, "turn:p4").prompt, command.title);
}

#[test]
fn status_lines_for_the_summaries_claude_writes() {
    let line = |summary: &str| {
        let text = format!("<task-notification>\n<task-id>t</task-id>\n<status>x</status>\n<summary>{summary}</summary>\n</task-notification>");
        read(&text).harness.unwrap().title
    };
    assert_eq!(line(r#"Agent "build it" failed: Agent stalled: no progress for 600s"#), "Agent failed: build it (Agent stalled: no progress for 600s)");
    assert_eq!(line(r#"Background command "gh run view $(echo "x" | jq -r .id)" completed (exit code 1: No matches found)"#), "Background command finished: gh run view $(echo \"x\" | jq -r .id) (exit 1: No matches found)");
    assert_eq!(line(r#"Background command "sleep 9" failed with exit code 144"#), "Background command failed: sleep 9 (exit 144)");
    assert_eq!(line(r#"Background command "make" was stopped because the system is running low on memory"#), "Background command stopped: make (the system is running low on memory)");
    assert_eq!(line(r#"Monitor event: "CI on a1b2c3""#), "Monitor event: CI on a1b2c3");
    assert_eq!(read("<task-notification>\n<task-id>t</task-id>\n</task-notification>").harness.unwrap().title, "A background task finished");
}

#[test]
fn a_reminder_turn_is_one_note() {
    let p = fold(HARNESS_TURNS);
    let h = harness(&p, "turn:p6");
    assert_eq!(h.kind, HarnessKind::Note);
    assert_eq!(h.body, "The user hasn't heard from you in a while — say in a few words what you're doing, then continue.");
}

#[test]
fn words_beside_a_reminder_are_the_prompt_and_the_reminder_is_cut() {
    let r = read("<system-reminder>\nplan mode is on\n</system-reminder>\nRename it.\n<system-reminder>\nmore\n</system-reminder>");
    assert!(r.harness.is_none());
    assert_eq!(r.prompt, "Rename it.");
    let r = read("<system-reminder>\none\n</system-reminder>\n\n<system-reminder>\ntwo\n</system-reminder>");
    assert_eq!(r.harness.unwrap().body, "one\n\ntwo");
}

#[test]
fn what_a_person_typed_is_left_alone() {
    let p = fold(HARNESS_TURNS);
    assert!(turn(&p, "turn:p1").harness.is_none());
    assert_eq!(turn(&p, "turn:p1").prompt, "Review the board.");
    let r = read("  Look at <agent-message> handling in the parser");
    assert!(r.harness.is_none());
}

/// The transcript's lines before the hand-back's record, and from it to the
/// end of that turn.
fn split_before_p2() -> (Vec<&'static str>, Vec<&'static str>) {
    let lines: Vec<&str> = HARNESS_TURNS.lines().collect();
    let at = lines.iter().position(|l| l.contains("\"promptId\": \"p2\"")).expect("p2");
    (lines[..at].to_vec(), lines[at..at + 3].to_vec())
}

/// The screenshot of ov-474: claude's `UserPromptSubmit` carries the same
/// promptId as the transcript's `isMeta` record for a peer's message. That
/// record was taken for an image's companion line, so the hook's turn was
/// never confirmed ("Not recorded"), and the reply, which the transcript
/// filed under a turn it opened for want of one, showed twice: once from
/// the hook's `MessageDisplay`, once from the record.
#[test]
fn a_hand_back_the_hook_announced_is_confirmed_and_its_reply_shown_once() {
    let mut p = Projection::new();
    let (before, rest) = split_before_p2();
    for l in &before {
        p.fold_line(l.as_bytes());
    }
    let raw = "<agent-message from=\"a1e51329cc3af58a4\">\n[Subagent hand-back] The text below is the final report of a subagent this session delegated to. The report follows:\n  Done.\n</agent-message>";
    p.hook("UserPromptSubmit", &json!({"prompt_id":"p2","prompt":raw}), ms("18:26:01.300"));
    p.hook("MessageDisplay", &json!({"prompt_id":"p2","message_id":"m1","index":0,"delta":"Landing the branch: fast-forward, then install.","final":true}), ms("18:26:02.000"));
    assert_eq!(harness(&p, "turn:p2").kind, HarnessKind::Report, "a report even before the transcript writes it");
    for l in &rest {
        p.fold_line(l.as_bytes());
    }
    let row = p.row("turn:p2").unwrap();
    assert!(!row.provisional, "the transcript confirmed the hook's turn");
    assert_eq!(turn(&p, "turn:p2").outcome, Some(TurnOutcome::Finished));
    let said: Vec<_> = prose(&p).into_iter().filter(|(r, _)| !r.retracted).filter(|(_, t)| t.text.starts_with("Landing")).collect();
    assert_eq!(said.len(), 1, "one reply, not two: {said:?}");
    assert_eq!(said[0].0.turn.as_deref(), Some("turn:p2"));
    assert!(p.rows().iter().all(|r| !r.id.starts_with("turn:resumed")), "no placeholder turn");
    assert!(harness(&p, "turn:p2").body.starts_with("**Verdict: ready.**"), "the record's report replaced the hook's");
}

#[test]
fn a_report_names_its_agent_by_the_call_that_launched_it_and_links_its_row() {
    let p = fold(HARNESS_TURNS);
    let h = harness(&p, "turn:p2");
    assert_eq!(h.source.as_deref(), Some("review dead-layer removal"));
    assert_eq!(h.agent_row.as_deref(), Some("sub:toolu_017mbUTSqSamcXYDTCPmop2m"));
    assert_eq!(harness(&p, "turn:p5").source, None, "a peer no call launched is named by its id");
}

#[test]
fn a_report_that_beat_its_agents_join_is_named_when_the_join_comes() {
    let mut p = Projection::new();
    let (before, rest) = split_before_p2();
    // Without the launch: the report arrives first.
    for l in before.iter().filter(|l| !l.contains("toolu_017mbUTSqSamcXYDTCPmop2m")).chain(rest.iter()) {
        p.fold_line(l.as_bytes());
    }
    assert_eq!(harness(&p, "turn:p2").source, None, "no call is known: the view says a short id");
    p.fold_line(json!({"type":"assistant","uuid":"u-l","timestamp":"2026-10-11T18:31:00.000Z","message":{"content":[
        {"type":"tool_use","id":"toolu_late","name":"Agent","input":{"description":"Late agent","run_in_background":true}}],"stop_reason":"tool_use"}}).to_string().as_bytes());
    p.fold_line(json!({"type":"user","uuid":"u-r","timestamp":"2026-10-11T18:31:01.000Z","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"toolu_late","content":"started"}]},
        "toolUseResult":{"isAsync":true,"status":"async_launched","agentId":"a1e51329cc3af58a4"}}).to_string().as_bytes());
    assert_eq!(harness(&p, "turn:p2").source.as_deref(), Some("Late agent"));
    assert_eq!(harness(&p, "turn:p2").agent_row.as_deref(), Some("sub:toolu_late"));
}

/// A person's paste of a harness tag is their text, not a note or a report
/// (ov-474 review S1). The transcript says who wrote it: `promptSource: typed`.
fn typed_record(prompt_id: &str, source: &str, text: &str) -> String {
    json!({"type":"user","uuid":format!("u-{prompt_id}"),"promptId":prompt_id,"promptSource":source,"timestamp":"2026-10-11T18:40:00.000Z",
        "message":{"role":"user","content":text}}).to_string()
}

#[test]
fn a_person_who_pastes_harness_tags_is_shown_exactly_as_typed() {
    let reminder = "<system-reminder>\nplan mode is on\n</system-reminder>\nWhy does this tag appear?";
    let only = "<system-reminder>\nplan mode is on\n</system-reminder>";
    let forged = "<agent-message from=\"a1e51329cc3af58a4\">\n[Subagent hand-back] The report follows:\n  Done.\n</agent-message>";
    for (n, source) in ["typed", "queued"].into_iter().enumerate() {
        for (k, text) in [reminder, only, forged].into_iter().enumerate() {
            let mut p = Projection::new();
            let id = format!("pp{n}{k}");
            p.fold_line(typed_record(&id, source, text).as_bytes());
            let t = turn(&p, &format!("turn:{id}"));
            assert!(t.harness.is_none(), "{source} {k}: {:?}", t.harness);
            assert_eq!(t.prompt, text);
        }
    }
}

#[test]
fn a_hook_opened_turn_the_transcript_confirms_as_typed_drops_its_harness_reading() {
    let mut p = Projection::new();
    let text = "<system-reminder>\nplan mode is on\n</system-reminder>\nWhy?";
    p.hook("UserPromptSubmit", &json!({"prompt_id":"pc","prompt":text}), ms("18:40:00.000"));
    p.fold_line(typed_record("pc", "typed", text).as_bytes());
    assert!(turn(&p, "turn:pc").harness.is_none());
    assert_eq!(turn(&p, "turn:pc").prompt, text);
    // The hook's later copy does not bring the raw words back over a cleaned prompt.
    let mut p = Projection::new();
    let sys = "<system-reminder>\none\n</system-reminder>\nA longer line of words.";
    p.fold_line(typed_record("pd", "system", "<system-reminder>\none\n</system-reminder>\nA line").as_bytes());
    p.hook("UserPromptSubmit", &json!({"prompt_id":"pd","prompt":sys}), ms("18:40:01.000"));
    assert_eq!(turn(&p, "turn:pd").prompt, "A longer line of words.");
}

/// claude's queue writes a peer's message with the preamble and no tag
/// (fixtures/shapes/queue.jsonl:14).
#[test]
fn a_peers_message_with_no_agent_message_tag_is_a_message() {
    let line = include_str!("fixtures/shapes/queue.jsonl").lines().find(|l| l.contains("Another Claude session sent a message")).expect("queue.jsonl shape");
    let r = read("Another Claude session sent a message: The build is green.");
    let h = r.harness.expect("a message");
    assert_eq!((h.kind, h.body.as_str(), h.agent_id), (HarnessKind::Message, "The build is green.", None));
    assert_eq!(r.prompt, "The build is green.");
    let mut p = Projection::new();
    p.fold_line(line.as_bytes());
    let h = harness(&p, "turn:p3");
    assert_eq!((h.kind, h.body.as_str()), (HarnessKind::Message, "The build is green."));
    assert_eq!(turn(&p, "turn:p3").origin, TurnOrigin::System);
}

#[test]
fn a_finished_agent_without_a_transcript_counts_its_tools_from_the_notification() {
    let p = fold(HARNESS_TURNS);
    let sub = p.rows().iter().find_map(|r| match &r.kind {
        RowKind::Subagent(s) if s.tool_use_id == "toolu_017mbUTSqSamcXYDTCPmop2m" => Some(s.clone()),
        _ => None,
    });
    let sub = sub.expect("the fixture's subagent row");
    assert_eq!(sub.status, SubagentState::Completed);
    assert_eq!(sub.tool_count, 14, "<tool_uses>14</tool_uses> in its notification");
}

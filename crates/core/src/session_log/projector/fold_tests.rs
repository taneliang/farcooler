//! The fold over each fixture: what a person would see, row by row.

use super::fixtures::*;
use super::rows::*;
use super::Projection;

#[test]
fn a_recorded_session_folds_into_its_prompts_queued_message_and_replies() {
    let p = fold(RECORDED);
    let t = turns(&p);
    let prompts: Vec<&str> = t.iter().map(|(_, t)| t.prompt.as_str()).collect();
    assert_eq!(
        prompts,
        [
            "First say one short sentence about your plan. Then use the Agent tool to launch one general-purpose subagent with the description 'count readme lines' that runs wc -l README.md and reports the number. After it returns, use Bash to run: echo done > out.txt",
            "line one of a draft\nline two\n  indented line three",
            "second message typed while busy",
            "[Image #1] what color is this?",
            "status probe",
        ],
        "every prompt is a row, in full, line breaks kept (ov-358 finding 4)"
    );
    assert_eq!(t[2].1.origin, TurnOrigin::Queued, "the busy message came off claude's queue");
    let revoked = TurnOutcome::Failed { detail: "OAuth token revoked · Please run /login".into() };
    assert!(t.iter().all(|(_, t)| t.outcome.as_ref() == Some(&revoked)), "isApiErrorMessage fails the turn: {t:?}");
    assert_eq!(t[1].1.duration_ms, Some(14247), "turn_duration's own number wins over the span");

    let queued: Vec<&Queued> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Queued(q) => Some(q), _ => None }).collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].text, "second message typed while busy");
    assert_eq!(queued[0].state, QueuedState::Sent, "the dequeue sent it");

    assert!(prose(&p).is_empty(), "claude's error report is the outcome, not the model's prose");
    assert_eq!(p.stats().gaps, 0, "nothing in a real session is unreadable");
}

#[test]
fn the_image_prompt_skips_its_payload_and_its_meta_companion_opens_no_turn() {
    let p = fold(RECORDED);
    let t = turn(&p, "turn:06ad284b-c727-440e-830f-43da4ae43245");
    assert_eq!(t.prompt, "[Image #1] what color is this?", "the text block, not the image");
    assert_eq!(turns(&p).len(), 5, "the isMeta companion with the same promptId is not a sixth turn");
}

#[test]
fn a_background_subagent_ends_after_its_turn_and_the_turn_says_so_until_it_does() {
    let mut p = Projection::new();
    let lines: Vec<&str> = BACKGROUND.lines().collect();
    // Up to and including the first turn's turn_duration.
    let first_end = lines.iter().position(|l| l.contains("pendingBackgroundAgentCount")).unwrap();
    for line in &lines[..=first_end] {
        p.fold_line(line.as_bytes());
    }
    let t1 = turn(&p, "turn:p1");
    assert_eq!(t1.outcome, Some(TurnOutcome::Finished));
    assert_eq!(t1.background_running, 1, "over, with one agent still running");
    let bg = sub(&p, "sub:toolu_bg");
    assert!(bg.background);
    assert_eq!(bg.status, SubagentState::Running);
    assert_eq!(bg.agent_id.as_deref(), Some("abg1"), "the async launch names it");

    for line in &lines[first_end + 1..] {
        p.fold_line(line.as_bytes());
    }
    let bg = sub(&p, "sub:toolu_bg");
    assert_eq!(bg.status, SubagentState::Completed, "the notification ended it");
    assert_eq!(bg.ended_ms, Some(ms("10:01:00.000")), "its first end: the enqueued notification");
    assert_eq!(turn(&p, "turn:p1").background_running, 0);
    let t2 = turn(&p, "turn:p2");
    assert_eq!(t2.origin, TurnOrigin::Notification);
    assert_eq!(t2.prompt, "Agent \"Count the lines\" finished");
    let queued = p.rows().iter().filter(|r| matches!(r.kind, RowKind::Queued(_))).count();
    assert_eq!(queued, 0, "claude's own queued notification is not a person's message");
}

#[test]
fn a_foreground_subagent_ends_at_its_result_with_its_count() {
    let p = fold(BACKGROUND);
    let fg = sub(&p, "sub:toolu_fg");
    assert!(!fg.background);
    assert_eq!(fg.status, SubagentState::Completed);
    assert_eq!(fg.agent_type, "Explore");
    assert_eq!(fg.tool_count, 2, "totalToolUseCount");
    assert_eq!(fg.ended_ms.zip(fg.started_ms).map(|(e, s)| e - s), Some(5000));
}

#[test]
fn thinking_is_a_duration_and_nothing_else() {
    let p = fold(BACKGROUND);
    let thinking: Vec<&Thinking> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Thinking(t) => Some(t), _ => None }).collect();
    assert_eq!(thinking.len(), 1);
    assert_eq!(thinking[0].started_ms, Some(ms("10:00:00.000")), "from the prompt");
    assert_eq!(thinking[0].ended_ms, Some(ms("10:00:02.000")), "to the record");
}

#[test]
fn tools_carry_status_duration_and_an_edit_its_hunks() {
    let p = fold(EDITS);
    let bash = tool(&p, "tool:toolu_b1");
    assert_eq!(bash.status, ToolStatus::Failed, "is_error");
    assert_eq!(bash.summary, "Run the add test");
    assert_eq!(bash.ended_ms.zip(bash.started_ms).map(|(e, s)| e - s), Some(500));
    let edit = tool(&p, "tool:toolu_e1");
    assert_eq!(edit.status, ToolStatus::Done);
    assert_eq!(edit.file_path.as_deref(), Some("/tmp/p/tests/test_add.sh"));
    assert_eq!(edit.diff.len(), 1);
    assert_eq!(edit.diff[0].lines[2], "-[ \"$(expr 2 + 3)\" = 4 ] || exit 1");
    assert_eq!((edit.diff[0].old_start, edit.diff[0].new_lines), (1, 3));
}

#[test]
fn a_question_is_an_ask_row_answered_by_its_result() {
    let p = fold(EDITS);
    let RowKind::Ask(ask) = &p.row("ask:toolu_q1").unwrap().kind else { panic!() };
    assert_eq!(ask.kind, AskKind::Question);
    assert_eq!(ask.text, "Commit the fix?");
    assert!(ask.answered);
    assert_eq!(ask.answered_ms, Some(ms("11:00:40.000")));
}

#[test]
fn an_interrupt_ends_the_turn_as_interrupted() {
    let p = fold(EDITS);
    assert_eq!(turn(&p, "turn:p1").outcome, Some(TurnOutcome::Finished));
    assert_eq!(turn(&p, "turn:p2").outcome, Some(TurnOutcome::Interrupted));
    assert_eq!(tool(&p, "tool:toolu_b2").status, ToolStatus::Failed, "the rejected call");
}

#[test]
fn compaction_is_one_notice_and_the_summary_opens_no_turn() {
    let p = fold(COMPACT);
    let notices: Vec<&Notice> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Notice(n) => Some(n), _ => None }).collect();
    assert_eq!(notices.iter().map(|n| n.kind).collect::<Vec<_>>(), [NoticeKind::Compacted, NoticeKind::Command]);
    assert_eq!(notices[1].text, "/compact");
    assert_eq!(turns(&p).len(), 2, "the compact summary is not a prompt");
    assert_eq!(turn(&p, "turn:p2").outcome, Some(TurnOutcome::Finished));
}

#[test]
fn non_monotonic_timestamps_clamp_to_zero_and_never_reorder() {
    let p = fold(NONMONOTONIC);
    let t = tool(&p, "tool:toolu_n1");
    assert_eq!(t.ended_ms, t.started_ms, "a result stamped before its call is a zero-length call");
    let turn = turn(&p, "turn:p1");
    assert_eq!(turn.duration_ms, Some(3000), "claude's own duration");
    assert!(turn.ended_ms >= turn.started_ms);
    let order: Vec<&str> = p.rows().iter().map(|r| r.id.as_str()).collect();
    assert_eq!(order[0], "turn:p1", "line order, not timestamp order");
    assert_eq!(order[1], "tool:toolu_n1");
}

#[test]
fn unknown_and_unreadable_lines_become_gaps_and_odd_fields_read_as_absent() {
    let p = fold(UNKNOWN);
    let gaps: Vec<&Gap> = p.rows().iter().filter_map(|r| match &r.kind { RowKind::Gap(g) => Some(g), _ => None }).collect();
    assert_eq!(
        gaps,
        [&Gap { reason: GapReason::Unknown("brand-new-record".into()), count: 2 }, &Gap { reason: GapReason::Unparsed, count: 1 }],
        "two unknown records fold into one row"
    );
    let t = tool(&p, "tool:toolu_u1");
    assert_eq!(t.summary, "", "a command that is an array is no summary, not a failure");
    assert_eq!(t.status, ToolStatus::Done, "a string toolUseResult still closes the call");
    assert_eq!(prose(&p).last().unwrap().1.text, "Still standing.");
    assert_eq!(turn(&p, "turn:p1").outcome, Some(TurnOutcome::Finished));
}

#[test]
fn a_record_before_any_prompt_opens_a_placeholder_turn() {
    let p = fold(&BACKGROUND.lines().skip(3).collect::<Vec<_>>().join("\n"));
    let first = &p.rows()[0];
    assert!(first.id.starts_with("turn:resumed:"), "{first:?}");
}

#[test]
fn ids_and_ords_never_move_and_revisions_only_grow() {
    let mut p = Projection::new();
    let mut seen: Vec<(String, u64)> = Vec::new();
    let mut last_rev = 0;
    for line in EDITS.lines() {
        p.fold_line(line.as_bytes());
        for (id, ord) in &seen {
            assert_eq!(p.row(id).map(|r| r.ord), Some(*ord), "{id} moved");
        }
        seen = p.rows().iter().map(|r| (r.id.clone(), r.ord)).collect();
        assert!(p.revision() >= last_rev);
        last_rev = p.revision();
    }
    let changed = p.changed_since(0).len();
    assert_eq!(changed, p.rows().len(), "every row changed since 0");
    assert!(p.changed_since(p.revision()).is_empty());
    let page = p.page(None, 3);
    assert_eq!(page.len(), 3);
    assert_eq!(page.last().unwrap().ord as usize, p.rows().len() - 1);
    let before = p.page(Some(page[0].ord), 100);
    assert_eq!(before.last().unwrap().ord + 1, page[0].ord, "pages abut");
}

#[test]
fn take_changed_names_each_changed_row_once() {
    let mut p = fold(EDITS);
    let first = p.take_changed();
    assert_eq!(first.len(), p.rows().len());
    assert!(p.take_changed().is_empty());
    p.fold_line(br#"{"type":"user","promptId":"p3","promptSource":"typed","message":{"content":"again"}}"#);
    let next = p.take_changed();
    assert!(next.contains(&"turn:p3".to_string()), "{next:?}");
}

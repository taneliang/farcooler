use super::*;
use farcooler_protocol::v1::{TaskDetail, TaskNote};

const QUESTION: &str = include_str!("../testdata/workflow-change-question.txt");

fn approving() -> String {
    let at = QUESTION.rfind("answer `approve ").unwrap() + "answer `".len();
    QUESTION[at..].split('`').next().unwrap().to_owned()
}

fn asked_as(actor: &str, id: &str, options: &[&str], body: &str) -> TaskNote {
    TaskNote { id: bytes::Bytes::copy_from_slice(Uuid::from_u128(7).as_bytes()), kind: TaskNoteKind::Question as i32,
        actor: actor.into(), body: body.into(),
        extra_json: serde_json::json!({ "workflow_draft": { "id": id }, "options": options }).to_string(),
        ..Default::default() }
}

fn card(notes: Vec<TaskNote>) -> TaskDetail { TaskDetail { notes, ..Default::default() } }

#[test]
fn a_draft_shows_the_whole_question_and_approves_with_its_own_words() {
    let approve = approving();
    let detail = card(vec![asked_as("manager", "d1", &[&approve, "decline"], QUESTION)]);
    let shown = draft("d1", &detail);
    assert_eq!((shown.question.as_str(), shown.problem.clone()), (QUESTION, None));
    assert_eq!(decision("d1", &shown.note, true, &detail), Some(approve));
    assert_eq!(decision("d1", &shown.note, false, &detail), Some("decline".into()));
    assert_eq!(decision("d1", "another-note", true, &detail), None, "a question asked again isn't the one read");
}

#[test]
fn only_the_runners_latest_question_for_this_draft_can_be_answered() {
    let approve = approving();
    let good = asked_as("manager", "d1", &[&approve, "decline"], QUESTION);
    for (why, notes) in [
        ("someone else asked", vec![asked_as("user", "d1", &[&approve, "decline"], QUESTION)]),
        ("another draft's", vec![asked_as("manager", "d2", &[&approve, "decline"], QUESTION)]),
        ("a later question replaced it", vec![good.clone(), asked_as("user", "d1", &[], "Which one?")]),
        ("the words aren't in what is shown", vec![asked_as("manager", "d1", &["approve ffffff", "decline"], QUESTION)]),
        ("no question", vec![]),
    ] {
        let detail = card(notes);
        let shown = draft("d1", &detail);
        assert!(shown.problem.is_some() && shown.question.is_empty() && shown.note.is_empty(), "{why}: {shown:?}");
        assert_eq!(decision("d1", "00000000-0000-0000-0000-000000000007", true, &detail), None, "{why}");
    }
}

#[test]
fn an_answer_is_reported_from_the_drafts_state_not_from_the_press() {
    let finding = TaskNote { kind: TaskNoteKind::Finding as i32, body: "Only the owner or a Maintainer can approve.".into(),
        ..Default::default() };
    let detail = card(vec![finding]);
    assert!(decision_outcome(true, "approved", &detail).ok);
    assert!(decision_outcome(false, "declined", &detail).ok);
    for (approve, state) in [(true, "pending"), (true, "failed"), (true, "out_of_date"), (true, "declined"), (false, "approved")] {
        let outcome = decision_outcome(approve, state, &detail);
        assert!(!outcome.ok, "{approve} {state}: {outcome:?}");
        assert_eq!(outcome.detail.as_deref(), Some("Only the owner or a Maintainer can approve."));
    }
}

#[test]
fn a_receipt_says_what_the_runner_did() {
    let receipt = |result: serde_json::Value| serde_json::json!({ "receipt": { "result": result } });
    assert!(receipt_outcome(Kind::Cancel, &receipt(serde_json::json!({ "Ok": null }))).ok);
    assert_eq!(receipt_outcome(Kind::Cancel, &receipt(serde_json::json!({ "Err": "Stale" }))).message, CHANGED);
    let settle = receipt_outcome(Kind::SettleLanded, &receipt(serde_json::json!({ "Err": "the remote disagrees" })));
    assert_eq!((settle.ok, settle.detail.as_deref()), (false, Some("the remote disagrees")));
    assert!(!settle.message.contains("remote disagrees"), "the runner's words stay apart from ours");
    assert!(!receipt_outcome(Kind::Resolve, &serde_json::json!({})).ok, "an unreadable answer is no success");
}

#[test]
fn a_cancellation_is_resolved_only_with_the_persons_reason() {
    let selection = |reason: Option<&str>| Selection { item: "ov-1".into(), kind: Kind::ResolveCancellation,
        guard: String::new(), reason: reason.map(str::to_owned) };
    for reason in [None, Some("   "), Some("bell\u{7}")] {
        assert!(missing_reason(&selection(reason)).is_some(), "{reason:?}");
    }
    assert!(missing_reason(&selection(Some("I checked."))).is_none());
}

#[test]
fn the_plans_cards_win_over_the_boards_list() {
    let plan = Plan { cards: vec![pb::PlanCard { key: "ov-1".into(), title: "From the plan".into(), ..Default::default() }],
        ..Default::default() };
    let raw = serde_json::json!({ "state": { "items": { "ov-1": {}, "ov-2": {}, "group-1": { "group_of": {} } } } });
    assert!(lacks_titles(&raw, &plan));
    let tasks = ["ov-1", "ov-2"].map(|key| pb::Task { key: key.into(), title: "From the board".into(), ..Default::default() });
    let plan = with_titles(plan, &tasks);
    assert_eq!(plan.cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["From the plan", "From the board"]);
    assert!(!lacks_titles(&raw, &plan), "a landing group has no ticket to name");
}

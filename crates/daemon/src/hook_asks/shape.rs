//! What a held ask asks, and what a device's answer to it becomes (ov-370).
//!
//! claude routes three kinds of dialog through `PermissionRequest`, and a
//! hook answers each differently (measured on claude 2.1.290, ov-370):
//!
//! - **A permission** (may this tool run): a plain allow or a deny.
//! - **A question** (`AskUserQuestion`): an allow whose `updatedInput` is the
//!   question's own input plus `answers`, each question's text to its
//!   answer. Without `questions` claude refuses the input; a plain allow
//!   leaves the dialog up.
//! - **A plan's approval** (`ExitPlanMode`): an allow that carries an
//!   `updatedInput` approves it, and leaves claude in its default mode; a
//!   plain allow leaves the dialog up. A deny keeps claude planning, and its
//!   message reaches the model.
//!
//! So each takes the options its view offers, and nothing else: a phone's
//! `allow` can't approve a plan nobody saw through a shape that isn't one,
//! and can't answer a question at all.

use std::collections::HashMap;

use farcooler_agent_hooks::wire::Decision;
use farcooler_core::session_log::projector::held::ASK_ENDED;
use serde_json::Value;
use uuid::Uuid;

use super::AnswerRefused;

/// What a held ask is asking.
#[derive(Debug, Clone, PartialEq)]
pub enum AskShape {
    /// May a tool run: `allow` or `deny`.
    Permission,
    /// An `AskUserQuestion`, with claude's input for it: `answer`, with an
    /// answer to every question.
    Question { input: Value },
    /// An `ExitPlanMode`, with claude's input for it: `allow` approves the
    /// plan, `deny` keeps claude planning.
    Plan { input: Value },
}

impl AskShape {
    /// The ask a claude `PermissionRequest` for `tool` with `input` makes.
    pub fn of(tool: Option<&str>, input: &Value) -> Self {
        match tool {
            Some("AskUserQuestion") => AskShape::Question { input: input.clone() },
            Some("ExitPlanMode") => AskShape::Plan { input: input.clone() },
            _ => AskShape::Permission,
        }
    }

    /// Whether every surface may offer it as Allow and Deny (`Permission`
    /// events, the lock screen, the watch). A question or a plan is answered
    /// only where it can be read: its row in a conversation view.
    pub fn is_permission(&self) -> bool {
        matches!(self, AskShape::Permission)
    }
}

/// The verdict a device's `option` (and, for a question, `answers`) becomes,
/// or why it can't be one. `decider` names the device.
pub(super) fn decide(
    shape: &AskShape,
    option: &str,
    answers: &HashMap<String, String>,
    decider: &str,
) -> Result<Decision, AnswerRefused> {
    match (shape, option) {
        (AskShape::Permission, "allow") => Ok(Decision::allow()),
        (AskShape::Permission, "deny") => Ok(Decision::Deny { message: format!("Denied from {decider}") }),
        // claude's own input, unchanged: the plan it approves is the one it
        // wrote, and an allow without an input leaves the dialog up.
        (AskShape::Plan { input }, "allow") => Ok(Decision::Allow { updated_input: Some(input.clone()) }),
        (AskShape::Plan { .. }, "deny") => {
            Ok(Decision::Deny { message: format!("Keep planning: the plan wasn't approved from {decider}.") })
        }
        (AskShape::Question { input }, "answer") => answered(input, answers),
        _ => Err(AnswerRefused::UnknownOption),
    }
}

/// A question's input with `answers` in it, when they answer each of its
/// questions and nothing else.
fn answered(input: &Value, answers: &HashMap<String, String>) -> Result<Decision, AnswerRefused> {
    let asked: Vec<&str> = input["questions"]
        .as_array()
        .map(|qs| qs.iter().filter_map(|q| q["question"].as_str()).collect())
        .unwrap_or_default();
    let every = !asked.is_empty()
        && asked.iter().all(|q| answers.get(*q).is_some_and(|a| !a.trim().is_empty()))
        && answers.keys().all(|k| asked.contains(&k.as_str()));
    if !every {
        return Err(AnswerRefused::Unanswered);
    }
    let mut input = input.clone();
    input["answers"] = serde_json::to_value(answers).map_err(|_| AnswerRefused::Unanswered)?;
    Ok(Decision::Allow { updated_input: Some(input) })
}

/// The hold of `id` on `terminal` ended: the row it was on stops offering
/// it, and says `by` answered it when a device did.
pub(super) fn ended(terminal: Uuid, id: &str, by: Option<&str>) {
    let note = serde_json::json!({ "ask_id": id, "by": by });
    crate::session_projectors::global().hook(terminal, ASK_ENDED, &note);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Value {
        serde_json::json!({ "questions": [
            { "question": "Which color?", "header": "Color", "options": [{ "label": "Red" }, { "label": "Blue" }], "multiSelect": false },
            { "question": "Which sizes?", "header": "Sizes", "options": [{ "label": "S" }, { "label": "L" }], "multiSelect": true },
        ] })
    }

    fn answers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(q, a)| (q.to_string(), a.to_string())).collect()
    }

    /// The input claude takes as a question's answer: its own, plus
    /// `answers` (measured: without `questions` claude refuses it).
    #[test]
    fn a_question_is_answered_with_its_own_input_and_the_answers() {
        let given = answers(&[("Which color?", "Blue"), ("Which sizes?", "S, L")]);
        let decision = decide(&AskShape::Question { input: question() }, "answer", &given, "Mac").unwrap();
        let Decision::Allow { updated_input: Some(input) } = decision else { panic!("{decision:?}") };
        assert_eq!(input["questions"], question()["questions"], "claude validates the questions it asked");
        assert_eq!(input["answers"], serde_json::json!({ "Which color?": "Blue", "Which sizes?": "S, L" }));
    }

    #[test]
    fn a_question_answered_in_part_or_beside_the_point_is_refused() {
        let shape = AskShape::Question { input: question() };
        let part = answers(&[("Which color?", "Blue")]);
        assert_eq!(decide(&shape, "answer", &part, "Mac"), Err(AnswerRefused::Unanswered));
        let blank = answers(&[("Which color?", "Blue"), ("Which sizes?", "  ")]);
        assert_eq!(decide(&shape, "answer", &blank, "Mac"), Err(AnswerRefused::Unanswered));
        let extra = answers(&[("Which color?", "Blue"), ("Which sizes?", "S"), ("Why?", "x")]);
        assert_eq!(decide(&shape, "answer", &extra, "Mac"), Err(AnswerRefused::Unanswered));
        assert_eq!(decide(&shape, "allow", &HashMap::new(), "iPhone"), Err(AnswerRefused::UnknownOption), "a phone's Allow");
    }

    /// An approval carries claude's input (measured: a plain allow leaves
    /// the dialog up); keeping on planning names the device.
    #[test]
    fn a_plan_is_approved_with_its_own_input_or_kept_in_planning() {
        let input = serde_json::json!({ "plan": "# Plan", "planFilePath": "/p.md" });
        let shape = AskShape::Plan { input: input.clone() };
        assert_eq!(decide(&shape, "allow", &HashMap::new(), "Mac"), Ok(Decision::Allow { updated_input: Some(input) }));
        let Ok(Decision::Deny { message }) = decide(&shape, "deny", &HashMap::new(), "iPhone") else { panic!() };
        assert!(message.contains("Keep planning") && message.contains("iPhone"), "{message}");
        assert_eq!(decide(&shape, "answer", &HashMap::new(), "Mac"), Err(AnswerRefused::UnknownOption));
    }

    #[test]
    fn a_permission_is_allowed_plainly_or_denied_by_name() {
        assert_eq!(decide(&AskShape::Permission, "allow", &HashMap::new(), "Mac"), Ok(Decision::allow()));
        assert_eq!(
            decide(&AskShape::Permission, "deny", &HashMap::new(), "iPhone"),
            Ok(Decision::Deny { message: "Denied from iPhone".into() })
        );
        assert_eq!(decide(&AskShape::Permission, "answer", &HashMap::new(), "Mac"), Err(AnswerRefused::UnknownOption));
    }

    #[test]
    fn only_claudes_two_dialogs_are_not_permissions() {
        let input = serde_json::json!({});
        assert!(AskShape::of(Some("Bash"), &input).is_permission());
        assert!(AskShape::of(None, &input).is_permission());
        assert!(!AskShape::of(Some("AskUserQuestion"), &input).is_permission());
        assert!(!AskShape::of(Some("ExitPlanMode"), &input).is_permission());
    }
}

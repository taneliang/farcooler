//! `terminal agent-answer`: one answer to an agent's held ask, from a device
//! or a script. A claude question (ov-370) is answered with option `answer`
//! and `--answers-json`, each question's text to its answer.

use std::collections::HashMap;

use farcooler_protocol::v1::request;
use uuid::Uuid;

use super::{Fallible, id_bytes, req, short, tasks, with};

/// `--answers-json`, as the map the runner takes: a JSON object of strings.
/// Absent is no answers, which every ask but a question takes.
pub(crate) fn parse_answers(json: Option<&str>) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let Some(json) = json else { return Ok(HashMap::new()) };
    serde_json::from_str::<HashMap<String, String>>(json)
        .map_err(|_| "--answers-json takes an object of strings: {\"<question>\": \"<answer>\"}".into())
}

/// What a question answered in part is told.
pub(crate) const ANSWER_EVERY_QUESTION: &str =
    "answer every question: --answers-json '{\"<question>\": \"<answer>\"}' with option answer";

/// `terminal agent-answer`'s call, with its refusals said by `answer_refused`.
pub(crate) async fn answer_agent<L: tasks::DispatchLink>(
    link: &mut L,
    terminal: Uuid,
    request_id: String,
    option_id: String,
    answers: HashMap<String, String>,
) -> Fallible {
    link.call(with(
        req("terminal.agent_answer"),
        request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
            terminal_id: id_bytes(terminal),
            request_id,
            option_id,
            answers,
        }),
    ))
    .await
    .map_err(|e| answer_refused(e, &short(terminal)))?;
    Ok(())
}

/// A refused `terminal agent-answer`, in this CLI's words when the runner
/// named which of its two conflicts it was.
///
/// Both are `resource-conflict`, and the runner's own message is the apps'
/// capitalized sentence. `said_about` holds this CLI's line for each, in
/// clap's style. Anything else is left as it was.
pub(crate) fn answer_refused(e: farcooler_transport::ClientError, terminal: &str) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && matches!(what.as_str(), "not_held" | "not_delivered")
        && let Some(said) = tasks::said_about(what)
    {
        return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
    }
    // A question answered in part, or not at all (review 1 L3).
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && what == "answers"
    {
        return Box::new(tasks::Refused::naming(ANSWER_EVERY_QUESTION.to_string(), *code, what.clone()));
    }
    tasks::agent_refused(terminal)(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_an_object_of_strings_or_nothing() {
        assert!(parse_answers(None).unwrap().is_empty());
        let parsed = parse_answers(Some(r#"{"Which color?": "Blue", "Sizes?": "Small, Large"}"#)).unwrap();
        assert_eq!(parsed["Which color?"], "Blue");
        assert_eq!(parsed["Sizes?"], "Small, Large");
        assert!(parse_answers(Some(r#"["Blue"]"#)).is_err(), "a list names no question");
        assert!(parse_answers(Some(r#"{"Which?": 1}"#)).is_err(), "an answer is words");
    }

    #[test]
    fn a_question_answered_in_part_is_told_in_this_clis_words() {
        let refused = answer_refused(
            farcooler_transport::ClientError::Daemon {
                code: farcooler_protocol::v1::ErrorCode::InvalidArgument as i32,
                retryable: false,
                message: "Invalid argument.".into(),
                what: "answers".into(),
            },
            "ab12",
        );
        assert_eq!(refused.to_string(), ANSWER_EVERY_QUESTION);
    }
}

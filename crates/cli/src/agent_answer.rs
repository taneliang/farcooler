//! `terminal agent-answer`: one answer to an agent's held ask, from a device
//! or a script. A claude question (ov-370) is answered with option `answer`
//! and `--answers-json`, each question's text to its answer.

use std::collections::HashMap;

use farcooler_protocol::v1::request;
use uuid::Uuid;

use super::{Fallible, id_bytes, req, tasks, with};

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
            actor: caller(std::env::var(farcooler_core::pane_env::ACTOR).ok()),
        }),
    ))
    .await
    .map_err(answer_refused)?;
    Ok(())
}

/// Who this CLI answers as (`AgentAnswer.actor`, ov-493): the pane's
/// `FARCOOLER_ACTOR` when it names one, else `user`. The runner decides who
/// the caller is from its process (ov-647), and takes this only as a word
/// against: an actor that isn't `user` is never answered as the person. It
/// used to send `pane` from inside a runner's tmux, which refused a person in
/// their own shell pane, whom the runner can tell from an agent.
fn caller(actor: Option<String>) -> String {
    actor.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| "user".to_string())
}

/// A refused `terminal agent-answer`, in this CLI's words when the runner
/// named which of its conflicts it was: answered elsewhere, undelivered, or
/// not the person's (ov-647).
///
/// Each is `resource-conflict`, and the runner's own message is the apps'
/// capitalized sentence. `said_about` holds this CLI's line for each, in
/// clap's style. Anything else is left as it was.
pub(crate) fn answer_refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && matches!(what.as_str(), "not_held" | "not_delivered" | "not_a_person")
        && let Some(said) = tasks::said_about(what)
    {
        return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
    }
    // A folder-trust prompt's own refusals (ov-493).
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && let Some(said) = trust_refusal(what)
    {
        return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
    }
    // A question answered in part, or not at all (review 1 L3).
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && what == "answers"
    {
        return Box::new(tasks::Refused::naming(ANSWER_EVERY_QUESTION.to_string(), *code, what.clone()));
    }
    Box::new(e)
}

/// What this CLI says for a refused answer to a folder-trust prompt.
fn trust_refusal(what: &str) -> Option<&'static str> {
    Some(match what {
        "not_on_yes" => "the prompt's cursor isn't on Yes. answer it in the terminal",
        "too_soon" => "an answer was just sent there. wait a moment, and check the terminal before answering again",
        "unconfirmed" => "the Enter was sent, but Claude Code hasn't moved on yet. check the terminal before answering again",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Who answers: the pane's actor, else the person, whom the runner checks
    /// by the caller's process (ov-647).
    #[test]
    fn the_caller_is_named() {
        assert_eq!(caller(Some("agent:1".into())), "agent:1");
        assert_eq!(caller(Some("manager".into())), "manager");
        assert_eq!(caller(Some(" ".into())), "user");
        assert_eq!(caller(None), "user");
    }

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
        );
        assert_eq!(refused.to_string(), ANSWER_EVERY_QUESTION);
    }
}

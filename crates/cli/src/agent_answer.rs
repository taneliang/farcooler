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
            actor: caller(std::env::var(farcooler_core::pane_env::ACTOR).ok(), std::env::var("TMUX").ok()),
        }),
    ))
    .await
    .map_err(answer_refused)?;
    Ok(())
}

/// Who this CLI answers as (`AgentAnswer.actor`, ov-493): the pane's
/// `FARCOOLER_ACTOR` when it names one, `pane` from inside a runner's tmux
/// with none (a shell beside an agent), else `user`, a person. A runner's
/// tmux socket is named `farcooler-…`.
fn caller(actor: Option<String>, tmux: Option<String>) -> String {
    if let Some(actor) = actor.map(|a| a.trim().to_string()).filter(|a| !a.is_empty()) {
        return actor;
    }
    let in_a_runner = tmux.is_some_and(|t| t.split(',').next().unwrap_or("").rsplit('/').next().unwrap_or("").starts_with("farcooler-"));
    if in_a_runner { "pane".to_string() } else { "user".to_string() }
}

/// A refused `terminal agent-answer`, in this CLI's words when the runner
/// named which of its two conflicts it was.
///
/// Both are `resource-conflict`, and the runner's own message is the apps'
/// capitalized sentence. `said_about` holds this CLI's line for each, in
/// clap's style. Anything else is left as it was.
pub(crate) fn answer_refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e
        && matches!(what.as_str(), "not_held" | "not_delivered")
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
        "not_a_person" => "only a person answers a folder-trust prompt: tell the owner it's waiting in Needs You",
        "not_on_yes" => "the prompt's cursor isn't on Yes. answer it in the terminal",
        "too_soon" => "an answer was just sent there. wait a moment, and check the terminal before answering again",
        "unconfirmed" => "the Enter was sent, but Claude Code hasn't moved on yet. check the terminal before answering again",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Who answers: the pane's actor, a shell in a runner's pane, or a person.
    #[test]
    fn the_caller_is_named() {
        assert_eq!(caller(Some("agent:1".into()), None), "agent:1");
        assert_eq!(caller(Some("manager".into()), Some("/tmp/tmux-502/farcooler-01a,1,0".into())), "manager");
        assert_eq!(caller(None, Some("/private/tmp/tmux-502/farcooler-01a00cb1,123,4".into())), "pane");
        assert_eq!(caller(Some(" ".into()), Some("/tmp/tmux-502/default,1,0".into())), "user");
        assert_eq!(caller(None, None), "user");
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

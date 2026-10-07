//! `terminal.agent_answer`'s `answers` (ov-370): a claude question's
//! answers, each question's words to its answer, as an app sends them.

use std::collections::HashMap;

use serde_json::Value;

use crate::session::SessionError;

/// `args.answers` as the map the runner takes: absent or null is none, an
/// object of strings is the answers, anything else is the app's mistake,
/// said as one rather than sent as no answers (review 1 L2), which the runner
/// would word as a question left unanswered.
pub(super) fn answers(method: &str, args: &Value) -> Result<HashMap<String, String>, SessionError> {
    match args.get("answers") {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(given) => serde_json::from_value(given.clone())
            .map_err(|_| SessionError::Protocol(format!("{method} needs answers as an object of strings"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_are_an_object_of_strings_or_none() {
        assert!(answers("m", &json!({})).unwrap().is_empty());
        assert!(answers("m", &json!({ "answers": null })).unwrap().is_empty());
        let given = answers("m", &json!({ "answers": { "Which color?": "Blue" } })).unwrap();
        assert_eq!(given["Which color?"], "Blue");
        assert!(answers("m", &json!({ "answers": ["Blue"] })).is_err(), "a list names no question");
        assert!(answers("m", &json!({ "answers": { "Which color?": 1 } })).is_err(), "an answer is words");
    }
}

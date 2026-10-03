//! How a Claude turn that went wrong ends: the CLI's API-error frames and
//! the `result` after them, read as `EndReason::Failed` (ov-140).
//!
//! Its own file so `normalize.rs` stays inside its size budget, and because
//! this is the one part of normalizing that reads failures rather than
//! conversation.

use farcooler_agent_core::event::{EndReason, classify_error};

use super::end_reason;

/// The `SDKAssistantMessageError` that is an ending, not a failure.
pub(super) const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";

/// The `error` word on an assistant frame that reports a failed API call
/// rather than an answer (`SDKAssistantMessage.error` in the SDK's types).
/// The on-disk transcript marks the same record `isApiErrorMessage`.
///
/// `max_output_tokens` is the exception. It is in the same type, but it is not
/// a failure: the answer ran out of room, and the CLI's notice saying so is the
/// only explanation the person gets. The turn ends `MaxTokens`.
pub(super) fn api_error_of(frame: &serde_json::Value) -> Option<&str> {
    match frame["error"].as_str() {
        Some(MAX_OUTPUT_TOKENS) => None,
        Some(code) if !code.is_empty() => Some(code),
        _ if frame["isApiErrorMessage"].as_bool() == Some(true) => Some("unknown"),
        _ => None,
    }
}

/// Whether a `result` reports a turn that failed rather than one that ended.
///
/// `is_error` is the signal, and NOT `subtype`: a 401 arrives as
/// `"subtype":"success","is_error":true`. The `error_*` subtypes are failures
/// too.
pub(super) fn turn_failed(frame: &serde_json::Value) -> bool {
    frame["is_error"].as_bool() == Some(true)
        || frame["subtype"].as_str().is_some_and(|s| s.starts_with("error_"))
}

/// How the turn a `result` closes ended.
///
/// `code` is the word the turn's API-error `assistant` frame carried, when the
/// caller saw one. A Stop still reads as `Cancelled` whatever else is set.
pub(super) fn result_reason(frame: &serde_json::Value, code: Option<&str>) -> EndReason {
    if code == Some(MAX_OUTPUT_TOKENS) {
        return EndReason::MaxTokens;
    }
    let reason = end_reason(frame["stop_reason"].as_str().unwrap_or_default());
    if matches!(reason, EndReason::Cancelled | EndReason::MaxTokens) || !turn_failed(frame) {
        return reason;
    }
    let detail = match &frame["result"] {
        serde_json::Value::String(s) if !s.is_empty() => s.clone(),
        _ => frame["errors"]
            .as_array()
            .map(|e| e.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; "))
            .unwrap_or_default(),
    };
    let subtype = frame["subtype"].as_str().filter(|s| s.starts_with("error_"));
    let status = frame["api_error_status"].as_u64().and_then(|s| u16::try_from(s).ok());
    let kind = classify_error(code.filter(|c| *c != "unknown").or(subtype), status, &detail);
    EndReason::Failed { kind, detail }
}

#[cfg(test)]
mod tests {
    use farcooler_agent_core::event::{AgentEvent, EndReason, Role};

    use super::super::Live;

    #[test]
    fn running_out_of_output_room_is_max_tokens_and_keeps_its_notice() {
        // `max_output_tokens` is in `SDKAssistantMessageError` beside the real
        // failures. Read as one, a truncated answer lost the CLI's notice
        // saying why and ended `Failed`.
        let mut live = Live::default();
        let notice = serde_json::json!({
            "type": "assistant", "error": "max_output_tokens",
            "message": { "id": "m1", "model": "<synthetic>",
                         "content": [{ "type": "text", "text": "Claude's response exceeded the output token maximum." }] }
        });
        assert!(matches!(
            live.frame_to_events(&notice).as_slice(),
            [AgentEvent::Message { role: Role::Agent, .. }]
        ));
        let result = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": true, "stop_reason": null
        });
        assert_eq!(
            live.frame_to_events(&result),
            [AgentEvent::TurnEnded { reason: EndReason::MaxTokens }]
        );
        // And with no frame before it, a result that says so itself.
        let said = serde_json::json!({
            "type": "result", "is_error": true, "stop_reason": "max_tokens"
        });
        assert_eq!(
            Live::default().frame_to_events(&said),
            [AgentEvent::TurnEnded { reason: EndReason::MaxTokens }]
        );
    }

    #[test]
    fn an_overloaded_api_mid_turn_fails_the_turn_and_a_stop_does_not() {
        // A 529 after some answer has already streamed: no `error` word was
        // seen, so the status decides.
        let overloaded = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": true,
            "api_error_status": 529, "stop_reason": null,
            "result": "API Error: 529 Overloaded"
        });
        let mut live = Live::default();
        assert_eq!(
            live.frame_to_events(&overloaded),
            [AgentEvent::TurnEnded {
                reason: EndReason::Failed {
                    kind: farcooler_agent_core::event::FailureKind::Overloaded,
                    detail: "API Error: 529 Overloaded".into(),
                }
            }]
        );
        // The same error after a Stop is the Stop.
        live.interrupting();
        let stopped = serde_json::json!({
            "type": "result", "subtype": "error_during_execution", "is_error": true
        });
        assert_eq!(
            live.frame_to_events(&stopped),
            [AgentEvent::TurnEnded { reason: EndReason::Cancelled }]
        );
        // And only that turn: the flag does not outlive the result it was for.
        assert!(matches!(
            live.frame_to_events(&stopped).as_slice(),
            [AgentEvent::TurnEnded { reason: EndReason::Failed { .. } }]
        ));
    }
}

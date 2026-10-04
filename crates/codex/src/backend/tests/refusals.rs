//! Refusals the backend must report instead of swallowing (ov-142).

use super::*;
use farcooler_agent_core::event::AgentGapReason;

/// A server that refuses `thread/resume` with `message`, then starts a new
/// thread, then has no model list: the shape of a resume that failed.
async fn start_after_failed_resume(message: &str) -> Vec<AgentEvent> {
    let script = format!(
        r#"take; fix 1 1; take
take; printf '{{"id":2,"error":{{"code":-32603,"message":"{message}"}}}}\n'
take; printf '{{"id":3,"result":{{"thread":{{"id":"fresh","turns":[]}},"model":"m"}}}}\n'
take; printf '{{"id":4,"error":{{"code":-32601,"message":"no list"}}}}\n'
read -r done"#
    );
    let (launch, capture) = fake_launch(&script, "");
    let (_backend, prelude) = CodexBackend::start(&launch, capture.0.clone(), Some(THREAD.into()))
        .await
        .expect("a failed resume still opens a conversation");
    prelude
}

#[tokio::test]
async fn a_resume_the_server_refused_says_it_started_a_new_thread() {
    let prelude = start_after_failed_resume("thread store is corrupt").await;
    assert!(
        prelude.iter().any(|e| matches!(
            e,
            AgentEvent::Gap { reason: AgentGapReason::LoadFailed { detail } } if detail.contains("corrupt")
        )),
        "a silent fresh thread presents an empty chat as the one asked for: {prelude:?}"
    );
}

#[tokio::test]
async fn a_resume_with_nothing_recorded_is_the_empty_case_not_a_failure() {
    let prelude = start_after_failed_resume("no rollout found for thread id x").await;
    assert!(prelude.contains(&AgentEvent::Gap { reason: AgentGapReason::LoadEmpty }), "{prelude:?}");
    assert!(!prelude.iter().any(|e| matches!(e, AgentEvent::Gap { reason: AgentGapReason::LoadFailed { .. } })));
}

#[tokio::test]
async fn a_selector_the_backend_does_not_have_is_refused_not_acknowledged() {
    let (mut backend, _capture) = fake_app_server("read -r done", "").await;
    assert!(matches!(
        backend.set_config_option("verbosity", "high").await,
        Err(BackendError::Refused(_))
    ));
    backend.set_config_option("model", "m").await.expect("a real selector is accepted");
}

#[tokio::test]
async fn an_unrelated_not_found_is_a_failure_to_load() {
    let prelude = start_after_failed_resume("model not found").await;
    assert!(
        prelude.iter().any(|e| matches!(
            e,
            AgentEvent::Gap { reason: AgentGapReason::LoadFailed { detail } } if detail.contains("model not found")
        )),
        "{prelude:?}"
    );
}

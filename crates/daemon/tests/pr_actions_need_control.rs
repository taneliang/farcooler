//! `pr.ready` and `pr.thread_reply` act as the runner's own `gh` login, so they
//! are Control's (ov-317): a Read client is refused before anything is looked
//! up or run, whatever it sends, and a Control client's same request is not
//! refused for scope. Through the real dispatch table and scope check.

#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::capability::PR_ACTIONS;
use farcooler_protocol::v1::{self as pb, ErrorCode, Scope, request};
use farcooler_transport::{ClientError, request as request_for};
use in_process::*;

fn the_two() -> Vec<pb::Request> {
    let nobody = bytes::Bytes::copy_from_slice(uuid::Uuid::from_u128(7).as_bytes());
    let mut ready = request_for("pr.ready");
    ready.payload = Some(request::Payload::PrReady(pb::PrReady { lane_id: nobody.clone(), pr_number: 12 }));
    let mut reply = request_for("pr.thread_reply");
    reply.payload =
        Some(request::Payload::PrThreadReply(pb::PrThreadReply { task_id: nobody, body: "Hello".into(), send: true, note_id: Default::default(), digest: String::new() }));
    [ready, reply].into_iter().map(|mut r| {
        r.required_capabilities = vec![PR_ACTIONS.into()];
        r
    }).collect()
}

fn code_of(error: &ClientError) -> i32 {
    match error {
        ClientError::Daemon { code, .. } => *code,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[tokio::test]
async fn a_read_client_is_refused_both_before_anything_is_looked_up() {
    let h = start(Scope::Read).await;
    for req in the_two() {
        let method = req.method.clone();
        let refused = connect(&h).await.call(req).await.expect_err(&method);
        assert_eq!(code_of(&refused), ErrorCode::ScopeDenied as i32, "{method}: {refused:?}");
    }
}

#[tokio::test]
async fn a_control_client_is_past_the_scope_check() {
    let h = start(Scope::Control).await;
    for req in the_two() {
        let method = req.method.clone();
        // Nothing is on this runner to act on: the lane and the card are not
        // there, which is the next question, not the scope's.
        let refused = connect(&h).await.call(req).await.expect_err(&method);
        assert_eq!(code_of(&refused), ErrorCode::NotFound as i32, "{method}: {refused:?}");
    }
}

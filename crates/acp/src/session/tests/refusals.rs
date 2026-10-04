//! Requests this client cannot do are refused, never answered as success (ov-142).

use super::*;

#[tokio::test]
async fn a_method_this_client_does_not_implement_is_answered_method_not_found() {
    // It used to be answered `{}`, which an adapter reads as the method having
    // worked: an elicitation or a terminal request "succeeded" with nothing.
    let (wt, _) = worktree_and_outside("unhandled-method");
    let record = wt.with_file_name("record");
    let mut session = recorded_session(&wt, &record).await;
    let request = fs_request("terminal/create", serde_json::json!({ "command": "ls" }));
    session.handle(request).await.expect("handled");
    let frame = received(&record).await;
    assert_eq!(frame["id"], 7);
    assert!(frame.get("result").is_none(), "an unhandled request is not a result: {frame}");
    assert_eq!(frame["error"]["code"], METHOD_NOT_FOUND, "{frame}");
}

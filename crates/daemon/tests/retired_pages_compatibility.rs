#[path = "support/in_process.rs"]
mod in_process;

use farcooler_protocol::v1::{PlanGetRequest, Scope, request::Payload, result::Value};
use farcooler_transport::request;

#[tokio::test]
async fn an_older_app_can_open_a_board_without_the_pages_capability() {
    let harness = in_process::start(Scope::Control).await;
    let repo = in_process::a_repository(&harness);
    let mut app = in_process::connect(&harness).await;

    let capabilities = &app.server_hello().capabilities;
    assert!(capabilities.iter().any(|cap| cap == "board_plan"));
    // Released clients only list pages when the runner advertises this capability.
    assert!(!capabilities.iter().any(|cap| cap == "board_pages"));
    let mut plan = request("plan.get");
    plan.payload = Some(Payload::PlanGet(PlanGetRequest {
        workspace_id: bytes::Bytes::copy_from_slice(repo.workspace.as_bytes()),
        include_closed: false,
    }));
    let response = app.call(plan).await.unwrap();
    assert!(matches!(response.value, Some(Value::Plan(_))));
}

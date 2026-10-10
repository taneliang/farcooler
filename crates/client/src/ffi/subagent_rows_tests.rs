//! A subagent's own rows from an app (ov-453): `agent` on `agent.rows` and
//! `agent.rows_follow` reaches the runner as `agent_id`, naming
//! `subagent_rows`, so a runner without it refuses rather than answering
//! with the pane's rows; and one whose hello lacks it is never asked.

use serde_json::json;

use super::compose_upload_tests::a_recording_runner;
use super::dispatch;
use crate::session::Session;

fn agent_of(req: &farcooler_protocol::v1::Request) -> String {
    match &req.payload {
        Some(farcooler_protocol::v1::request::Payload::AgentRowsPage(p)) => p.agent_id.clone(),
        Some(farcooler_protocol::v1::request::Payload::AgentRowsFollow(p)) => p.agent_id.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_agents_rows_name_it_and_the_capability_they_need() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let seen = a_recording_runner(&socket, vec!["agent_rows".into(), "subagent_rows".into()]).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let terminal = uuid::Uuid::now_v7().to_string();
    // The recording runner answers with a value no rows call takes: only
    // what was sent matters here.
    let _ = dispatch(&session, "agent.rows", &json!({ "terminal": terminal, "agent": "a1" })).await;
    let _ = dispatch(&session, "agent.rows_follow", &json!({ "terminal": terminal, "agent": "a1", "waitMs": 0 })).await;
    let _ = dispatch(&session, "agent.rows", &json!({ "terminal": terminal })).await;
    let seen = seen.lock().unwrap().clone();
    let sent: Vec<(String, Vec<String>)> = seen.iter().map(|r| (agent_of(r), r.required_capabilities.clone())).collect();
    assert_eq!(
        sent,
        [("a1".into(), vec!["subagent_rows".to_string()]), ("a1".into(), vec!["subagent_rows".to_string()]), (String::new(), vec![])]
    );
}

#[tokio::test]
async fn a_runner_without_it_is_never_asked_for_an_agents_rows() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("r.sock");
    let seen = a_recording_runner(&socket, vec!["agent_rows".into()]).await;
    let session = Session::connect_local(&socket).await.expect("connect");
    let asked = dispatch(&session, "agent.rows", &json!({ "terminal": uuid::Uuid::now_v7().to_string(), "agent": "a1" })).await;
    assert!(asked.is_err(), "{asked:?}");
    assert!(seen.lock().unwrap().is_empty(), "refused before sending");
}

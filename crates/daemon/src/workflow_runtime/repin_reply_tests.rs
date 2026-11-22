//! ov-1373: a busy workspace's `workflow.get` stays under the 1 MiB reply cap, and `workspace re-pin` asks only for
//! the published workflow. Both lists grow without bound; the first fixture is the live machine's (766 cleanups).
use super::Runtime;
use super::wire_commands::bound_recent;
use farcooler_workflow::cleanup::CleanupEffect;
use farcooler_protocol::workflow_commands::Action;
use std::time::Duration;

const FRAME_CAP: usize = 1 << 20;

fn effect(workspace: &str, n: usize) -> CleanupEffect {
    CleanupEffect { id: format!("cleanup-{n:04}"), workspace: workspace.into(), revision: n as u64, item: format!("ov-{n}"), run: format!("run-{n}"), directory: None,
        restart: None, tree: None, folders: (0..12).map(|f| format!("/Users/someone/Library/Application Support/worktrees/overnight/ov-{n}/target/debug/build-{f}")).collect(), uncertain: false }
}

async fn busy() -> (impl Sized, tempfile::TempDir, std::sync::Arc<crate::service::Service>, uuid::Uuid, Runtime) {
    let (dir, svc, repo) = crate::test_support::fixture().await;
    let workspace = svc.store.ensure_main_workspace(repo).unwrap().id;
    let runtime_dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::create(&runtime_dir.path().join("runtime")).await.unwrap();
    for n in 0..800 { runtime.store.workflow_cleanup_enqueue(&effect(&workspace.to_string(), n)).unwrap(); }
    (dir, runtime_dir, svc, workspace, runtime)
}

#[test]
fn a_long_list_keeps_its_newest_entries_and_counts_the_rest() {
    let mut reply = serde_json::Map::new();
    bound_recent(&mut reply, "process_logs", (0..800).map(|n| serde_json::json!({"n": n, "tail": "x".repeat(300)})).collect());
    let kept = reply["process_logs"].as_array().unwrap();
    assert_eq!(kept.len(), 50);
    assert_eq!(kept[0]["n"], 750);
    assert_eq!(kept[49]["n"], 799);
    assert_eq!(reply["process_logs_omitted"], 750);
    let mut short = serde_json::Map::new();
    bound_recent(&mut short, "build_cleanup", vec![1, 2, 3]);
    assert_eq!(short["build_cleanup"], serde_json::json!([1, 2, 3]));
    assert_eq!(short["build_cleanup_omitted"], 0);
}

#[tokio::test]
async fn a_busy_workspaces_get_is_bounded_and_repin_asks_only_for_the_workflow() {
    use super::admission_tests::{factory, request, receipt};
    use farcooler_transport::{Handler, Peer};
    use farcooler_protocol::v1::Scope;
    let (_dir, _runtime_dir, svc, workspace, runtime) = busy().await;
    let ws = workspace.to_string();
    assert_eq!(runtime.store.workflow_cleanups(&ws).unwrap().len(), 800);
    let rpc = factory(&svc, &runtime, Peer { client_id: None, scope: Scope::Control }, None);

    let mut full_request = request(workspace, &Action::Get);
    full_request.accepts.push("workflow_full_v1".into());
    let reply = tokio::time::timeout(Duration::from_secs(5), rpc.handle(full_request.clone())).await.unwrap();
    let full = receipt(&reply);
    assert_eq!(full["build_cleanup"].as_array().unwrap().len(), 50);
    assert_eq!(full["build_cleanup_omitted"], 750);
    assert_eq!(full["build_cleanup"][49]["effect"]["id"], "cleanup-0799", "the newest survive");
    assert_eq!(full["process_logs_omitted"], 0);
    let size = full.to_string().len();
    println!("ov-1373 full get bytes={size} (800 cleanups unbounded was {})", serde_json::to_string(&runtime.store.workflow_cleanups(&ws).unwrap()).unwrap().len());
    assert!(size < FRAME_CAP / 4, "{size} bytes");

    full_request.accepts.push("workflow_repin_v1".into());
    let reply = tokio::time::timeout(Duration::from_secs(5), rpc.handle(full_request)).await.unwrap();
    let repin = receipt(&reply);
    let mut keys: Vec<_> = repin.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert!(keys.iter().all(|k| ["configuration", "context", "engine_codex_hooks", "repinned_adapter", "repinned_definition", "schema",
        "suggested_adapter", "suggested_adapter_refused"].contains(&k.as_str())), "{keys:?}");
    assert!(repin.get("state").is_none() && repin.get("build_cleanup").is_none() && repin.get("process_logs").is_none());
}

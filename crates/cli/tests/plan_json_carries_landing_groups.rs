//! The actual CLI bytes the Mac reads, held to a fixture consumed by its 320-pt Plan test.
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Handler, HandshakeConfig, Peer, UnixListenerServer};
use serde_json::{Value, json};
use uuid::Uuid;

const WORKSPACE: Uuid = Uuid::from_u128(0x202);
const REPO: Uuid = Uuid::from_u128(0x101);
fn id(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

#[derive(Clone)]
struct Runner;
impl Handler for Runner {
    fn peer(&self) -> Peer {
        Peer {
            client_id: None,
            scope: pb::Scope::Control,
        }
    }
    async fn handle(&self, req: pb::Request) -> pb::Response {
        let value = match req.method.as_str() {
            "repository.list" => result::Value::RepositoryList(pb::RepositoryList {
                items: vec![pb::Repository {
                    id: id(REPO),
                    display_name: "fixture".into(),
                    ..Default::default()
                }],
            }),
            "workspace.list" => result::Value::WorkspaceList(pb::WorkspaceList {
                items: vec![pb::Workspace {
                    id: id(WORKSPACE),
                    repository_id: id(REPO),
                    name: "Main".into(),
                    is_main: true,
                    ..Default::default()
                }],
            }),
            "plan.get" => {
                let Some(request::Payload::PlanGet(read)) = req.payload else {
                    panic!("plan payload")
                };
                assert_eq!(read.workspace_id, id(WORKSPACE));
                assert!(!read.include_closed);
                result::Value::Plan(pb::Plan { now_ms: 300_000, cards: vec![pb::PlanCard {
                    task_id: id(Uuid::from_u128(1)), key: "ov-1".into(), status: pb::TaskStatus::InReview as i32,
                    title: "Keep the complete long ticket title visible when the landing queue wraps on a narrow phone screen".into(),
                }], ..Default::default() })
            }
            "workflow.get" => {
                assert_eq!(req.target_resource_id, Some(id(WORKSPACE)));
                assert_eq!(
                    req.required_capabilities,
                    [farcooler_protocol::capability::WORKFLOW_COMMANDS]
                );
                let Some(request::Payload::WorkflowCommand(command)) = req.payload else {
                    panic!("workflow payload")
                };
                assert_eq!(
                    serde_json::from_str::<Value>(&command.json).unwrap(),
                    json!({ "action": "Get" })
                );
                result::Value::WorkflowReply(pb::WorkflowReply {
                    schema: 1,
                    json: workflow().to_string(),
                })
            }
            other => panic!("unexpected read {other}"),
        };
        pb::Response {
            request_id: req.request_id,
            outcome: Some(pb::response::Outcome::Result(pb::Result { value: Some(value) })),
        }
    }
}

fn workflow() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("../../client/src/testdata/workflow-presentation.json")).unwrap();
    let mut state = fixture[0]["request"]["snapshot"].clone();
    state["workspace"] = json!(WORKSPACE);
    state["revision"] = json!(20);
    let mut template = state["items"]["item"].clone();
    let stage = template["definition"]["stages"][0].clone();
    template["definition"]["schema"] = json!(2);
    template["definition"]["source"] = json!("group/landing");
    template["definition"]["max_reworks"] = json!(0);
    template["definition"]["rework_target"] = Value::Null;
    template["definition"]["stages"] = Value::Array(
        ["AwaitBase", "Fold", "Run", "Push", "AwaitCi"]
            .into_iter()
            .enumerate()
            .map(|(n, kind)| {
                let mut step = stage.clone();
                step["id"] = json!(format!("landing-{n}"));
                step["kind"] = json!(kind);
                step
            })
            .collect(),
    );
    let definition: farcooler_workflow::Definition = serde_json::from_value(template["definition"].clone()).unwrap();
    definition.validate_group().unwrap();
    template["pin"] = json!(definition.pin());
    state["items"] = json!({});
    for (n, phase, step, since) in [
        (1, "Running", 2, 60000),
        (2, "Blocked", 0, 120000),
        (3, "Running", 2, 180000),
        (4, "Failed", 2, 0),
    ] {
        let mut group = template.clone();
        group["group_of"] = json!({"name": "landing", "pin": "pin"});
        group["iteration"] = json!(n);
        group["stage"] = json!(step);
        group["phase"] = json!(phase);
        group["since_ms"] = json!(since);
        group["members"] = json!([{"item":"ov-1","candidate":"candidate"},{"item":"ov-2","candidate":"candidate"}]);
        if n >= 3 {
            group["group_failure"] =
                json!({"job":"cargo test --workspace", "firstLine":"error: expected a passing gate", "split": n == 3});
        }
        state["items"][format!("group-{n:020}-landing")] = group;
    }
    json!({"state": state, "context": {"binding":"fixture", "capabilities": {
        "workspace":WORKSPACE, "epoch":1, "inspect":true, "controller":true, "decision_principal":"owner"
    }}})
}

#[tokio::test]
async fn real_plan_json_reaches_the_shared_renderer_and_matches_the_mac_fixture() {
    let home = tempfile::Builder::new().prefix("plan1105-").tempdir_in("/tmp").unwrap();
    let socket = home.path().join("farcoolerd.sock");
    let listener = UnixListenerServer::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        listener
            .serve(|_| {
                Some((
                    HandshakeConfig {
                        daemon_version: "fixture".into(),
                    },
                    Runner,
                ))
            })
            .await
            .unwrap()
    });
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcooler"))
        .env("FARCOOLER_HOME", home.path())
        .env_remove("FARCOOLER_WORKSPACE")
        .env_remove("FARCOOLER_ACTOR")
        .args([
            "plan",
            "--json",
            "--repo",
            &REPO.to_string(),
            "--workspace",
            &WORKSPACE.to_string(),
        ])
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let plan: Value = serde_json::from_slice(&output.stdout).unwrap();
    let session = farcooler_client::session::Session::connect_local(&socket)
        .await
        .unwrap();
    assert_eq!(
        session.plan_with_workflow(WORKSPACE).await.unwrap(),
        plan,
        "the CLI and all Session-based apps must read the same enriched Plan"
    );
    server.abort();
    let answer: Value = serde_json::from_str(&farcooler_client::plan_view::json(
        &json!({
            "schema":1, "plan":plan, "context":{"style":"apple"}
        })
        .to_string(),
    ))
    .unwrap();
    assert!(answer["error"].is_null(), "{answer}");
    assert_eq!(answer["view"]["landingGroups"].as_array().unwrap().len(), 4);
    let queue: Value = serde_json::from_str(include_str!("../../../test/fixtures/landing-queue.json")).unwrap();
    assert_eq!(answer["view"]["landingGroups"], queue["expected"]);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/plan-landing-queue.json");
    if std::env::var_os("UPDATE_PLAN_LANDING_FIXTURE").is_some() {
        std::fs::write(&fixture, serde_json::to_string_pretty(&plan).unwrap() + "\n").unwrap();
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    assert_eq!(plan, expected, "the Mac fixture must remain actual plan --json output");
}

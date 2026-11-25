use super::*;
use crate::web_review::Request;
use farcooler_protocol::v1::ErrorCode;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as Process,
};

struct ReviewFixture {
    checkout: PathBuf,
    ticket: Uuid,
    key: String,
}

fn git(path: &Path, args: &[&str]) -> String {
    let out = Process::new("git")
        .current_dir(path)
        .args([
            "-c",
            "user.name=Review Fixture",
            "-c",
            "user.email=review@example.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

fn move_head(path: &Path) {
    git(path, &["checkout", "-qb", "replacement"]);
    fs::write(
        path.join("reports/previous.md"),
        "This candidate replaced the previous report.\n",
    )
    .unwrap();
    git(path, &["commit", "-qam", "Replace the candidate"]);
}

async fn fixture() -> (Harness, ReviewFixture) {
    use farcooler_store::models::{Actor, TaskUpdate};

    let mut fixture = None;
    let harness = start_with_service(|db, workspace, service| {
        let checkout = db.parent().unwrap().join("checkout");
        fs::create_dir_all(checkout.join("reports")).unwrap();
        git(&checkout, &["init", "-qb", "main"]);
        let brief = include_str!("../../../../../../../reports/ov-1446/README.md");
        let inventory = include_str!("../../../../../../../reports/ov-1439/parity.md");
        fs::write(checkout.join("reports/previous.md"), brief).unwrap();
        git(&checkout, &["add", "."]);
        git(
            &checkout,
            &["commit", "-qm", "Record the Web board rollout"],
        );
        git(&checkout, &["checkout", "-qb", "review/parity-follow-up"]);
        fs::write(
            checkout.join("reports/previous.md"),
            format!("{brief}\n{inventory}"),
        )
        .unwrap();
        fs::write(checkout.join("reports/context.md"), inventory).unwrap();
        fs::create_dir(checkout.join("samples")).unwrap();
        // Cross the byte and line limits independently in Git-generated patches.
        fs::write(
            checkout.join("samples/byte-limit.txt"),
            format!("{}\n", "x".repeat(1024)).repeat(2300),
        )
        .unwrap();
        fs::write(
            checkout.join("samples/line-limit.txt"),
            "added line\n".repeat(5000),
        )
        .unwrap();
        git(&checkout, &["add", "."]);
        git(
            &checkout,
            &[
                "commit",
                "-qm",
                "Add parity inventory to the Web rollout review",
            ],
        );
        let head = git(&checkout, &["rev-parse", "HEAD"]);

        let repository = service
            .store
            .get_workspace(workspace)
            .unwrap()
            .repository_id;
        let tree = service
            .store
            .create_worktree(
                repository,
                "review/parity-follow-up",
                checkout.to_str().unwrap(),
                false,
            )
            .unwrap();
        service.store.set_review_base(tree.id, "main").unwrap();
        let task = service
            .store
            .create_task_complete(
                workspace,
                &TaskUpdate {
                    title: "Web-first rollout: review the worktree parity inventory".into(),
                    intent: brief.into(),
                    acceptance: vec![],
                    constraints: vec![],
                    labels: vec![],
                    worktree_id: Some(tree.id),
                },
                Actor::User,
            )
            .unwrap();
        let store = farcooler_store::Store::open(db).unwrap();
        let definition = farcooler_workflow::Definition::tracker("owner");
        let receipt = store
            .workflow_apply(&Input {
                schema: 1,
                id: "review-fixture".into(),
                workspace: workspace.to_string(),
                expected_revision: 0,
                epoch: 1,
                source: "fixture".into(),
                source_sequence: 1,
                now_ms: 1000,
                grant: Grant::Controller,
                command: Command::Enqueue {
                    item: task.key.clone(),
                    definition,
                    policy_pin: "fixture".into(),
                    resource: "review-fixture".into(),
                    dependencies: Default::default(),
                    initial_candidate: Some(head),
                },
                busy_resources: Default::default(),
            })
            .unwrap();
        assert!(receipt.result.is_ok(), "{receipt:?}");
        fixture = Some(ReviewFixture {
            checkout,
            ticket: task.id,
            key: task.key,
        });
    })
    .await;
    (harness, fixture.unwrap())
}

#[tokio::test]
async fn web_review_socket_refuses_marks_for_byte_and_line_limited_diffs() {
    let (h, fixture) = fixture().await;
    let session = Session::connect_local(&h.socket).await.unwrap();
    let read = async |req| {
        session
            .web_review(
                h.workspace,
                fixture.ticket,
                serde_json::from_value(req).unwrap(),
            )
            .await
    };
    let initial = read(json!({"view":"changes"})).await.unwrap();
    for (path, reason) in [
        ("samples/byte-limit.txt", "exceeds the byte limit"),
        ("samples/line-limit.txt", "exceeds the line limit"),
    ] {
        let file = initial["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path)
            .unwrap();
        let view = read(json!({"view":"changes","position":file["position"]}))
            .await
            .unwrap();
        assert!(
            view["diff"]["notice"].as_str().unwrap().contains(reason),
            "{view}"
        );
        assert_eq!(view["diff"]["hunks"], json!([]));
        assert!(view["diff"]["next"].is_null());
        assert!(view["mark"].is_null());
        let mut position = view["position"].clone();
        // A saved or hand-edited offset beyond the oversized hunk returns an
        // empty uncapped page. It must still be refused.
        for from in [0, 1] {
            position["from"] = json!(from);
            assert!(
                matches!(
                    read(json!({"view":"changes","position":position,"edit":{"action":"review","reviewed":true}})).await,
                    Err(SessionError::Refused { code, what, .. })
                        if code == ErrorCode::InvalidArgument as i32 && what == "review_incomplete"
                ),
                "{path} at offset {from}"
            );
        }
    }
}

#[tokio::test]
async fn web_review_socket_checks_scope_exact_line_and_candidate_after_head_moves() {
    let (h, fixture) = fixture().await;
    let task = fixture.ticket;
    let workspace = h.workspace;
    let a = Session::connect_local(&h.socket).await.unwrap();
    let view = a
        .web_review(
            workspace,
            task,
            Request::Directory {
                path: "reports".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(view["entries"][1]["path"], "reports/previous.md");
    assert!(
        a.web_review(
            workspace,
            task,
            Request::File {
                path: "../workflow.sqlite".into()
            }
        )
        .await
        .is_err()
    );

    let changes = json!({"view":"changes"});
    let read = async |session: &Session, req: Value| {
        session
            .web_review(workspace, task, serde_json::from_value(req).unwrap())
            .await
    };
    let original = read(&a, changes.clone()).await.unwrap();
    assert_eq!(original["approval"]["action"]["label"], "Approve");
    let position = original["diff"]["hunks"][0]["lines"][0]["position"].clone();
    let note = json!({"view":"changes","position":position,"edit":{"action":"comment","body":"Please keep this context."}});
    let added = read(&a, note.clone()).await.unwrap();
    assert_eq!(
        added["diff"]["hunks"][0]["lines"][0]["comments"][0]["body"],
        "Please keep this context."
    );
    let mut unknown_line = note.clone();
    unknown_line["position"]["line"] = "incorrect anchor".into();
    assert!(
        matches!(read(&a, unknown_line).await, Err(SessionError::Refused { what, .. }) if what == "review_stale")
    );

    h.scope.store(Scope::Read as i32, Ordering::SeqCst);
    let reader = Session::connect_local(&h.socket).await.unwrap();
    let read_only = reader.web_ticket(workspace, task).await.unwrap();
    assert!(read_only["review"]["files"].is_null() && read_only["review"]["changes"].is_null());
    assert!(
        matches!(read(&reader, changes).await, Err(SessionError::Refused { code, .. })
        if code == ErrorCode::ScopeDenied as i32)
    );
    assert!(
        matches!(read(&reader, note.clone()).await, Err(SessionError::Refused { code, .. }) if code == ErrorCode::ScopeDenied as i32)
    );

    move_head(&fixture.checkout);
    assert!(
        matches!(read(&a, note).await, Err(SessionError::Refused { what, .. }) if what == "review_stale")
    );
    assert!(
        matches!(read(&a, json!({"view":"verify","candidate":position["candidate"]})).await,
        Err(SessionError::Refused { what, .. }) if what == "review_stale")
    );
    let replacement = read(&a, json!({"view":"changes","position":position}))
        .await
        .unwrap();
    assert!(replacement["approval"].is_null());
    assert_eq!(replacement["position"]["line"], "");
    assert!(replacement["notice"].is_string());
    assert_eq!(
        a.task_record(task)
            .await
            .unwrap()
            .notes
            .iter()
            .filter(|note| note.kind == farcooler_protocol::v1::TaskNoteKind::Comment as i32)
            .count(),
        1
    );
}

/// Private browser harness. The browser drives the production screen and
/// workflow journal against an authenticated fixture session over a Unix socket.
#[tokio::test]
#[ignore = "owned browser harness only"]
async fn web_review_browser_bridge() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let out = PathBuf::from(std::env::var_os("REVIEW_BROWSER_BRIDGE").unwrap());
    let (h, fixture) = fixture().await;
    let session = Session::connect_local(&h.socket).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let repo = h
        .service
        .store
        .get_workspace(h.workspace)
        .unwrap()
        .repository_id;
    fs::write(
        out.join("ready.json"),
        json!({
            "port": listener.local_addr().unwrap().port(), "workspace": h.workspace, "repo": repo,
            "ticket": fixture.ticket, "key": fixture.key,
        })
        .to_string(),
    )
    .unwrap();
    loop {
        let (stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(60), listener.accept())
                .await
                .unwrap()
                .unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = String::new();
        reader.read_line(&mut raw).await.unwrap();
        let body: Value = serde_json::from_str(&raw).unwrap();
        let method = body["method"].as_str().unwrap();
        let result = match method {
            "stop" => Ok(Value::Null),
            "move" => {
                move_head(&fixture.checkout);
                Ok(Value::Null)
            }
            "restore" => {
                git(
                    &fixture.checkout,
                    &["checkout", "-q", "review/parity-follow-up"],
                );
                Ok(Value::Null)
            }
            "form" => Ok(crate::web_board::form(None)),
            "ticket" => session.web_ticket(h.workspace, fixture.ticket).await,
            "intent" => {
                session
                    .web_board_intent(h.workspace, serde_json::from_value(body.clone()).unwrap())
                    .await
            }
            "review" => {
                session
                    .web_review(
                        h.workspace,
                        fixture.ticket,
                        serde_json::from_value(body["request"].clone()).unwrap(),
                    )
                    .await
            }
            _ => session.workflow_call(method, body["args"].clone()).await,
        };
        let reply = match result {
            Ok(value) => json!({"value": value}),
            Err(SessionError::Refused { code, what, .. }) => {
                json!({"error": "refused", "code": code, "what": what})
            }
            Err(error) => json!({"error": error.to_string()}),
        };
        reader
            .get_mut()
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
        if method == "stop" {
            break;
        }
    }
}

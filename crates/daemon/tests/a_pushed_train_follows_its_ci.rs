//! A pushed train follows its CI (ov-309), end to end: the real `farcoolerd`,
//! listening on its socket as a runner does, with a `gh` of this test's own
//! ahead on `PATH` that prints what a real `gh` printed for this repository
//! (`test/fixtures/ci/`).
//!
//! What this proves that the unit tests can't: that the daemon actually runs
//! the CI watch (`main.rs` starts it, and nothing else does), that giving a
//! train a SHA kicks it to read at once, that a short SHA is resolved through
//! `gh`, and that the read reaches `plan.get` with the train moved to red.

use std::time::{Duration, Instant};

use farcooler_protocol::capability::{BOARD_PLAN, BOARD_TRAINS, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as request_for};

mod common;

/// Long enough for the daemon to start, kick, run five `gh`s and write, on a
/// machine running the rest of the suite.
const BUDGET: Duration = Duration::from_secs(60);

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/ci")
}

/// A `gh` that answers the CI watch's four reads from the recorded fixtures,
/// with shell builtins only (the daemon runs gh inside an exec allowlist).
/// Anything else it's asked fails, as a logged-out gh would.
fn gh_shim(dir: &std::path::Path) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let gh = bin.join("gh");
    let body = format!(
        r#"#!/bin/sh
emit() {{ while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "{dir}/$1"; }}
[ "$1 $2 $3" = "api -X GET" ] || exit 1
case "$4" in
  "repos/{{owner}}/{{repo}}/commits/c85bf83d") emit commit-sha.txt ;;
  "repos/{{owner}}/{{repo}}/actions/runs") emit runs-sha-failed.json ;;
  "repos/{{owner}}/{{repo}}/actions/runs/37275435256/jobs") emit jobs-failed.json ;;
  "repos/{{owner}}/{{repo}}/actions/runs/"*"/jobs") printf '[]\n' ;;
  *) exit 1 ;;
esac
"#,
        dir = fixtures().display()
    );
    std::fs::write(&gh, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

async fn call(client: &mut SocketClient, mut r: pb::Request, capability: &str) -> result::Value {
    r.required_capabilities = vec![capability.into()];
    let method = r.method.clone();
    client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value.expect("a value")
}

#[tokio::test]
async fn a_pushed_train_turns_red_with_its_ci() {
    std::fs::create_dir_all("/tmp/fc-t").unwrap();
    let dir = tempfile::Builder::new().prefix("ci").tempdir_in("/tmp/fc-t").unwrap();
    let bin = gh_shim(dir.path());
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let root = dir.path().join("repos");
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        assert!(std::process::Command::new("git").args(&args).current_dir(&repo).status().unwrap().success());
    }

    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let _daemon = common::listening_daemon_with_env(&home, &[("PATH", &path)]).await;
    let mut client = Client::connect(&home.join("farcoolerd.sock"), "test-client", "0.0.0").await.expect("connect");

    let mut add = request_for("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(pb::RepositoryRootAdd {
        absolute_path: root.to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("repository_root.add");
    let mut register = request_for("repository.register");
    register.payload =
        Some(request::Payload::RepositoryRegister(pb::RepositoryRegister { relative_path: repo.to_string_lossy().into_owned() }));
    client.call(register).await.expect("repository.register");

    let result::Value::WorkspaceList(list) = call(&mut client, request_for("workspace.list"), WORKSTREAMS).await else {
        panic!("wrong result")
    };
    let workspace = list.items.iter().find(|w| w.is_main).expect("a Main workspace").id.clone();

    let mut start = request_for("train.start");
    start.payload = Some(request::Payload::TrainStart(pb::TrainStart {
        workspace_id: workspace.clone(),
        name: "integ-9".into(),
        base: "origin/main".into(),
        lane_ids: vec![],
        actor: "manager".into(),
    }));
    let result::Value::BoardTrain(train) = call(&mut client, start, BOARD_TRAINS).await else { panic!("wrong result") };
    let mut push = request_for("train.set");
    push.payload = Some(request::Payload::TrainSet(pb::TrainSet {
        train_id: train.id.clone(),
        sha: Some("c85bf83d".into()),
        actor: "manager".into(),
        ..Default::default()
    }));
    call(&mut client, push, BOARD_TRAINS).await;

    let deadline = Instant::now() + BUDGET;
    let plan = loop {
        let mut get = request_for("plan.get");
        get.payload = Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: workspace.clone(), include_closed: true }));
        let result::Value::Plan(plan) = call(&mut client, get, BOARD_PLAN).await else { panic!("wrong result") };
        if !plan.ci.is_empty() || Instant::now() > deadline {
            break plan;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert_eq!(plan.ci.len(), 1, "the watch read the train's SHA: {plan:?}");
    let read = &plan.ci[0];
    assert_eq!(read.subject, "sha:c85bf83d");
    assert_eq!(read.sha, "c85bf83dce46a6b71d7312afc623899ae7914658", "resolved through gh");
    assert_eq!(read.status, pb::BoardCiStatus::Failed as i32);
    let swift = read.jobs.iter().find(|j| j.name == "CI / Swift (shared + macOS)").expect("the CI run's jobs");
    assert_eq!(swift.state, "failed");
    assert_eq!(plan.trains[0].state, pb::BoardTrainState::Red as i32, "the train follows its CI");
}

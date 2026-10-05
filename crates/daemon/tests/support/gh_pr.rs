//! A real `farcoolerd` on its socket with a `gh` of the test's own ahead on
//! `PATH`, answering `pr list` and the GraphQL read with what a real `gh`
//! printed in shape (`test/fixtures/pr-stage/`), for the lane PR stage tests
//! (ov-312).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_protocol::capability::{BOARD_PLAN, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as request_for};

/// Long enough for the daemon to start, run the watch and write, on a machine
/// running the rest of the suite.
pub const BUDGET: Duration = Duration::from_secs(60);

pub type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/pr-stage")
}

/// The fixture's list with each card's key in place of `@N@` (card N is PR
/// 100 + N), split as the daemon asks for it: the open PRs, and the closed and
/// merged ones.
pub fn pr_lists(keys: &[String]) -> (String, String) {
    let mut text = std::fs::read_to_string(fixtures().join("pr-list.json")).unwrap();
    for (i, key) in keys.iter().enumerate() {
        text = text.replace(&format!("@{}@", i + 1), key);
    }
    let all: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    let (open, closed): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| p["state"] == "OPEN");
    (serde_json::to_string(&open).unwrap(), serde_json::to_string(&closed).unwrap())
}

/// Give the shim `keys`' PRs to list.
pub fn serve(dir: &Path, keys: &[String]) {
    let (open, closed) = pr_lists(keys);
    std::fs::write(dir.join("pr-list-open.json"), open).unwrap();
    std::fs::write(dir.join("pr-list-closed.json"), closed).unwrap();
}

/// A `gh` with shell builtins only (the daemon runs it inside an exec
/// allowlist). `pr list --state open|closed` prints `<dir>/pr-list-open.json` or
/// `-closed.json`; every call fails as a logged-out gh does while
/// `<dir>/offline` exists, and the GraphQL read alone fails while
/// `<dir>/graphql-fails` does. The GraphQL read prints the recorded answer for
/// the PR it is asked about. Every call is logged to `<dir>/calls.log`, a line
/// each: `pr list <state>` or `graphql <number>`.
pub fn gh_shim(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let gh = bin.join("gh");
    let body = format!(
        r#"#!/bin/sh
emit() {{ while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "$1"; }}
case "$1 $2" in
  "pr list")
    st=open
    for a in "$@"; do [ "$a" = closed ] && st=closed; done
    echo "pr list $st" >> "{dir}/calls.log"
    if [ -e "{dir}/offline" ]; then echo 'gh: To get started with GitHub CLI, please run: gh auth login' >&2; exit 4; fi
    emit "{dir}/pr-list-$st.json" ;;
  "api graphql")
    n=none
    for a in "$@"; do case "$a" in number=*) n="${{a#number=}}" ;; esac; done
    echo "graphql $n" >> "{dir}/calls.log"
    if [ -e "{dir}/offline" ] || [ -e "{dir}/graphql-fails" ]; then echo 'gh: HTTP 502' >&2; exit 1; fi
    if [ -e "{fix}/extras-$n.json" ]; then emit "{fix}/extras-$n.json"; else emit "{fix}/extras-none.json"; fi ;;
  "repo view") echo '{{"defaultBranchRef":{{"name":"main"}},"url":"https://github.example/o/r"}}' ;;
  *) exit 1 ;;
esac
"#,
        dir = dir.display(),
        fix = fixtures().display()
    );
    std::fs::write(&gh, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// How many `pr list` calls the shim has logged.
pub fn lists(dir: &Path) -> usize {
    calls(dir).iter().filter(|l| l.starts_with("pr list")).count()
}

/// The calls the shim has logged, oldest first.
pub fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(str::to_string).collect()
}

pub async fn call(client: &mut SocketClient, mut r: pb::Request, capability: &str) -> result::Value {
    r.required_capabilities = vec![capability.into()];
    let method = r.method.clone();
    client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value.expect("a value")
}

/// A runner holding one registered repository, with the shim ahead on `PATH`:
/// its scratch directory, the daemon, its client, and its Main workspace.
pub async fn a_runner() -> (tempfile::TempDir, crate::common::DaemonChild, SocketClient, pb::Workspace) {
    std::fs::create_dir_all("/tmp/fc-t").unwrap();
    let dir = tempfile::Builder::new().prefix("prs").tempdir_in("/tmp/fc-t").unwrap();
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
    let daemon = crate::common::listening_daemon_with_env(&home, &[("PATH", &path)]).await;
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
    let main = list.items.iter().find(|w| w.is_main).expect("a Main workspace").clone();
    (dir, daemon, client, main)
}

/// A card on `workspace`, by its key.
pub async fn a_card(client: &mut SocketClient, workspace: &pb::Workspace, title: &str) -> pb::Task {
    let mut req = request_for("task.create");
    req.payload = Some(request::Payload::TaskCreate(pb::TaskCreate {
        repository_id: workspace.repository_id.clone(),
        workspace_id: Some(workspace.id.clone()),
        title: title.into(),
        ..Default::default()
    }));
    req.required_capabilities = vec![WORKSTREAMS.into()];
    let result::Value::Task(task) = client.call(req).await.expect("task.create").value.expect("a value") else {
        panic!("wrong result")
    };
    task
}

/// A lane on `workspace` working `cards`, moved to `state` along the arrows.
pub async fn a_lane(
    client: &mut SocketClient,
    workspace: &pb::Workspace,
    name: &str,
    cards: &[&pb::Task],
    path: &[pb::LaneState],
) -> pb::Lane {
    let mut req = request_for("lane.create");
    req.payload = Some(request::Payload::LaneCreate(pb::LaneCreate {
        workspace_id: workspace.id.clone(),
        name: name.into(),
        reason: "Because.".into(),
        cards: cards.iter().map(|t| pb::LaneCard { task_id: t.id.clone(), slice: String::new(), stage: None }).collect(),
        branch: format!("lane/{name}"),
        harness: "claude".into(),
        model: "sonnet".into(),
        agent: Some(pb::LaneAgentRecord {
            harness: "claude".into(),
            agent_id: format!("agent-{name}"),
            role: pb::LaneAgentRole::Build as i32,
            model: None,
            ended: false,
        }),
        actor: "manager".into(),
        ..Default::default()
    }));
    let result::Value::Lane(mut lane) = call(client, req, BOARD_PLAN).await else { panic!("wrong result") };
    for state in path {
        lane = move_lane(client, &lane, *state).await;
    }
    lane
}

pub async fn move_lane(client: &mut SocketClient, lane: &pb::Lane, state: pb::LaneState) -> pb::Lane {
    let mut req = request_for("lane.update");
    req.payload = Some(request::Payload::LaneUpdate(pb::LaneUpdate {
        lane_id: lane.id.clone(),
        state: Some(state as i32),
        actor: "manager".into(),
        ..Default::default()
    }));
    let result::Value::Lane(lane) = call(client, req, BOARD_PLAN).await else { panic!("wrong result") };
    lane
}

pub async fn plan(client: &mut SocketClient, workspace: &pb::Workspace) -> pb::Plan {
    let mut get = request_for("plan.get");
    get.payload = Some(request::Payload::PlanGet(pb::PlanGetRequest { workspace_id: workspace.id.clone(), include_closed: true }));
    let result::Value::Plan(plan) = call(client, get, BOARD_PLAN).await else { panic!("wrong result") };
    plan
}

/// Read the plan until `done` holds of it, or fail with the last read.
pub async fn until(client: &mut SocketClient, workspace: &pb::Workspace, what: &str, done: impl Fn(&pb::Plan) -> bool) -> pb::Plan {
    let deadline = Instant::now() + BUDGET;
    loop {
        let read = plan(client, workspace).await;
        if done(&read) {
            return read;
        }
        assert!(Instant::now() < deadline, "{what}; the last plan read: {read:#?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

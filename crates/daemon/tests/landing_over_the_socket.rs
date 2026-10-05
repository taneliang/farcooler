//! How a repository lands work (ov-313), end to end: a real `farcoolerd` with a
//! `gh` of the test's own that prints recorded answers
//! (`test/fixtures/landing/`), asked `repository.landing` over its socket. Each
//! mode decision is one test, and what a read found never changes what a board
//! chose.

use farcooler_protocol::capability::{LANDING, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as request_for};

mod common;

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/landing")
}

/// A `gh` printing these three recorded answers, shell builtins only (the
/// daemon runs gh inside an exec allowlist). Anything else it's asked fails.
fn gh_shim(dir: &std::path::Path, rules: &str, branch: &str, repo: &str) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let body = format!(
        r#"#!/bin/sh
emit() {{ while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "{dir}/$1"; }}
case "$1 $2 $3 $4" in
  "repo view --json "*) emit {repo} ;;
  "api --paginate -X GET") [ "$5" = "repos/{{owner}}/{{repo}}/rules/branches/main?per_page=100" ] || exit 1; printf '['; emit {rules}; printf ']\n' ;;
  "api -X GET repos/{{owner}}/{{repo}}/branches/main") emit {branch} ;;
  *) exit 1 ;;
esac
"#,
        dir = fixtures().display()
    );
    let gh = bin.join("gh");
    std::fs::write(&gh, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

struct Runner {
    _dir: tempfile::TempDir,
    _daemon: common::DaemonChild,
    client: SocketClient,
    repository: bytes::Bytes,
}

/// A runner holding one repository whose `main` carries `workflow`, behind a `gh`
/// printing the named recorded answers.
async fn a_runner(rules: &str, branch: &str, repo: &str, workflow: &str) -> Runner {
    std::fs::create_dir_all("/tmp/fc-t").unwrap();
    let dir = tempfile::Builder::new().prefix("landing").tempdir_in("/tmp/fc-t").unwrap();
    let bin = gh_shim(dir.path(), rules, branch, repo);
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = dir.path().join("repos");
    let checkout = root.join("demo");
    std::fs::create_dir_all(checkout.join(".github/workflows")).unwrap();
    std::fs::write(checkout.join(".github/workflows/ci.yml"), workflow).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["add", "-A"],
        vec!["commit", "-q", "-m", "base"],
    ] {
        assert!(std::process::Command::new("git").args(&args).current_dir(&checkout).status().unwrap().success());
    }
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let daemon = common::listening_daemon_with_env(&home, &[("PATH", &path)]).await;
    let mut client = Client::connect(&home.join("farcoolerd.sock"), "test-client", "0.0.0").await.expect("connect");

    let mut add = request_for("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(pb::RepositoryRootAdd {
        absolute_path: root.to_string_lossy().into_owned(),
        typed_confirmation: String::new(),
    }));
    client.call(add).await.expect("repository_root.add");
    let mut register = request_for("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(pb::RepositoryRegister {
        relative_path: checkout.to_string_lossy().into_owned(),
    }));
    let result::Value::Repository(repository) = call(&mut client, register, WORKSTREAMS).await else {
        panic!("wrong result")
    };
    Runner { _dir: dir, _daemon: daemon, client, repository: repository.id }
}

async fn call(client: &mut SocketClient, mut r: pb::Request, capability: &str) -> result::Value {
    r.required_capabilities = vec![capability.into()];
    let method = r.method.clone();
    client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value.expect("a value")
}

impl Runner {
    async fn landing(&mut self) -> pb::RepositoryLanding {
        let mut r = request_for("repository.landing");
        r.target_resource_id = Some(self.repository.clone());
        let result::Value::RepositoryLanding(l) = call(&mut self.client, r, LANDING).await else { panic!("wrong result") };
        l
    }

    async fn main_workspace(&mut self) -> pb::Workspace {
        let mut list = request_for("workspace.list");
        list.target_resource_id = Some(self.repository.clone());
        let result::Value::WorkspaceList(l) = call(&mut self.client, list, WORKSTREAMS).await else { panic!("wrong result") };
        l.items.into_iter().find(|w| w.is_main).expect("a Main workspace")
    }
}

const PLAIN: &str = "name: CI\non:\n  push:\n  pull_request:\njobs: {}\n";
const WITH_QUEUE: &str = "name: CI\non:\n  pull_request:\n  merge_group:\njobs: {}\n";

#[tokio::test]
async fn an_unprotected_main_suggests_direct() {
    let mut r = a_runner("rules-unprotected.json", "branch-unprotected.json", "repo-view-admin.json", PLAIN).await;
    let l = r.landing().await;
    assert_eq!(l.suggested, pb::LandingMode::Direct as i32);
    assert!(!l.direct_impossible);
    assert_eq!(l.base, "main");
    let f = l.facts.unwrap();
    assert_eq!(f.branch_protected, Some(false));
    assert_eq!(f.merge_method.as_deref(), Some("squash"));
    assert_eq!(f.viewer_permission.as_deref(), Some("ADMIN"));
    assert_eq!((f.codeowners, f.merge_group_workflow), (Some(false), Some(false)));
}

#[tokio::test]
async fn a_pull_request_rule_refuses_direct_over_the_wire() {
    let mut r = a_runner("rules-pull-request.json", "branch-protected.json", "repo-view-admin.json", PLAIN).await;
    let l = r.landing().await;
    assert_eq!(l.suggested, pb::LandingMode::PullRequests as i32);
    assert!(l.direct_impossible);
    assert_eq!(l.reasons[0], "main requires pull requests, with 2 approving reviews.");
    assert_eq!(l.facts.unwrap().required_checks, ["rust", "swift"]);
}

#[tokio::test]
async fn a_merge_queue_with_no_merge_group_workflow_warns() {
    let mut r = a_runner("rules-merge-queue.json", "branch-protected.json", "repo-view-admin.json", PLAIN).await;
    let l = r.landing().await;
    assert!(l.direct_impossible);
    assert_eq!(l.warnings, ["The merge queue will wait forever: no workflow runs on `merge_group`."]);
    assert_eq!(l.facts.unwrap().merge_queue, Some(true));
}

#[tokio::test]
async fn a_merge_queue_with_a_merge_group_workflow_does_not() {
    let mut r = a_runner("rules-merge-queue.json", "branch-protected.json", "repo-view-admin.json", WITH_QUEUE).await;
    let l = r.landing().await;
    assert!(l.direct_impossible);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    assert_eq!(l.facts.unwrap().merge_group_workflow, Some(true));
}

#[tokio::test]
async fn a_protected_main_whose_rules_are_admin_only_suggests_pull_requests() {
    let mut r = a_runner("rules-unprotected.json", "branch-protected.json", "repo-view-admin.json", PLAIN).await;
    let l = r.landing().await;
    assert_eq!(l.suggested, pb::LandingMode::PullRequests as i32);
    assert!(!l.direct_impossible, "details unknown is not a refusal");
}

#[tokio::test]
async fn a_rule_restricting_updates_refuses_direct_over_the_wire() {
    let mut r = a_runner("rules-update.json", "branch-unprotected.json", "repo-view-admin.json", PLAIN).await;
    let l = r.landing().await;
    assert!(l.direct_impossible);
    assert_eq!(l.suggested, pb::LandingMode::PullRequests as i32);
}

#[tokio::test]
async fn a_read_only_login_cannot_land_direct() {
    let mut r = a_runner("rules-unprotected.json", "branch-unprotected.json", "repo-view-reader.json", PLAIN).await;
    let l = r.landing().await;
    assert!(l.direct_impossible);
    assert_eq!(l.facts.unwrap().viewer_permission.as_deref(), Some("READ"));
}

/// What a read found is said beside the board and never becomes its choice;
/// the board's own setting is written only by a person.
#[tokio::test]
async fn a_read_is_said_beside_the_board_and_does_not_choose_for_it() {
    let mut r = a_runner("rules-pull-request.json", "branch-protected.json", "repo-view-admin.json", PLAIN).await;
    let before = r.main_workspace().await;
    assert_eq!((before.landing, before.direct_refused.clone()), (None, None));
    r.landing().await;
    let after = r.main_workspace().await;
    assert_eq!(after.landing, None, "detection did not switch the board");
    assert_eq!(
        after.direct_refused.as_deref(),
        Some("Landing straight on main won't work. main requires pull requests, with 2 approving reviews.")
    );

    let mut set = request_for("workspace.set_settings");
    set.target_resource_id = Some(after.id.clone());
    set.payload = Some(request::Payload::WorkspaceSetSettings(pb::WorkspaceSetSettings {
        landing: Some(pb::LandingMode::PullRequests as i32),
        pr_cost_line: Some(true),
        expected_version: Some(after.resource_version),
        ..Default::default()
    }));
    set.required_capabilities = vec![LANDING.into(), farcooler_protocol::capability::WAKE_ON_ANSWER.into()];
    let result::Value::Workspace(chosen) = call(&mut r.client, set, WORKSTREAMS).await else { panic!("wrong result") };
    assert_eq!(chosen.landing, Some(pb::LandingMode::PullRequests as i32));
    assert_eq!((chosen.pr_cost_line, chosen.direct_refused), (Some(true), None));
}

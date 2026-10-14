//! The first run's survey (ov-519), end to end: a real `farcoolerd` with a
//! `gh` of the test's own on its `PATH`, asked `repository.survey` over its
//! socket, then `workspace.write_charter` for a draft and for the charter.
//! And the Suggest Themes prompt reaches a client in the runner's hello.

use farcooler_protocol::capability::{REPO_SURVEY, WORKSTREAMS};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as request_for};

mod common;

type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/landing")
}

/// A `gh` printing recorded answers with shell builtins only (the daemon runs
/// gh inside an exec allowlist): a main whose rules ask for pull requests,
/// and no merged pull requests. Anything else it's asked fails.
fn gh_shim(dir: &std::path::Path) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let body = format!(
        r#"#!/bin/sh
emit() {{ while IFS= read -r line || [ -n "$line" ]; do printf '%s\n' "$line"; done < "{dir}/$1"; }}
case "$1 $2" in
  "repo view") emit repo-view-admin.json ;;
  "pr list") printf '[]\n' ;;
  "api --paginate") printf '['; emit rules-pull-request.json; printf ']\n' ;;
  "api -X") emit branch-protected.json ;;
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
    dir: tempfile::TempDir,
    _daemon: common::DaemonChild,
    client: SocketClient,
    repository: bytes::Bytes,
}

async fn call(client: &mut SocketClient, mut r: pb::Request, capability: &str) -> result::Value {
    r.required_capabilities = vec![capability.into()];
    let method = r.method.clone();
    client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value.expect("a value")
}

/// A runner holding one Rust repository with CI and CODEOWNERS, behind the shim.
async fn a_runner() -> Runner {
    std::fs::create_dir_all("/tmp/fc-t").unwrap();
    let dir = tempfile::Builder::new().prefix("survey").tempdir_in("/tmp/fc-t").unwrap();
    let bin = gh_shim(dir.path());
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = dir.path().join("repos");
    let checkout = root.join("demo");
    std::fs::create_dir_all(checkout.join(".github/workflows")).unwrap();
    std::fs::write(checkout.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
    std::fs::write(checkout.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    std::fs::write(checkout.join("CODEOWNERS"), "* @ana\n").unwrap();
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
    Runner { dir, _daemon: daemon, client, repository: repository.id }
}

impl Runner {
    async fn survey(&mut self) -> pb::RepositorySurvey {
        let mut r = request_for("repository.survey");
        r.target_resource_id = Some(self.repository.clone());
        let result::Value::RepositorySurvey(s) = call(&mut self.client, r, REPO_SURVEY).await else { panic!("wrong result") };
        s
    }

    async fn main_workspace(&mut self) -> pb::Workspace {
        let mut list = request_for("workspace.list");
        list.target_resource_id = Some(self.repository.clone());
        let result::Value::WorkspaceList(l) = call(&mut self.client, list, WORKSTREAMS).await else { panic!("wrong result") };
        l.items.into_iter().find(|w| w.is_main).expect("a Main workspace")
    }

    async fn write(&mut self, p: pb::WorkspaceWriteCharter) -> pb::CharterWritten {
        let main = self.main_workspace().await;
        let mut r = request_for("workspace.write_charter");
        r.target_resource_id = Some(main.id);
        r.payload = Some(request::Payload::WorkspaceWriteCharter(p));
        let result::Value::CharterWritten(w) = call(&mut self.client, r, REPO_SURVEY).await else { panic!("wrong result") };
        w
    }
}

#[tokio::test]
async fn a_survey_over_the_socket_cites_github_and_the_files() {
    let mut r = a_runner().await;
    let s = r.survey().await;
    let answer = |key: &str| s.answers.iter().find(|a| a.key == key).unwrap_or_else(|| panic!("no {key}")).clone();
    let workflow = answer("workflow");
    assert_eq!(workflow.value, "pull_requests");
    assert_eq!(workflow.signal, "main requires pull requests, with 2 approving reviews");
    assert_eq!(answer("done_means").signal, "1 GitHub workflow, Cargo.toml");
    assert_eq!(answer("lanes").signal, "", "a default carries no signal");
    let g = s.github.unwrap();
    assert!(g.installed && g.answered, "{g:?}");
    let h = s.history.unwrap();
    assert_eq!((h.empty, h.commits), (false, 1));
    for heading in ["## Workflow", "## Done means", "## Review", "## Who decides", "## Reaching me", "## Lanes", "## Autonomy", "## Anything else"] {
        assert!(s.charter_draft.lines().any(|l| l == heading), "{heading}: {}", s.charter_draft);
    }
    assert!(s.took_ms < 10_000, "{}", s.took_ms);
}

#[tokio::test]
async fn a_draft_then_the_charter_are_written_and_a_charter_is_never_replaced() {
    let mut r = a_runner().await;
    let main = r.main_workspace().await;
    let charter = std::path::PathBuf::from(main.charter_path.expect("the charter's path, to a local client"));
    let draft = charter.with_file_name("charter-draft.md");

    let w = r.write(pb::WorkspaceWriteCharter { draft: true, ..Default::default() }).await;
    assert!(w.wrote && w.draft);
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), w.text);
    assert!(!charter.exists(), "a draft is not the charter");

    let chosen = pb::WorkspaceWriteCharter {
        landing: Some(pb::LandingMode::Direct as i32),
        lanes: Some(0),
        initiative: Some(false),
        ..Default::default()
    };
    let w = r.write(chosen).await;
    assert!(w.wrote && !w.draft);
    let text = std::fs::read_to_string(&charter).unwrap();
    assert_eq!(text, w.text);
    assert!(text.contains("## Workflow\nDirect: work lands on main"), "{text}");
    assert!(text.contains("As many agents at once as the work needs"), "{text}");
    assert!(text.contains("Initiative: off."), "{text}");
    assert!(!draft.exists(), "the draft outlived the charter");

    let again = r.write(pb::WorkspaceWriteCharter { lanes: Some(1), ..Default::default() }).await;
    assert!(!again.wrote);
    assert_eq!(std::fs::read_to_string(&charter).unwrap(), text, "a charter was replaced");
    drop(r.dir);
}

#[tokio::test]
async fn a_choice_the_runner_does_not_know_is_refused() {
    let mut r = a_runner().await;
    let main = r.main_workspace().await;
    let mut req = request_for("workspace.write_charter");
    req.target_resource_id = Some(main.id);
    req.required_capabilities = vec![REPO_SURVEY.into()];
    req.payload = Some(request::Payload::WorkspaceWriteCharter(pb::WorkspaceWriteCharter {
        landing: Some(0),
        ..Default::default()
    }));
    let e = r.client.call(req).await.expect_err("an unspecified landing");
    assert!(format!("{e:?}").contains("landing"), "{e:?}");
}

#[tokio::test]
async fn the_hello_carries_the_suggest_themes_prompt() {
    let mut r = a_runner().await;
    let result::Value::Host(host) = call(&mut r.client, request_for("host.get"), WORKSTREAMS).await else {
        panic!("wrong result")
    };
    assert_eq!(host.suggest_themes_prompt, farcooler_daemon::skill_install::SUGGEST_THEMES_PROMPT);
    assert!(host.suggest_themes_prompt.starts_with("Suggest themes"));
}

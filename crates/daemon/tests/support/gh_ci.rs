//! A real `farcoolerd` on its socket with a `gh` of the test's own ahead on
//! `PATH`, printing what a real `gh` printed for this repository
//! (`test/fixtures/ci/`): what the CI watch's end-to-end tests share
//! (ov-309, ov-306).

use std::time::Duration;

use farcooler_protocol::capability::WORKSTREAMS;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::{Client, request as request_for};

/// Long enough for the daemon to start, kick, run five `gh`s and write, on a
/// machine running the rest of the suite.
pub const BUDGET: Duration = Duration::from_secs(60);

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
  "repos/{{owner}}/{{repo}}/actions/runs")
    case "$6" in
      branch=*) emit runs-main.json ;;
      *) emit runs-sha-failed.json ;;
    esac ;;
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

pub type SocketClient = Client<tokio::net::unix::OwnedReadHalf, tokio::net::unix::OwnedWriteHalf>;

pub async fn call(client: &mut SocketClient, mut r: pb::Request, capability: &str) -> result::Value {
    r.required_capabilities = vec![capability.into()];
    let method = r.method.clone();
    client.call(r).await.unwrap_or_else(|e| panic!("{method}: {e:?}")).value.expect("a value")
}


/// A runner holding one registered repository, the client on its socket, and
/// its Main workspace's id. Hold the first two for the test's length.
pub async fn a_runner() -> (tempfile::TempDir, crate::common::DaemonChild, SocketClient, bytes::Bytes) {
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
    let workspace = list.items.iter().find(|w| w.is_main).expect("a Main workspace").id.clone();
    (dir, daemon, client, workspace)
}

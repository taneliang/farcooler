//! A repository's charter is adopted by the daemon, and never by a session.
//!
//! Adopting reads `<main checkout>/.farcooler/manager.md` for every
//! repository. That's a stat on every volume a repository lives on, and a
//! repository on an unmounted or slow network volume can hold it for the
//! mount's timeout. A `--stream` process is started per terminal a phone
//! opens, and a `--stdio` process per ssh connection, so work done while
//! either opens its `Service` is work between a phone and its first byte.
//!
//! So only the daemon adopts, at its start (and `register_repository`, for
//! the one repository it registers). This drives the real binary in all three
//! modes against one runtime directory, and checks the charter after each.
//! The daemon's half is the control: without it, a charter that no mode ever
//! adopted would pass the other two.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use farcooler_protocol::v1::{request, result, RepositoryRegister, RepositoryRootAdd};
use farcooler_transport::request;

mod common;
use common::{listening_daemon, spawn, TmuxReaper};

/// A git repository at `path` with one empty commit.
fn repository_at(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        let status = std::process::Command::new("git").args(&args).current_dir(path).status().unwrap();
        assert!(status.success(), "git {args:?}");
    }
}

/// The one workspace home under `home`: Main's, made at registration.
fn mains_charter(home: &Path) -> PathBuf {
    let homes: Vec<PathBuf> = std::fs::read_dir(home.join("workspaces"))
        .expect("registration made Main's home")
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(homes.len(), 1, "{homes:?}");
    homes[0].join("charter.md")
}

#[tokio::test]
async fn a_stream_or_stdio_session_adopts_no_charter_and_the_daemon_does() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    std::fs::create_dir_all(&home).unwrap();
    let _reaper = TmuxReaper::new(&home);
    let repo = dir.path().join("repo");
    repository_at(&repo);

    // Registered before the repository has a charter, so registration has
    // nothing to adopt and anything adopted later came from a start.
    {
        let (_child, mut client) = spawn(&home).await;
        let mut add = request("repository_root.add");
        add.payload = Some(request::Payload::RepositoryRootAdd(RepositoryRootAdd {
            absolute_path: dir.path().to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        }));
        client.call(add).await.expect("repository_root.add");
        let mut register = request("repository.register");
        register.payload = Some(request::Payload::RepositoryRegister(RepositoryRegister {
            relative_path: repo.to_string_lossy().into_owned(),
        }));
        let registered = client.call(register).await.expect("repository.register");
        assert!(matches!(registered.value, Some(result::Value::Repository(_))));
    }
    let charter = mains_charter(&home);
    assert!(!charter.exists(), "the repository had no charter to adopt");
    std::fs::create_dir_all(repo.join(".farcooler")).unwrap();
    std::fs::write(repo.join(".farcooler/manager.md"), "ship small").unwrap();

    // A stdio session with no daemon to relay to opens its own `Service`,
    // and the handshake is answered only once that's open.
    {
        let (_child, mut client) = spawn(&home).await;
        client.call(request("repository.list")).await.expect("repository.list");
    }
    assert!(!charter.exists(), "a stdio session adopted a charter");

    // A stream opens its `Service` before it looks for the terminal, so an
    // unknown one still gets that far.
    // Bounded, so a stream that waited instead of exiting fails here rather
    // than hanging the suite.
    let stream = tokio::process::Command::new(env!("CARGO_BIN_EXE_farcoolerd"))
        .args(["--stream", &uuid::Uuid::now_v7().to_string()])
        .env("FARCOOLER_HOME", &home)
        .env("FARCOOLER_CONFIG", home.join("config.toml"))
        .env("FARCOOLER_TEST_STUB_AGENTS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();
    let stream = tokio::time::timeout(Duration::from_secs(30), stream)
        .await
        .expect("a stream of an unknown terminal exits")
        .expect("run farcoolerd --stream");
    let said = String::from_utf8_lossy(&stream.stderr);
    assert!(said.contains("cannot stream that terminal"), "it never opened the service: {said}");
    assert!(!charter.exists(), "a stream adopted a charter");

    // The daemon adopts it at its start. Polled, because it may do so after
    // it's listening: a slow volume mustn't hold up the socket either.
    let _daemon = listening_daemon(&home).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !charter.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(std::fs::read_to_string(&charter).expect("the daemon adopted it"), "ship small");
}

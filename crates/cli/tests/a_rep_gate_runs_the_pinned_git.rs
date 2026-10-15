//! The rep gate's Read check runs the git it is told (`--git`), never one
//! found through the pane's `PATH`, which a checkout can influence (ov-555).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

const REAL_GIT: &str = "/usr/bin/git";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new(REAL_GIT)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// The gate's exit code for a Read of `a.txt`, with `fake` first on its `PATH`.
fn read(checkout: &Path, fake: &Path, git: &[&str]) -> i32 {
    let cli = env!("CARGO_BIN_EXE_farcooler");
    let mut cmd = Command::new(cli);
    cmd.args(["rep-gate", "--checkout"]).arg(checkout).args(["--cli", cli]);
    cmd.args(git);
    let path = format!("{}:/usr/bin:/bin", fake.display());
    let mut child = cmd
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let call = serde_json::json!({ "tool_name": "Read", "tool_input": { "file_path": checkout.join("a.txt") } });
    child.stdin.take().unwrap().write_all(call.to_string().as_bytes()).unwrap();
    child.wait().unwrap().code().unwrap()
}

#[test]
fn a_git_planted_on_the_path_is_not_the_one_that_runs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap().join("repo");
    let fake = std::fs::canonicalize(dir.path()).unwrap().join("bin");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&fake).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "hi").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "x"]);
    // A git that records that it ran, and says every file is tracked.
    let marker = dir.path().join("planted-ran");
    let script = format!("#!/bin/sh\ntouch '{}'\necho a.txt\n", marker.display());
    std::fs::write(fake.join("git"), script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(fake.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    assert_eq!(read(&repo, &fake, &["--git", REAL_GIT]), 0, "the pinned git says a.txt is tracked");
    assert!(!marker.exists(), "the planted git ran though --git named another");

    // The control: with no --git the planted one is what the PATH finds, so
    // the test above is about the pin and not about a PATH that skips it.
    read(&repo, &fake, &[]);
    assert!(marker.exists(), "the planted git is not first on the PATH, so this test proves nothing");
}

/// A gate told no git, or a git it can't use, refuses the Read; it never
/// falls back to the `PATH`, where the planted git would say yes.
#[test]
fn a_gate_with_no_usable_git_refuses_and_runs_no_other() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap().join("repo");
    let fake = std::fs::canonicalize(dir.path()).unwrap().join("bin");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&fake).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), "hi").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "x"]);
    let marker = dir.path().join("planted-ran");
    let script = format!("#!/bin/sh\ntouch '{}'\necho a.txt\n", marker.display());
    std::fs::write(fake.join("git"), script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(fake.join("git"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert_eq!(read(&repo, &fake, &["--no-git"]), 2, "no git named");
    assert_eq!(read(&repo, &fake, &["--git", "git"]), 2, "a git that isn't an absolute path");
    assert!(!marker.exists(), "the planted git ran");
}

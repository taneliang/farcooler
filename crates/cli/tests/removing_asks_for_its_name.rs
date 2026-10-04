//! What the Mac reads off the CLI when a removal needs the typed name.
//!
//! `DaemonClient.removeWorktree` and `removeRoot` ask whether stderr carries
//! `code: confirmation-required`. Both commands used to turn the daemon's
//! refusal into a bare string, so that line was never printed, and a dirty
//! worktree could not be removed from the Mac at all while every Swift test,
//! fed a hand-typed `code:` line, stayed green. This runs the real binary
//! against a real scratch daemon and reads what it really prints.
//!
//! Needs tmux, as every daemon test here does (CI installs it).

use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch {
    home: PathBuf,
}

impl Scratch {
    fn new() -> Scratch {
        // Short: the daemon's socket lives under it, and a unix socket path
        // has about a hundred bytes.
        let home = PathBuf::from(format!("/tmp/fcr-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        Scratch { home }
    }

    fn run(&self, args: &[&str]) -> (bool, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_farcooler"))
            .env("FARCOOLER_HOME", self.home.join("h"))
            .args(args)
            .output()
            .expect("run farcooler");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.run(&["daemon", "stop"]);
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_dirty_worktree_and_a_wrong_root_name_are_refused_with_the_confirmation_word() {
    let s = Scratch::new();
    let repos = s.home.join("repos");
    let demo = repos.join("demo");
    std::fs::create_dir_all(&demo).unwrap();
    git(&demo, &["init", "-q"]);
    git(&demo, &["commit", "-q", "--allow-empty", "-m", "init"]);

    let (ok, _, e) = s.run(&["--json", "daemon", "ensure"]);
    assert!(ok, "{e}");
    let (ok, _, e) = s.run(&["root", "add", repos.to_str().unwrap()]);
    assert!(ok, "{e}");
    let (ok, _, e) = s.run(&["repo", "register", demo.to_str().unwrap()]);
    assert!(ok, "{e}");
    let (ok, out, e) = s.run(&["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"]);
    assert!(ok, "{e}");
    let made: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let short = made["short"].as_str().unwrap().to_string();
    std::fs::write(PathBuf::from(made["worktree"].as_str().unwrap()).join("dirty.txt"), "x").unwrap();

    // The first call of the Mac's remove sheet: no name yet.
    let (ok, _, e) = s.run(&["--json", "worktree", "remove", &short]);
    assert!(!ok);
    assert!(e.lines().any(|l| l.trim() == "code: confirmation-required"), "no word in:\n{e}");

    // A root removed with the wrong name.
    let (_, roots, _) = s.run(&["root", "list"]);
    let root = roots.split_whitespace().next().unwrap().to_string();
    let (ok, _, e) = s.run(&["--json", "root", "remove", &root, "--confirm", "wrong"]);
    assert!(!ok);
    assert!(e.lines().any(|l| l.trim() == "code: confirmation-required"), "no word in:\n{e}");

    // And the right name goes through, so the word is the whole difference.
    let (ok, _, e) = s.run(&["--json", "worktree", "remove", &short, "--confirm", "wt1"]);
    assert!(ok, "{e}");
}

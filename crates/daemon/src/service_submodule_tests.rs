//! The codex manager skill in a worktree whose `.git` is a file (ov-202).
//!
//! A submodule's checkout, like a `--separate-git-dir` one, has a `.git` file,
//! and the skill used to be skipped there with only a warning, because the
//! exclude line could not be written.

use super::*;
use crate::skill_install::{Harness, PROJECT_SKILL, PROJECT_SKILL_FILES, PROJECT_SKILL_POLICY};

fn git_in(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A real submodule at `<base>/super/mod`, made by git.
fn submodule(base: &Path) -> (PathBuf, PathBuf) {
    for name in ["lib", "super"] {
        std::fs::create_dir_all(base.join(name)).unwrap();
        git_in(&base.join(name), &["init", "-q", "-b", "main"]);
        git_in(&base.join(name), &["commit", "-q", "--allow-empty", "-m", "first"]);
    }
    let sup = std::fs::canonicalize(base.join("super")).unwrap();
    let lib = std::fs::canonicalize(base.join("lib")).unwrap();
    git_in(&sup, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", lib.to_str().unwrap(), "mod"]);
    (sup.clone(), std::fs::canonicalize(sup.join("mod")).unwrap())
}

/// A launch of codex in a submodule's checkout leaves both skill files and
/// ignores them, in the submodule's own record.
#[tokio::test]
async fn a_submodule_checkout_keeps_the_codex_skill() {
    let dir = tempfile::tempdir().unwrap();
    let (sup, module) = submodule(dir.path());
    assert!(module.join(".git").is_file(), "a submodule's .git is a file");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    install_project_skill(&module, Harness::Codex, deadline).await;
    for relative in PROJECT_SKILL_FILES {
        assert!(module.join(relative).is_file(), "{relative} was left out of the submodule");
    }
    let exclude = std::fs::read_to_string(sup.join(".git/modules/mod/info/exclude")).unwrap();
    assert!(exclude.contains(&format!("/{PROJECT_SKILL}")), "{exclude}");
    assert!(exclude.contains(&format!("/{PROJECT_SKILL_POLICY}")), "{exclude}");
}

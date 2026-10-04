//! `exclude_locally` for a worktree whose `.git` is a file: a submodule and a
//! `--separate-git-dir` checkout (ov-202). Their git dir is their own common
//! dir, so the main-checkout test used to open `.git` as a directory and fail
//! with ENOTDIR. A `.git` file is the agent's to rewrite, so what proves the
//! git dir is this worktree's is the git dir's own `core.worktree`.

use std::path::{Path, PathBuf};

use super::tests::{exclude, git_in, ours, repo};

/// `<base>/super` with a real submodule at `<base>/super/mod`, made by git.
fn with_submodule(base: &Path) -> (PathBuf, PathBuf) {
    let lib = repo(&base.join("lib"));
    let sup = repo(&base.join("super"));
    git_in(&sup, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", lib.to_str().unwrap(), "mod"]);
    (sup.clone(), std::fs::canonicalize(sup.join("mod")).unwrap())
}

/// A submodule's worktree gets the exclude line, in the superproject's
/// record of the submodule.
#[tokio::test]
async fn a_submodule_checkout_gets_its_exclude_line() {
    let dir = tempfile::tempdir().unwrap();
    let (sup, module) = with_submodule(dir.path());
    assert!(module.join(".git").is_file(), "a submodule's .git is a file");
    assert!(exclude(&module).await, "refused a real submodule");
    let file = sup.join(".git/modules/mod/info/exclude");
    assert_eq!(ours(&std::fs::read_to_string(file).unwrap()), 1);
    assert!(exclude(&module).await, "again");
}

/// A `git init --separate-git-dir` store: the checkout at `<base>/<name>`
/// and its git dir at `<base>/<name>-store.git`.
fn separate(base: &Path, name: &str) -> (PathBuf, PathBuf) {
    let tree = base.join(name);
    let store = base.join(format!("{name}-store.git"));
    std::fs::create_dir_all(&tree).unwrap();
    git_in(&tree, &["init", "-q", "-b", "main", "--separate-git-dir", store.to_str().unwrap()]);
    git_in(&tree, &["commit", "-q", "--allow-empty", "-m", "first"]);
    (std::fs::canonicalize(&tree).unwrap(), store)
}

/// A `--separate-git-dir` checkout records nothing that ties its store to it,
/// so it can't be told from a borrowed one and is refused: the skill is left
/// out, and no exclude line is written.
#[tokio::test]
async fn a_separate_git_dir_checkout_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (tree, store) = separate(dir.path(), "tree");
    assert!(tree.join(".git").is_file());
    assert!(!exclude(&tree).await);
    assert_eq!(ours(&std::fs::read_to_string(store.join("info/exclude")).unwrap_or_default()), 0);
}

/// The attack: an agent's `.git` file names another repository's real
/// separate git dir. Git accepts it (`rev-parse` names that store for both),
/// and nothing in the store says the agent's tree is not its own.
#[tokio::test]
async fn a_dot_git_file_naming_another_separate_git_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (_victim, store) = separate(dir.path(), "victim");
    let attacker = dir.path().join("attacker");
    std::fs::create_dir_all(&attacker).unwrap();
    std::fs::write(attacker.join(".git"), format!("gitdir: {}\n", store.display())).unwrap();
    let attacker = std::fs::canonicalize(&attacker).unwrap();
    assert!(!exclude(&attacker).await, "written through another repository's store");
    assert_eq!(ours(&std::fs::read_to_string(store.join("info/exclude")).unwrap_or_default()), 0);
}

/// An agent rewrites its checkout's `.git` file to name another repository's
/// git dir. Nothing in that git dir says this worktree is its own, so it is
/// refused, whether the other repository is a plain one or one with a
/// `.git` file of its own.
#[tokio::test]
async fn a_dot_git_file_naming_another_repository_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let plain = repo(&dir.path().join("plain"));
    let (sup, module) = with_submodule(&dir.path().join("sub"));
    let bare = dir.path().join("bare.git");
    std::fs::create_dir_all(&bare).unwrap();
    git_in(&bare, &["init", "-q", "--bare"]);
    for (name, target) in [
        ("plain", plain.join(".git")),
        ("submodule", sup.join(".git/modules/mod")),
        ("bare", bare.clone()),
    ] {
        let wt = dir.path().join(format!("wt-{name}"));
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", target.display())).unwrap();
        let wt = std::fs::canonicalize(&wt).unwrap();
        assert!(!exclude(&wt).await, "{name}: written through a borrowed git dir");
        let file = target.join("info/exclude");
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert_eq!(ours(&text), 0, "{name}: {} was written", file.display());
    }
    assert!(module.join(".git").is_file());
}

/// `.git` a link to a real git-dir file or directory is refused as ever.
#[tokio::test]
async fn a_dot_git_file_is_never_followed_through_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let (sup, module) = with_submodule(dir.path());
    let real = std::fs::read(module.join(".git")).unwrap();
    std::fs::remove_file(module.join(".git")).unwrap();
    std::fs::write(dir.path().join("elsewhere"), real).unwrap();
    std::os::unix::fs::symlink(dir.path().join("elsewhere"), module.join(".git")).unwrap();
    assert!(!exclude(&module).await, "a linked .git file was followed");
    assert_eq!(ours(&std::fs::read_to_string(sup.join(".git/modules/mod/info/exclude")).unwrap_or_default()), 0);
}

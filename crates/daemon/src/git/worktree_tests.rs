use super::*;

/// Parsing is exercised through a real repository, because the shape of
/// `--porcelain` output is the thing under test and a hand-written fixture
/// would only prove I can copy it.
#[tokio::test]
async fn lists_the_main_checkout_and_every_linked_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git(&repo, &args).await.unwrap();
    }

    let extra = dir.path().join("side");
    git(&repo, &["worktree", "add", "-q", "-b", "feat/side", extra.to_str().unwrap()])
        .await
        .unwrap();

    let found = list_worktrees(&repo).await.unwrap();
    assert_eq!(found.len(), 2, "main checkout plus the linked one: {found:?}");

    let main = &found[0];
    assert!(main.is_main, "the first record is always the main checkout");
    assert_eq!(main.branch.as_deref(), Some("main"));

    let side = &found[1];
    assert!(!side.is_main);
    assert_eq!(side.branch.as_deref(), Some("feat/side"), "refs/heads/ is stripped");
    assert!(!side.head.is_empty());
    assert!(!side.locked && !side.prunable);
}

#[tokio::test]
async fn a_detached_worktree_has_no_branch_rather_than_a_fake_one() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git(&repo, &args).await.unwrap();
    }
    let extra = dir.path().join("detached");
    git(&repo, &["worktree", "add", "-q", "--detach", extra.to_str().unwrap()])
        .await
        .unwrap();

    let found = list_worktrees(&repo).await.unwrap();
    let detached = found.iter().find(|w| w.path.ends_with("detached")).expect("found");
    assert_eq!(detached.branch, None);
    assert!(!detached.head.is_empty(), "it still has a commit");
}

/// A repository and one linked worktree beside it, `side`, on a new branch.
async fn a_linked_worktree(dir: &Path) -> (PathBuf, PathBuf) {
    let repo = dir.join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        git(&repo, &args).await.unwrap();
    }
    let side = dir.join("side");
    git(&repo, &["worktree", "add", "-q", "-b", "side", side.to_str().unwrap()]).await.unwrap();
    (repo, side)
}

/// The mark names the install that wrote it, whichever that was. The
/// comparison against this install's id is the caller's
/// (`forked_this_worktree`), and its own test writes another install's
/// mark.
#[tokio::test]
async fn forked_by_names_the_install_that_marked_it_and_nothing_unmarked() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, side) = a_linked_worktree(dir.path()).await;

    assert_eq!(forked_by(&side), None, "unmarked");
    assert_eq!(forked_by(&repo), None, "a main checkout has a .git directory, not a file");
    mark_forked(&side, "install-b").await;
    assert_eq!(forked_by(&side).as_deref(), Some("install-b"));
}

/// git 2.48 and later can write the pointer relative to the worktree
/// (`worktree.useRelativePaths`). Read against the worktree, not against
/// whatever directory the daemon happens to run in.
#[tokio::test]
async fn forked_by_follows_a_relative_gitdir() {
    let dir = tempfile::tempdir().unwrap();
    let (_repo, side) = a_linked_worktree(dir.path()).await;
    mark_forked(&side, "install-a").await;

    let pointer = std::fs::read_to_string(side.join(".git")).unwrap();
    let admin = pointer.trim().strip_prefix("gitdir:").unwrap().trim().to_string();
    let tail = Path::new(&admin)
        .strip_prefix(dir.path().canonicalize().unwrap())
        .or_else(|_| Path::new(&admin).strip_prefix(dir.path()))
        .expect("the admin dir is under the test's directory")
        .to_path_buf();
    std::fs::write(side.join(".git"), format!("gitdir: ../{}\n", tail.display())).unwrap();

    assert_eq!(forked_by(&side).as_deref(), Some("install-a"));
}

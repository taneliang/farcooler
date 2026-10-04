//! `exclude_locally` against real repositories, and against the ways an agent
//! in a worktree could point the append somewhere else (ov-187's re-review).
//!
//! Each hostile case keeps a file outside the repository and checks it byte
//! for byte afterwards. The case-folded spellings alias only on a
//! case-insensitive volume (APFS by default); elsewhere they are ordinary
//! names and the cases pass without saying much.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use super::exclude_locally;

pub(super) const LINE: &str = ".agents/skills/farcooler-manager/SKILL.md";
const PRECIOUS: &str = "[user]\n\tname = precious\n";

pub(super) fn git_in(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository with one commit, so a worktree can be added.
pub(super) fn repo(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path).unwrap();
    git_in(path, &["init", "-q", "-b", "main"]);
    git_in(path, &["commit", "-q", "--allow-empty", "-m", "first"]);
    // Resolved, as the daemon's paths are, so `/var` and `/private/var` agree.
    std::fs::canonicalize(path).unwrap()
}

/// The repository at `<base>/repo` and a linked worktree of it at `<base>/wt`.
fn linked(base: &Path) -> (PathBuf, PathBuf) {
    let repo = repo(&base.join("repo"));
    git_in(&repo, &["worktree", "add", "-q", "-b", "wt", base.join("wt").to_str().unwrap()]);
    (repo, std::fs::canonicalize(base.join("wt")).unwrap())
}

/// A file outside every repository, the target an agent would aim at.
fn victim(base: &Path) -> PathBuf {
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let file = outside.join("gitconfig");
    std::fs::write(&file, PRECIOUS).unwrap();
    file
}

pub(super) async fn exclude(worktree: &Path) -> bool {
    exclude_locally(worktree, LINE, tokio::time::Instant::now() + std::time::Duration::from_secs(10)).await
}

fn untouched(file: &Path) {
    assert_eq!(std::fs::read_to_string(file).unwrap(), PRECIOUS, "{} was written", file.display());
}

pub(super) fn ours(text: &str) -> usize {
    text.lines().filter(|l| *l == format!("/{LINE}")).count()
}

/// The ordinary case still works, once, in a main checkout and in a linked
/// worktree whose common dir has no `info` yet.
#[tokio::test]
async fn the_line_lands_in_the_repositorys_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(&dir.path().join("main"));
    assert!(exclude(&main).await);
    assert!(exclude(&main).await, "again");
    assert_eq!(ours(&std::fs::read_to_string(main.join(".git/info/exclude")).unwrap()), 1);

    let (repo, wt) = linked(dir.path());
    std::fs::remove_dir_all(repo.join(".git/info")).unwrap();
    assert!(exclude(&wt).await, "a linked worktree");
    assert_eq!(ours(&std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap()), 1);
}

/// `info/exclude` is a symbolic link, in either spelling.
#[tokio::test]
async fn a_linked_exclude_file_is_refused() {
    for name in ["exclude", "EXCLUDE", "Exclude"] {
        let dir = tempfile::tempdir().unwrap();
        let victim = victim(dir.path());
        let main = repo(&dir.path().join("main"));
        let _ = std::fs::remove_file(main.join(".git/info/exclude"));
        symlink(&victim, main.join(".git/info").join(name)).unwrap();
        exclude(&main).await;
        untouched(&victim);
    }
}

/// `info` is a symbolic link to a directory that holds an `exclude`.
#[tokio::test]
async fn a_linked_info_directory_is_refused() {
    for name in ["info", "INFO", "Info"] {
        let dir = tempfile::tempdir().unwrap();
        let victim = victim(dir.path());
        std::fs::rename(&victim, victim.with_file_name("exclude")).unwrap();
        let victim = victim.with_file_name("exclude");
        let main = repo(&dir.path().join("main"));
        std::fs::remove_dir_all(main.join(".git/info")).unwrap();
        symlink(victim.parent().unwrap(), main.join(".git").join(name)).unwrap();
        exclude(&main).await;
        untouched(&victim);
    }
}

/// `info/exclude` is a hard link to a file elsewhere: no link to refuse, one
/// inode with two names.
#[tokio::test]
async fn a_hard_linked_exclude_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let victim = victim(dir.path());
    let main = repo(&dir.path().join("main"));
    let _ = std::fs::remove_file(main.join(".git/info/exclude"));
    std::fs::hard_link(&victim, main.join(".git/info/exclude")).unwrap();
    assert!(!exclude(&main).await);
    untouched(&victim);
}

/// The worktree's `.git` rewritten to name a git dir of the agent's making,
/// whose `commondir` is another repository: git then reports that one's
/// directory, and its `info/exclude` is an ordinary file.
#[tokio::test]
async fn a_dot_git_pointing_at_another_repository_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (_repo, wt) = linked(dir.path());
    let other = repo(&dir.path().join("other"));
    std::fs::write(other.join(".git/info/exclude"), PRECIOUS).unwrap();
    let fake = wt.join("fake2");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(fake.join("commondir"), format!("{}\n", other.join(".git").display())).unwrap();
    std::fs::write(fake.join("gitdir"), format!("{}\n", wt.join(".git").display())).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", fake.display())).unwrap();
    assert!(!exclude(&wt).await);
    untouched(&other.join(".git/info/exclude"));
}

/// The re-review's reproduction: `commondir` names a directory in the
/// worktree, whose `info/exclude` links out.
#[tokio::test]
async fn a_planted_common_dir_with_a_linked_exclude_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let victim = victim(dir.path());
    let (_repo, wt) = linked(dir.path());
    let fake = wt.join("fake2");
    let common = wt.join("fakecommon");
    for d in [fake.clone(), common.join("objects"), common.join("refs"), common.join("info")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(fake.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(fake.join("commondir"), "../fakecommon\n").unwrap();
    symlink(&victim, common.join("info/exclude")).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", fake.display())).unwrap();
    assert!(!exclude(&wt).await);
    untouched(&victim);
}

/// A main checkout whose `.git` is a link to another repository's.
#[tokio::test]
async fn a_linked_dot_git_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let main = repo(&dir.path().join("main"));
    let other = repo(&dir.path().join("other"));
    std::fs::write(other.join(".git/info/exclude"), PRECIOUS).unwrap();
    std::fs::remove_dir_all(main.join(".git")).unwrap();
    symlink(other.join(".git"), main.join(".git")).unwrap();
    assert!(!exclude(&main).await);
    untouched(&other.join(".git/info/exclude"));
}

/// The `.git` pointer spelled in NFC, the link on disk in NFD: APFS opens the
/// one by the other, and the link leads to another repository's git dir.
#[tokio::test]
async fn a_dot_git_pointer_through_an_nfd_link_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (_repo, wt) = linked(dir.path());
    let other = repo(&dir.path().join("other"));
    std::fs::write(other.join(".git/info/exclude"), PRECIOUS).unwrap();
    symlink(other.join(".git"), wt.join("e\u{301}")).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt.join("\u{e9}").display())).unwrap();
    assert!(!exclude(&wt).await);
    untouched(&other.join(".git/info/exclude"));
}

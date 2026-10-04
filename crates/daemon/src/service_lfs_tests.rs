//! A worktree says which large files weren't downloaded, and can try again
//! (ov-199): the daemon records the paths a new worktree left as pointers,
//! reports the count, and `worktree.hydrate_lfs` retries through a throwaway
//! index, leaving the agent's staged work and files alone.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::git_lfs::{LIMIT, WRITTEN, helper};
use crate::service::Service;
use crate::test_support::fixture;

/// A plain git, LFS off, as the fixtures of `git_lfs`'s own tests run it.
fn plain(dir: &Path, args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    let out = cmd
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "filter.lfs.process=", "-c", "filter.lfs.required=false", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn oid_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn pointer_text(bytes: &[u8]) -> String {
    format!("version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n", oid_of(bytes), bytes.len())
}

/// Put `bytes` in `repo`'s local LFS store, where the helper looks.
fn store_object(repo: &Path, bytes: &[u8]) {
    let oid = oid_of(bytes);
    let at = repo.join(".git/lfs/objects").join(&oid[0..2]).join(&oid[2..4]);
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join(&oid), bytes).unwrap();
}

/// Some content a few hundred bytes long, and not all one byte.
fn small(seed: u8) -> Vec<u8> {
    (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed).collect()
}

/// A registered repository whose `big.bin` is committed as a pointer, with
/// the object in the store only when `stored`. `None` when this build has no
/// helper to hydrate with (`--lib` builds none), skipped out loud.
async fn lfs_repo(
    test: &str,
    content: &[u8],
    stored: bool,
) -> Option<(crate::test_support::ScratchDir, std::sync::Arc<Service>, Uuid, PathBuf)> {
    if helper().is_none() {
        assert!(std::env::var_os("CI").is_none(), "{test}: no farcooler-lfs-filter beside the test binary");
        eprintln!("SKIP {test}: no farcooler-lfs-filter beside the test binary (`cargo test -p farcooler-daemon` builds it)");
        return None;
    }
    let (dir, svc, repo_id) = fixture().await;
    let repo = dir.path().join("repo");
    std::fs::write(repo.join(".gitattributes"), "*.bin filter=lfs -text\n").unwrap();
    std::fs::write(repo.join("big.bin"), pointer_text(content)).unwrap();
    if stored {
        store_object(&repo, content);
    }
    plain(&repo, &["add", "-A"]);
    plain(&repo, &["commit", "-q", "-m", "lfs"]);
    Some((dir, svc, repo_id, repo))
}

fn git_dir(worktree: &Path) -> PathBuf {
    let text = std::fs::read_to_string(worktree.join(".git")).unwrap();
    PathBuf::from(text.trim_start_matches("gitdir: ").trim_end())
}

async fn make(svc: &Service, repo: Uuid) -> farcooler_store::models::Worktree {
    svc.create_worktree(repo, "feature", "feature", "HEAD").await.unwrap()
}

#[tokio::test]
async fn a_worktree_made_with_a_missing_object_records_the_pointer_and_says_so_on_the_wire() {
    let content = small(1);
    let Some((_dir, svc, repo_id, _repo)) = lfs_repo("a_worktree_made_with_a_missing_object", &content, false).await
    else {
        return;
    };
    let ws = make(&svc, repo_id).await;
    assert_eq!(svc.store.lfs_pointer_paths(ws.id).unwrap(), ["big.bin"]);
    let view = svc.worktree_view(&ws).await.unwrap();
    assert_eq!(view.lfs_pointers, 1);
    let wire = crate::wire::worktree(&view, farcooler_protocol::v1::Scope::Read);
    assert_eq!(wire.lfs_pointers, 1, "the count is on the wire, to every scope");

    // A repository with its objects in the store leaves nothing to say.
    let content = small(2);
    let Some((_dir, svc, repo_id, _repo)) = lfs_repo("a_worktree_made_with_its_object", &content, true).await else {
        return;
    };
    let ws = make(&svc, repo_id).await;
    assert_eq!(std::fs::read(Path::new(&ws.worktree_path).join("big.bin")).unwrap(), content);
    assert_eq!(svc.worktree_view(&ws).await.unwrap().lfs_pointers, 0);
}

#[tokio::test]
async fn trying_again_downloads_what_arrived_and_keeps_the_agents_staged_work() {
    let content = small(3);
    let Some((_dir, svc, repo_id, repo)) = lfs_repo("trying_again_downloads", &content, false).await else { return };
    let ws = make(&svc, repo_id).await;
    let wt = PathBuf::from(&ws.worktree_path);
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 1);

    // The agent has an unrelated change staged, and an editor's unstaged one.
    std::fs::write(wt.join("notes.txt"), "mine\n").unwrap();
    plain(&wt, &["add", "notes.txt"]);

    // Nothing arrived: the retry says the same, and writes nothing.
    let same = svc.hydrate_lfs(ws.id).await.unwrap();
    assert_eq!(svc.store.lfs_pointer_count(same.id).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(wt.join("big.bin")).unwrap(), pointer_text(&content));

    store_object(&repo, &content);
    let after = svc.hydrate_lfs(ws.id).await.unwrap();
    assert!(after.resource_version > ws.resource_version, "the version moved, so clients refetch");
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 0, "no pointers left");
    assert!(std::fs::read(wt.join("big.bin")).unwrap() == content, "the file is its content");
    assert_eq!(plain(&wt, &["diff", "--cached", "--name-only"]).trim(), "notes.txt", "the staged change stands");
    let leftovers: Vec<_> = std::fs::read_dir(git_dir(&wt))
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().into_string().ok())
        .filter(|n| n.contains("fc-lfs") || n == "index.lock")
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    // Through the daemon's own git, which reads the hydrated file by its pointer.
    let tree = crate::change_set::working_tree(&wt).await.unwrap();
    let paths = |files: Vec<crate::change_set::FileChange>| files.into_iter().map(|f| f.path).collect::<Vec<_>>();
    assert_eq!(paths(tree.staged), ["notes.txt"], "only the agent's own change is staged");
    assert!(tree.unstaged.is_empty(), "and the hydrated file reads as unchanged: {:?}", tree.unstaged);
}

#[tokio::test]
async fn a_path_the_agent_changed_is_not_rewritten() {
    let content = small(4);
    let Some((_dir, svc, repo_id, repo)) = lfs_repo("a_path_the_agent_changed", &content, false).await else { return };
    let ws = make(&svc, repo_id).await;
    let wt = PathBuf::from(&ws.worktree_path);
    std::fs::write(wt.join("big.bin"), "the agent's own\n").unwrap();
    store_object(&repo, &content);
    svc.hydrate_lfs(ws.id).await.unwrap();
    assert_eq!(std::fs::read_to_string(wt.join("big.bin")).unwrap(), "the agent's own\n");
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 0, "no longer a pointer, so nothing to download");
}

#[tokio::test]
async fn a_retry_that_runs_out_puts_the_pointer_back_and_leaves_the_rest() {
    // Large enough that a debug build can't stream it in the limit below.
    let big: Vec<u8> = (0..16u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
    let Some((_dir, svc, repo_id, repo)) = lfs_repo("a_retry_that_runs_out", &big, false).await else { return };
    let ws = make(&svc, repo_id).await;
    let wt = PathBuf::from(&ws.worktree_path);
    std::fs::write(wt.join("notes.txt"), "mine\n").unwrap();
    plain(&wt, &["add", "notes.txt"]);
    store_object(&repo, &big);

    LIMIT.with(|l| l.set(Some(std::time::Duration::from_millis(150))));
    let after = svc.hydrate_lfs(ws.id).await;
    LIMIT.with(|l| l.set(None));
    after.unwrap();
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 1, "still a pointer");
    assert_eq!(std::fs::read_to_string(wt.join("big.bin")).unwrap(), pointer_text(&big), "back to its pointer");
    assert!(!git_dir(&wt).join("index.lock").exists());
    assert_eq!(plain(&wt, &["diff", "--cached", "--name-only"]).trim(), "notes.txt");

    // Given time it works, so the first one really was cut short.
    svc.hydrate_lfs(ws.id).await.unwrap();
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 0);
    assert!(std::fs::read(wt.join("big.bin")).unwrap() == big);
}

#[tokio::test]
async fn reading_the_changes_notices_files_an_agent_downloaded_itself() {
    let content = small(5);
    let Some((_dir, svc, repo_id, _repo)) = lfs_repo("reading_the_changes_notices", &content, false).await else {
        return;
    };
    let ws = make(&svc, repo_id).await;
    assert!(!svc.recheck_lfs_pointers(ws.id).await, "nothing moved");
    // `git lfs pull`, by hand.
    std::fs::write(Path::new(&ws.worktree_path).join("big.bin"), &content).unwrap();
    assert!(svc.recheck_lfs_pointers(ws.id).await, "the count moved");
    assert_eq!(svc.store.lfs_pointer_count(ws.id).unwrap(), 0);
    assert!(!svc.recheck_lfs_pointers(ws.id).await, "and says so once");
}

/// Try Again never writes the worktree's real index (review 1004j X2): an
/// `update-index` there could put HEAD's entry back over one the agent
/// staged a moment before, or fail the agent's `git commit` on `index.lock`.
/// The index file is the same bytes after the retry as before it, and the
/// filled file still reads as unchanged.
#[tokio::test]
async fn trying_again_leaves_the_real_index_byte_for_byte() {
    let content = small(6);
    let Some((_dir, svc, repo_id, repo)) = lfs_repo("trying_again_leaves_the_real_index", &content, false).await
    else {
        return;
    };
    let ws = make(&svc, repo_id).await;
    let wt = PathBuf::from(&ws.worktree_path);
    std::fs::write(wt.join("notes.txt"), "mine\n").unwrap();
    plain(&wt, &["add", "notes.txt"]);
    store_object(&repo, &content);

    let index = git_dir(&wt).join("index");
    let before = std::fs::read(&index).unwrap();
    svc.hydrate_lfs(ws.id).await.unwrap();
    assert!(std::fs::read(wt.join("big.bin")).unwrap() == content, "the file is its content");
    assert!(std::fs::read(&index).unwrap() == before, "the real index was written");

    let tree = crate::change_set::working_tree(&wt).await.unwrap();
    assert!(tree.unstaged.is_empty(), "the filled file reads as unchanged: {:?}", tree.unstaged);
}

/// An agent that stages an edit of a large file while Try Again runs keeps
/// it: the entry it staged, and the file it wrote, are what's there after.
#[tokio::test]
async fn an_edit_the_agent_stages_mid_retry_survives() {
    let content = small(7);
    let Some((_dir, svc, repo_id, repo)) = lfs_repo("an_edit_the_agent_stages_mid_retry", &content, false).await
    else {
        return;
    };
    let ws = make(&svc, repo_id).await;
    let wt = PathBuf::from(&ws.worktree_path);
    store_object(&repo, &content);

    fn agent(wt: &Path) {
        std::fs::write(wt.join("big.bin"), "the agent's own\n").unwrap();
        plain(wt, &["add", "big.bin"]);
    }
    WRITTEN.with(|w| w.set(Some(agent)));
    let after = svc.hydrate_lfs(ws.id).await;
    WRITTEN.with(|w| w.set(None));
    after.unwrap();

    let staged = plain(&wt, &["ls-files", "-s", "big.bin"]);
    let theirs = plain(&wt, &["hash-object", "big.bin"]);
    assert!(staged.contains(theirs.trim()), "the agent's staged entry stands: {staged}");
    assert_eq!(std::fs::read_to_string(wt.join("big.bin")).unwrap(), "the agent's own\n");
    assert_eq!(plain(&wt, &["diff", "--cached", "--name-only"]).trim(), "big.bin");
}

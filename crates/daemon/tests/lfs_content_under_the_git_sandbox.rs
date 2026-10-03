//! A worktree the daemon makes of a Git LFS repository holds the files'
//! content, with every daemon git inside the exec allowlist; and nothing an
//! LFS repository's own config names (a filter, an `lfs.extension`, a custom
//! transfer agent) runs.
//!
//! The daemon's LFS filter is `farcooler-lfs-filter`, a binary of this
//! package, which cargo builds for these tests; `git_lfs::locate` finds it
//! above the test binary. git-lfs itself is never run by the daemon, so most
//! of this needs no git-lfs: the repository is written by hand, pointer and
//! object store both, the way git-lfs lays them out. The two tests that use
//! the real git-lfs (one makes the repository with it; the other shows that
//! the planted extension would run under it) skip when it isn't installed,
//! and say so. CI installs it, and there a missing one fails them.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use farcooler_daemon::change_set::working_tree;
use farcooler_daemon::git::{create_worktree, git_launch};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

/// git as the fixture runs it: no inherited `GIT_` variables, no global or
/// system config (a user's own `filter.lfs` would run git-lfs), no hooks, and
/// LFS off unless `extra` turns it on.
fn run_with(dir: &Path, extra: &[&str], args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.current_dir(dir)
        .args(["-c", "core.hooksPath=/dev/null", "-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "filter.lfs.process=", "-c", "filter.lfs.clean=", "-c", "filter.lfs.smudge="])
        .args(["-c", "filter.lfs.required=false"])
        .args(extra)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn run(dir: &Path, args: &[&str]) {
    let out = run_with(dir, &[], args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pointer(oid: &str, size: usize) -> String {
    format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {size}\n")
}

/// Content big enough to take several pkt-lines, and not all one byte.
fn content(seed: u8) -> Vec<u8> {
    (0..200_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed).collect()
}

/// git-lfs, if it's installed, by absolute path.
fn git_lfs() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("git-lfs")).find(|p| p.is_file())
}

/// git-lfs, or `None` and a line saying the test skipped. Under CI, where
/// the workflow installs it, a missing git-lfs fails instead.
fn skip_without_git_lfs(test: &str) -> Option<PathBuf> {
    let lfs = git_lfs();
    if lfs.is_none() {
        assert!(std::env::var_os("CI").is_none(), "{test}: CI installs git-lfs, and it isn't on PATH");
        eprintln!("SKIPPED {test}: git-lfs isn't installed here (CI installs it)");
    }
    lfs
}

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let f = Fixture { dir: tempfile::tempdir().unwrap() };
        std::fs::create_dir_all(f.repo()).unwrap();
        run(&f.repo(), &["init", "-q", "-b", "main"]);
        run(&f.repo(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(f.repo().join(".gitattributes"), "*.bin filter=lfs diff=lfs merge=lfs -text\n").unwrap();
        let probe = f.root().join("probe");
        std::fs::write(&probe, "#!/bin/sh\ntouch \"$(dirname \"$0\")/ran-$1\"\ncat\n").unwrap();
        std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
        f
    }

    fn root(&self) -> PathBuf {
        self.dir.path().canonicalize().unwrap()
    }

    fn repo(&self) -> PathBuf {
        self.root().join("repo")
    }

    fn worktree(&self) -> PathBuf {
        self.root().join("wt")
    }

    fn probe(&self, role: &str) -> String {
        format!("{} {role}", self.root().join("probe").display())
    }

    /// Every probe that ran, and the record cleared.
    fn ran(&self) -> Vec<String> {
        let mut ran: Vec<String> = std::fs::read_dir(self.root())
            .unwrap()
            .filter_map(|e| e.unwrap().file_name().into_string().ok())
            .filter_map(|n| n.strip_prefix("ran-").map(str::to_string))
            .collect();
        for role in &ran {
            std::fs::remove_file(self.root().join(format!("ran-{role}"))).unwrap();
        }
        ran.sort();
        ran
    }

    /// `name` committed as the pointer to `bytes`, with `stored` (usually
    /// `bytes`) in the store under that pointer's oid, as git-lfs leaves it.
    fn lfs_file(&self, name: &str, bytes: &[u8], stored: Option<&[u8]>) -> String {
        let oid = hex(&Sha256::digest(bytes));
        let text = pointer(&oid, bytes.len());
        std::fs::write(self.repo().join(name), &text).unwrap();
        if let Some(stored) = stored {
            let at = self.repo().join(".git/lfs/objects").join(&oid[0..2]).join(&oid[2..4]);
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join(&oid), stored).unwrap();
        }
        text
    }

    fn commit(&self) {
        run(&self.repo(), &["add", "-A"]);
        run(&self.repo(), &["commit", "-q", "-m", "base"]);
    }

    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.worktree().join(name)).unwrap()
    }

    /// Rewrite `name` with what it holds: new mtime, so status can't trust
    /// the index's stat data and reads it through the clean filter.
    fn restat(&self, name: &str) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let path = self.worktree().join(name);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, bytes).unwrap();
    }

    async fn status(&self) -> Vec<String> {
        let tree = working_tree(&self.worktree()).await.unwrap();
        let mut paths: Vec<String> =
            [tree.staged, tree.unstaged, tree.untracked].into_iter().flatten().map(|f| f.path).collect();
        paths.sort();
        paths
    }
}

fn sandbox_and_helper_are_on() {
    assert!(
        git_launch().unwrap().sandbox.is_some(),
        "this host has no exec sandbox, so this can't show anything; on Linux that's a kernel without Landlock"
    );
    assert!(
        farcooler_daemon::git_lfs::helper().is_some(),
        "no farcooler-lfs-filter beside the test binary: cargo builds it for integration tests"
    );
}

#[tokio::test]
async fn a_new_worktree_gets_lfs_content_with_the_sandbox_on() {
    sandbox_and_helper_are_on();
    let f = Fixture::new();
    let big = content(1);
    f.lfs_file("big.bin", &big, Some(&big));
    let missing = f.lfs_file("missing.bin", &content(2), None);
    // An object whose bytes aren't what its name promises, at the right size:
    // what an agent's store would hold to read a file it can't.
    let liar = f.lfs_file("liar.bin", &content(3), Some(&content(4)));
    f.commit();

    create_worktree(&f.repo(), "feature", "HEAD", &f.worktree()).await.unwrap();
    assert!(f.read("big.bin") == big, "big.bin is its content, not its pointer");
    assert_eq!(f.read("missing.bin"), missing.as_bytes(), "an object that isn't here stays a pointer");
    assert_eq!(f.read("liar.bin"), liar.as_bytes(), "an object that fails its hash stays a pointer");

    // Review sees a clean worktree, also once git has to read the hydrated
    // file again: the filter turns it back into the pointer the index holds.
    assert_eq!(f.status().await, Vec::<String>::new());
    f.restat("big.bin");
    assert_eq!(f.status().await, Vec::<String>::new(), "a hydrated file compares by its pointer");

    let mut changed = big.clone();
    changed[1000] ^= 0xff;
    std::fs::write(f.worktree().join("big.bin"), &changed).unwrap();
    assert_eq!(f.status().await, ["big.bin"], "and a changed one shows as changed");
}

/// Everything an LFS repository's config can name, planted, and a pointer
/// that names an extension: the daemon's worktree and status run none of it.
#[tokio::test]
async fn a_planted_lfs_extension_or_filter_runs_nothing() {
    sandbox_and_helper_are_on();
    let f = Fixture::new();
    let big = content(5);
    f.lfs_file("big.bin", &big, Some(&big));
    let ext_oid = hex(&Sha256::digest(&big));
    let ext = format!(
        "version https://git-lfs.github.com/spec/v1\next-0-x sha256:{ext_oid}\noid sha256:{ext_oid}\nsize {}\n",
        big.len()
    );
    std::fs::write(f.repo().join("ext.bin"), &ext).unwrap();
    f.commit();
    for (key, value) in [
        ("filter.lfs.process", f.probe("process")),
        ("filter.lfs.smudge", f.probe("smudge")),
        ("filter.lfs.clean", f.probe("clean")),
        ("filter.lfs.required", "true".to_string()),
        ("lfs.extension.x.clean", f.probe("ext-clean")),
        ("lfs.extension.x.smudge", f.probe("ext-smudge")),
        ("lfs.extension.x.priority", "0".to_string()),
        ("lfs.customtransfer.x.path", f.probe("transfer")),
        ("lfs.standalonetransferagent", "x".to_string()),
    ] {
        run(&f.repo(), &["config", "--add", key, &value]);
    }

    create_worktree(&f.repo(), "feature", "HEAD", &f.worktree()).await.unwrap();
    assert!(f.read("big.bin") == big, "the daemon's own filter, not the planted one");
    assert_eq!(f.read("ext.bin"), ext.as_bytes(), "a pointer naming an extension is never hydrated");
    f.restat("big.bin");
    assert_eq!(f.status().await, Vec::<String>::new());
    assert_eq!(f.ran(), Vec::<String>::new(), "a planted program ran");

    // The control for the filter keys: plain git, with nothing but the
    // repository's own config, runs them.
    f.restat("big.bin");
    let mut plain = Command::new("git");
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("GIT_") {
            plain.env_remove(k);
        }
    }
    let _ = plain.env("GIT_CONFIG_GLOBAL", "/dev/null").current_dir(f.worktree()).args(["status"]).output().unwrap();
    assert_eq!(f.ran(), ["process"], "control: plain git runs the planted filter");
}

/// The control for `lfs.extension`: under git-lfs, the planted extension
/// runs on the very status the daemon makes. Without it, the test above
/// proves nothing about extensions.
#[tokio::test]
async fn the_planted_extension_does_run_under_git_lfs() {
    let Some(lfs) = skip_without_git_lfs("the_planted_extension_does_run_under_git_lfs") else { return };
    let f = Fixture::new();
    let big = content(6);
    f.lfs_file("big.bin", &big, Some(&big));
    f.commit();
    run(&f.repo(), &["config", "lfs.extension.x.clean", &format!("{} %f", f.probe("ext-clean"))]);
    run(&f.repo(), &["config", "lfs.extension.x.smudge", &format!("{} %f", f.probe("ext-smudge"))]);
    run(&f.repo(), &["config", "lfs.extension.x.priority", "0"]);
    let process = format!("filter.lfs.process={} filter-process", lfs.display());
    let with_lfs = ["-c", process.as_str(), "-c", "filter.lfs.required=true"];
    // Hydrated by git-lfs, as a user's own checkout would be, then read
    // again by status, as the daemon's status reads it.
    std::fs::remove_file(f.repo().join("big.bin")).unwrap();
    let out = run_with(&f.repo(), &with_lfs, &["checkout", "--", "big.bin"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::read(f.repo().join("big.bin")).unwrap() == big, "git-lfs hydrated it");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(f.repo().join("big.bin"), &big).unwrap();
    let out = run_with(&f.repo(), &with_lfs, &["status", "--porcelain"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(f.ran(), ["ext-clean"], "git-lfs ran the extension the repository's config named");
}

/// A repository git-lfs itself made: its pointers and its store are what the
/// daemon's filter reads.
#[tokio::test]
async fn a_repository_git_lfs_made_gets_its_content() {
    let Some(lfs) = skip_without_git_lfs("a_repository_git_lfs_made_gets_its_content") else { return };
    sandbox_and_helper_are_on();
    let f = Fixture::new();
    let big = content(7);
    std::fs::write(f.repo().join("big.bin"), &big).unwrap();
    let with_lfs = [
        "-c",
        &format!("filter.lfs.process={} filter-process", lfs.display()),
        "-c",
        "filter.lfs.required=true",
    ];
    for args in [&["add", "-A"][..], &["commit", "-q", "-m", "base"]] {
        let out = run_with(&f.repo(), &with_lfs, args);
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let committed = run_with(&f.repo(), &[], &["show", "HEAD:big.bin"]).stdout;
    assert!(committed.starts_with(b"version https://git-lfs.github.com/spec/v1\n"), "git-lfs committed a pointer");

    create_worktree(&f.repo(), "feature", "HEAD", &f.worktree()).await.unwrap();
    assert!(f.read("big.bin") == big, "the content git-lfs stored");
    f.restat("big.bin");
    assert_eq!(f.status().await, Vec::<String>::new());
}

/// A commit whose tree is `.gitattributes`, the LFS pointer `file`, and a
/// symlink `link` to `outside`, made with plumbing because no working tree
/// on a case- or normalization-insensitive volume can hold both. `file`'s
/// first directory and `link` are the same name to such a volume.
fn aliased_commit(repo: &Path, file: &str, link: &str, outside: &Path) {
    std::fs::create_dir_all(repo).unwrap();
    run(repo, &["init", "-q", "-b", "main"]);
    let blobs = repo.parent().unwrap().join("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    let blob = |name: &str, bytes: &[u8]| {
        let path = blobs.join(name);
        std::fs::write(&path, bytes).unwrap();
        let out = run_with(repo, &[], &["hash-object", "-w", "--no-filters", path.to_str().unwrap()]);
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let attributes = blob("attributes", b"*.bin filter=lfs -text\n");
    let pointer = blob("pointer", pointer(&"0".repeat(64), 3).as_bytes());
    let target = blob("link", outside.as_os_str().as_encoded_bytes());
    for (mode, sha, path) in [("100644", &attributes, ".gitattributes"), ("100644", &pointer, file), ("120000", &target, link)]
    {
        // As written: a Mac's git would otherwise fold NFD to NFC.
        let out = run_with(
            repo,
            &["-c", "core.precomposeUnicode=false"],
            &["update-index", "--add", "--cacheinfo", &format!("{mode},{sha},{path}")],
        );
        assert!(out.status.success(), "update-index {path:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let tree = String::from_utf8(run_with(repo, &[], &["write-tree"]).stdout).unwrap().trim().to_string();
    let commit = String::from_utf8(run_with(repo, &[], &["commit-tree", &tree, "-m", "base"]).stdout).unwrap();
    run(repo, &["update-ref", "refs/heads/main", commit.trim()]);
}

/// The ov-187 re-review's reproduction, and its variants: a symlink that a
/// case-insensitive (APFS) or normalization-insensitive volume reads as the
/// LFS file's parent directory. Hydrating must not reach through it: the
/// file of the same name outside the worktree survives. On a volume that
/// tells the names apart there's no alias, and it holds trivially.
#[tokio::test]
async fn hydration_never_reaches_through_a_symlink_aliasing_a_directory() {
    sandbox_and_helper_are_on();
    let cases = [
        ("A/x.bin", "a"),
        ("Dir/inner/x.bin", "dir"),
        ("\u{e9}/x.bin", "e\u{301}"),
        ("e\u{301}/x.bin", "\u{e9}"),
    ];
    let mut touched = Vec::new();
    for (i, (file, link)) in cases.into_iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = root.join("outside");
        let victim = outside.join(file.split_once('/').unwrap().1);
        std::fs::create_dir_all(victim.parent().unwrap()).unwrap();
        std::fs::write(&victim, "keep me\n").unwrap();
        let repo = root.join("repo");
        aliased_commit(&repo, file, link, &outside);

        let _ = create_worktree(&repo, "feature", "main", &root.join("wt")).await;
        if std::fs::read(&victim).ok().as_deref() != Some(&b"keep me\n"[..]) {
            touched.push(format!("case {i}: {file:?} beside a link {link:?}"));
        }
    }
    assert_eq!(touched, Vec::<String>::new(), "a file outside the worktree was touched");
}

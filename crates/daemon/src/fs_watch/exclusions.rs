//! Which parts of a worktree its watch should never be told about.
//!
//! Only consulted where one registration covers the whole tree (macOS). There,
//! everything under the worktree root reaches the stream unless it is named
//! here, and the routes drop what git ignores only after fseventsd has paid to
//! deliver it. Eight names per stream is FSEvents' limit, so this ranks.
//!
//! **What may be excluded, and why it is safe.** A repository's own files
//! decide what lands here — its `.gitignore`, its directory names, a nested
//! repository someone cloned into it — so the rule is about git's view, not
//! about names: a path is excluded only if git itself says nothing under it is
//! part of this worktree's change set. That is a directory git ignores, a
//! nested repository or worktree (git shows it as one `?? dir/` line and never
//! looks inside), or the worktree's own `.git`, which has a watch of its own.
//! Never a directory that holds a tracked file, whatever its name: the guard
//! is the tracked-directory set the walk already made, not a list of names.
//! FSEvents still delivers the excluded path ITSELF — its creation and its
//! removal — so `?? nested/` appearing or going is still seen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The most paths one FSEvents stream may exclude.
///
/// Apple's `FSEventStreamSetExclusionPaths` documents eight; past that the
/// call fails and the stream excludes nothing.
pub(super) const MAX_EXCLUSIONS: usize = 8;

/// Build output names, most expensive first, excluded ahead of other ignored
/// directories and probed at the root even before they exist.
///
/// Probed because the first `cargo build` in a fresh worktree creates
/// `target/` AFTER the worktree was registered, and a stream's exclusions are
/// fixed when it starts. Asking git whether `target/x` would be ignored works
/// for a path that is not there, and FSEvents honors an exclusion for a path
/// that does not exist yet as long as it is spelled canonically.
const BUILD_OUTPUT: &[&str] = &[
    "target",
    "node_modules",
    ".build",
    "DerivedData",
    "build",
    ".gradle",
    "dist",
    ".next",
];

/// Ignored-directory records past this are not ranked: a repository with a
/// hundred thousand `__pycache__` directories still gets its eight.
const MAX_CANDIDATES: usize = 20_000;

/// The absolute paths this worktree's stream should leave out, best first.
///
/// `dirs` is the tracked-directory set and `nested` the nested repositories,
/// both from `tracked_directories`; an ignored directory that holds checkouts
/// joins `nested`.
///
/// `None` when git could not answer — an error, or a timeout during the heavy
/// build that is exactly when re-walks happen. The caller then keeps what it
/// had: an empty list would restart the stream with nothing left out and let
/// the flood back in until some later re-walk.
pub(super) async fn for_worktree(
    worktree: &Path,
    git_dir: &Path,
    dirs: &HashSet<PathBuf>,
    nested: &[PathBuf],
) -> Option<Vec<PathBuf>> {
    let (ignored, probed) = tokio::join!(ignored_directories(worktree), ignored_at_root(worktree));
    let (ignored, probed) = (ignored?, probed?);
    let base = super::canonical_ish(worktree).unwrap_or_else(|| worktree.to_path_buf());
    let own_git = super::canonical_ish(git_dir)
        .and_then(|git| git.strip_prefix(&base).ok().map(Path::to_path_buf));
    let mut shallow: Vec<&PathBuf> = ignored.iter().collect();
    shallow.sort_by_key(|p| p.components().count());
    let mut nests: Vec<PathBuf> = nested.to_vec();
    nests.extend(
        shallow.into_iter().take(NEST_PROBES).filter(|rel| holds_checkouts(&worktree.join(rel))).cloned(),
    );
    Some(rank(dirs, &nests, &ignored, &probed, own_git.as_deref()).into_iter().map(|rel| base.join(rel)).collect())
}

/// Order the candidates and keep the first `MAX_EXCLUSIONS`, all relative.
///
/// Nested repositories first, shallowest first: each is a whole tree with its
/// own build output, and the main checkout's `.claude/worktrees` is the case
/// that doubled every lane's build. Then build output by `BUILD_OUTPUT` order,
/// then the worktree's own `.git`, then any other ignored directory,
/// shallowest first.
fn rank(
    dirs: &HashSet<PathBuf>,
    nested: &[PathBuf],
    ignored: &[PathBuf],
    probed: &[PathBuf],
    own_git: Option<&Path>,
) -> Vec<PathBuf> {
    let depth = |p: &PathBuf| p.components().count();
    let build_rank = |p: &PathBuf| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| BUILD_OUTPUT.iter().position(|b| *b == n))
    };

    let mut nested: Vec<PathBuf> = nested.to_vec();
    nested.sort_by_key(|p| (depth(p), p.clone()));

    let mut build: Vec<PathBuf> =
        ignored.iter().chain(probed).filter(|p| build_rank(p).is_some()).cloned().collect();
    build.sort_by_key(|p| (build_rank(p), depth(p), p.clone()));

    let mut other: Vec<PathBuf> = ignored.iter().filter(|p| build_rank(p).is_none()).cloned().collect();
    other.sort_by_key(|p| (depth(p), p.clone()));

    let mut out: Vec<PathBuf> = Vec::new();
    let candidates = nested.into_iter().chain(build).chain(own_git.map(Path::to_path_buf)).chain(other);
    for rel in candidates {
        if out.len() == MAX_EXCLUSIONS {
            break;
        }
        // The guard that makes a hostile `.gitignore` harmless: nothing that
        // holds a tracked or untracked-but-wanted file, at any depth, since
        // `dirs` names every ancestor of every such file. The root is in
        // `dirs` as the empty path, so it can never be excluded either.
        if dirs.contains(&rel) || rel.as_os_str().is_empty() {
            continue;
        }
        // FSEvents takes paths as UTF-8 strings; one it cannot spell is
        // dropped here, alone, rather than costing the whole list its slot.
        if rel.to_str().is_none() {
            continue;
        }
        // Already covered by an ancestor that made the list.
        if out.iter().any(|kept| rel.starts_with(kept)) {
            continue;
        }
        out.retain(|kept| !kept.starts_with(&rel));
        out.push(rel);
    }
    out
}

/// The topmost ignored directories, from git, relative to the worktree.
///
/// `None` when git failed or timed out.
async fn ignored_directories(worktree: &Path) -> Option<Vec<PathBuf>> {
    let listed = crate::git::git_bytes(
        worktree,
        &["ls-files", "-z", "--others", "--ignored", "--exclude-standard", "--directory"],
    )
    .await;
    let out = listed.ok().filter(|out| out.ok)?;
    let found = out
        .stdout
        .split(|b| *b == 0)
        .filter_map(|record| record.strip_suffix(b"/"))
        .filter(|record| !record.is_empty())
        .take(MAX_CANDIDATES)
        .filter_map(super::os_path)
        .collect();
    Some(found)
}

/// Two names nobody's `.gitignore` names on purpose, probed under each
/// `BUILD_OUTPUT` directory. A name counts as ignored only if BOTH are: a
/// pattern like `x`, `**/x` or `*.tmp` that happens to match one probe is not
/// a statement about the directory, and matches the other by accident only if
/// it matches nearly everything — `*` — in which case the directory is indeed
/// ignored.
const PROBES: [&str; 2] = ["farcooler-probe-7f3c", "FcProbe.c9e1/x"];

/// The `BUILD_OUTPUT` names git would ignore at the root, present or not.
///
/// `None` when git failed or timed out.
async fn ignored_at_root(worktree: &Path) -> Option<Vec<PathBuf>> {
    let probes: Vec<String> = BUILD_OUTPUT
        .iter()
        .flat_map(|name| PROBES.iter().map(move |probe| format!("{name}/{probe}")))
        .collect();
    // Not `-z`, which git accepts only with `--stdin`: these are our own
    // ASCII names, so a newline-separated answer is unambiguous.
    let mut args = vec!["check-ignore", "--no-index", "--"];
    args.extend(probes.iter().map(String::as_str));
    let out = crate::git::git_bytes(worktree, &args).await.ok()?;
    // `check-ignore` exits 1 when nothing matched, which is an answer; an
    // exit with something on stderr (128) is not.
    if !out.ok && !out.stderr.trim().is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let said: HashSet<&str> = text.lines().collect();
    let found = BUILD_OUTPUT
        .iter()
        .filter(|name| PROBES.iter().all(|probe| said.contains(format!("{name}/{probe}").as_str())))
        .map(PathBuf::from)
        .collect();
    Some(found)
}

/// Whether an ignored directory is, or directly holds, a git checkout.
///
/// The main checkout's `.claude/worktrees` is ignored, so git lists it as one
/// ignored directory and never as the worktrees inside it — but each of those
/// is a whole tree with its own `target/`, which is exactly what most needs
/// leaving out. One `stat` for the directory and one per child, the children
/// capped, for at most the shallowest `NEST_PROBES` candidates.
fn holds_checkouts(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().take(256).any(|entry| entry.path().join(".git").exists())
}

/// How many ignored directories are checked with `holds_checkouts`.
const NEST_PROBES: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    async fn scratch(ignore: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();
        crate::git::git(&root, &["init", "-q", "."]).await.unwrap();
        crate::git::git(&root, &["config", "core.excludesFile", "/dev/null"]).await.unwrap();
        std::fs::write(root.join(".gitignore"), ignore).unwrap();
        (dir, root)
    }

    #[tokio::test]
    async fn a_pattern_that_only_matches_a_probe_name_is_not_build_output() {
        // `x` matched the old `target/x` probe and so named all eight.
        for pattern in ["x\n", "**/x\n", "farcooler-probe-7f3c\n"] {
            let (_dir, root) = scratch(pattern).await;
            let got = ignored_at_root(&root).await.unwrap();
            assert!(got.is_empty(), "{pattern:?} named {got:?}");
        }
        let (_dir, root) = scratch("/target\nnode_modules/\n").await;
        assert_eq!(ignored_at_root(&root).await.unwrap(), paths(&["target", "node_modules"]));
    }

    #[tokio::test]
    async fn git_failing_is_no_answer_rather_than_no_exclusions() {
        // Not a repository: both gits fail, which must not read as "nothing
        // is ignored" and restart a stream with nothing left out.
        let dir = tempfile::tempdir().unwrap();
        let none = for_worktree(dir.path(), &dir.path().join(".git"), &HashSet::new(), &[]).await;
        assert_eq!(none, None);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_fsevents_cannot_spell_is_dropped_alone() {
        use std::os::unix::ffi::OsStrExt;
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"caf\xe9"));
        let dirs = set(&[""]);
        let got = rank(&dirs, &[], &[odd, PathBuf::from("target")], &[], None);
        assert_eq!(got, paths(&["target"]));
    }

    fn set(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_directory_holding_a_tracked_file_is_never_excluded() {
        // A `.gitignore` that names `src` cannot hide `src` if git tracks a
        // file under it: `dirs` has `src`, so `src` is refused by name.
        let dirs = set(&["", "src", "build"]);
        let got = rank(&dirs, &[], &paths(&["src", "build", "target"]), &[], None);
        assert_eq!(got, paths(&["target"]));
    }

    #[test]
    fn nested_repositories_come_first_then_build_output_then_git() {
        let dirs = set(&[""]);
        let got = rank(
            &dirs,
            &paths(&[".claude/worktrees/b", "vendor/lib"]),
            &paths(&["__pycache__", "node_modules", "target"]),
            &paths(&["target", ".build"]),
            Some(Path::new(".git")),
        );
        assert_eq!(
            got,
            paths(&[
                "vendor/lib",
                ".claude/worktrees/b",
                "target",
                "node_modules",
                ".build",
                ".git",
                "__pycache__"
            ])
        );
    }

    #[test]
    fn never_more_than_fsevents_takes_and_never_a_path_twice() {
        let dirs = set(&[""]);
        let many: Vec<String> = (0..20).map(|i| format!("ignored{i:02}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut ignored = paths(&many);
        ignored.push(PathBuf::from("target"));
        let got = rank(&dirs, &[], &ignored, &paths(&["target"]), None);
        assert_eq!(got.len(), MAX_EXCLUSIONS);
        assert_eq!(got[0], PathBuf::from("target"));
        assert_eq!(got.iter().filter(|p| *p == Path::new("target")).count(), 1);
    }

    #[test]
    fn an_ancestor_covers_its_descendants() {
        let dirs = set(&[""]);
        let got = rank(
            &dirs,
            &paths(&[".claude/worktrees/a/target"]),
            &paths(&[".claude/worktrees"]),
            &[],
            None,
        );
        assert_eq!(got, paths(&[".claude/worktrees"]));
    }
}

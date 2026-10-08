//! Git LFS in the daemon's own gits: `farcooler-lfs-filter`, never git-lfs.
//!
//! The daemon's gits run inside an exec allowlist with no shell
//! (`crate::git_sandbox`), so the user's own `filter.lfs.process = git-lfs
//! filter-process` can't start: git runs a filter with arguments through
//! `sh -c`. git-lfs can't simply be allowed either, even by exact path. It
//! reads `lfs.extension.<name>.smudge`/`.clean` and
//! `lfs.customtransfer.<name>.path` from the repository's own config, which
//! the agent writes, and execs them with arguments, no shell needed. With git
//! on the list, an extension of `git config --global core.hooksPath <dir>`
//! is the user's global config rewritten; on Linux, where the dynamic loader
//! is on the list, `ld-linux <file>` runs any program. The names are the
//! config's to choose, so they can't be pinned off either, short of racing
//! the agent's writes. Running git-lfs outside the sandbox instead (after
//! `worktree add`) is the same: it would read that config as the user.
//!
//! So `filter.lfs.process` is pinned ([`pins`], in `crate::git_guard`'s
//! fixed set, which outranks every config file) to `farcooler-lfs-filter`, a
//! program shipped beside the daemon. By its bare name, with no arguments:
//! git execs a command with no shell metacharacter in it directly, and finds
//! a bare name on the `PATH` the daemon hands it, which starts with the
//! helper's directory ([`path`]). The name rather than the path because the
//! path may hold a space (`Far Cooler.app`), and a space means `sh -c`.
//!
//! The helper hydrates a pointer from the repository's local LFS store and
//! turns content back into its pointer for status and diff; it execs nothing
//! and reads no config. What it can't do: fetch. An object that isn't in the
//! local store stays a pointer, as it did before (the agent runs `git lfs
//! pull`). A worktree made from a checkout that has its LFS content gets that
//! content.
//!
//! **Hydration is its own step** ([`hydrate`]). A new worktree is filled with
//! LFS off, so it holds pointers and the fill stays inside `GIT_TIMEOUT`
//! (measured: hydrating one 2 GB object took about 25 s). Then a `checkout`
//! of the `filter=lfs` paths, with the helper, gets [`HYDRATE_LIMIT`]; one
//! that fails or runs out is undone back to pointers and logged, and the
//! worktree stands either way.
//!
//! With no helper beside the daemon (an install that didn't ship it), LFS is
//! off as before: the filter is emptied, and the daemon says so once.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::git_guard::Pin;

/// The helper's file name, beside `farcoolerd`.
pub const NAME: &str = "farcooler-lfs-filter";

/// Where the helper is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    /// Its directory, which goes first on git's `PATH`.
    pub dir: PathBuf,
    /// The helper itself, for the exec allowlist.
    pub program: PathBuf,
}

/// The helper beside this daemon, looked for once.
pub fn helper() -> Option<&'static Helper> {
    static HELPER: OnceLock<Option<Helper>> = OnceLock::new();
    HELPER
        .get_or_init(|| {
            let found = std::env::current_exe().ok().and_then(|exe| locate(&exe));
            if found.is_none() {
                tracing::warn!(
                    "no {NAME} beside the daemon, so its gits leave Git LFS files as pointers"
                );
            }
            found
        })
        .as_ref()
}

/// `farcooler-lfs-filter` beside `exe`, by `exe`'s real path: in its
/// directory, or for a `cargo test` binary (in `target/<profile>/deps`) the
/// one above. `None` when there's no executable there, or its directory can't
/// be a `PATH` entry (it holds a `:`).
pub fn locate(exe: &Path) -> Option<Helper> {
    let exe = exe.canonicalize().ok()?;
    let dir = exe.parent()?;
    let mut dirs = vec![dir.to_path_buf()];
    if dir.file_name().is_some_and(|n| n == "deps") {
        dirs.extend(dir.parent().map(Path::to_path_buf));
    }
    dirs.into_iter().find_map(|dir| {
        let program = dir.join(NAME);
        let usable = crate::git_sandbox::is_executable(&program)
            && !crate::git_sandbox::is_script(&program)
            && !dir.as_os_str().as_encoded_bytes().contains(&b':');
        usable.then_some(Helper { dir, program })
    })
}

/// The `filter.lfs` pins: the helper as the process filter when there is
/// one, and never `clean`, `smudge` or `required`, whoever configured them.
pub fn pins() -> Vec<Pin> {
    pins_for(helper())
}

fn pins_for(helper: Option<&Helper>) -> Vec<Pin> {
    let process = if helper.is_some() { NAME } else { "" };
    [("process", process), ("clean", ""), ("smudge", ""), ("required", "false")]
        .into_iter()
        .map(|(var, value)| (OsString::from(format!("filter.lfs.{var}")), OsString::from(value)))
        .collect()
}

/// `filter.lfs` off, whatever else is pinned: for a fill that must not wait
/// on hydration.
pub fn off() -> Vec<Pin> {
    pins_for(None)
}

/// How a new worktree is filled (with [`off`]), and how one whose hydration
/// was cut short is put back to pointers. `reset --hard
/// --no-recurse-submodules` is what `worktree add` itself runs.
pub const FILL: &[&str] = &["reset", "-q", "--hard", "--no-recurse-submodules"];

/// Every path whose attributes say `filter=lfs`.
const LFS_PATHS: &str = ":(attr:filter=lfs)";

/// How long hydrating a new worktree may take, all objects together: long
/// enough for a few GB, short enough that a task's start isn't held for
/// longer than someone would wait for it.
pub const HYDRATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

#[cfg(test)]
thread_local! {
    /// [`HYDRATE_LIMIT`] for this thread's worktrees, for a test that wants a
    /// hydration to run out.
    pub(crate) static LIMIT: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
    /// Run by [`rehydrate`] once its files are written, standing in for an
    /// agent working in the worktree meanwhile.
    pub(crate) static WRITTEN: std::cell::Cell<Option<fn(&Path)>> = const { std::cell::Cell::new(None) };
}

fn hydrate_limit() -> std::time::Duration {
    #[cfg(test)]
    if let Some(limit) = LIMIT.with(std::cell::Cell::get) {
        return limit;
    }
    HYDRATE_LIMIT
}

/// Replace the LFS pointers in the freshly filled `worktree` with their
/// content, from the local store, within [`HYDRATE_LIMIT`]. Best effort:
/// nothing here fails the worktree. Answers the paths that are still pointer
/// files, which is what the worktree says it didn't download (ov-199): every
/// `filter=lfs` path after a hydration that failed or ran out, and after one
/// that finished, the ones the store had no object for ([`pointers`]).
///
/// git rewrites only what it sees as changed, and a pointer the fill just
/// wrote matches the index. So the `filter=lfs` entries are taken out of the
/// index (`rm --cached`, which touches no file), and `checkout HEAD` of those
/// paths writes them again through the filter and puts them back.
///
/// **The daemon itself touches no path in the worktree.** Every write and
/// unlink is git's, which refuses to go through a symlink on the way to a
/// path. The daemon's own `unlink` did: on a case-insensitive volume a commit
/// holding `A/x.bin` and a symlink `a -> <anywhere>` made it delete
/// `<anywhere>/x.bin` (ov-187 re-review).
///
/// One that fails or runs out was killed mid-write, so what it leaves is
/// undone: its `index.lock` removed (nothing else writes a worktree this new)
/// and the fill run again, which restores the index from `HEAD` and puts every
/// file back to its pointer. An object the store doesn't have stays a
/// pointer.
pub async fn hydrate(worktree: &Path) -> Vec<String> {
    let listed = crate::git::git_bytes(worktree, &["ls-files", "-z", "--", LFS_PATHS]).await;
    let listed: Vec<String> = match &listed {
        Ok(l) if l.ok => paths_of(&l.stdout),
        _ => return Vec::new(),
    };
    if listed.is_empty() {
        return listed;
    }
    // No helper beside the daemon: LFS is off, and every one of them stays a
    // pointer, which is what the worktree should say.
    if helper().is_none() {
        return listed;
    }
    let unlisted = crate::git::git(worktree, &["rm", "-r", "-q", "--cached", "--", LFS_PATHS]).await;
    if !matches!(&unlisted, Ok(u) if u.ok) {
        tracing::warn!("Git LFS files couldn't be marked for hydration; they stay pointers");
        return listed;
    }
    let limit = hydrate_limit();
    match crate::git::git_with(worktree, &["checkout", "-q", "HEAD", "--", LFS_PATHS], &[], limit).await {
        Ok(out) if out.ok => return pointers(worktree, &listed).await,
        Ok(out) => tracing::warn!(stderr = %out.stderr, "Git LFS files couldn't be hydrated; they stay pointers"),
        Err(_) => tracing::warn!(?limit, "Git LFS files took too long to hydrate; they stay pointers"),
    }
    match crate::git::git(worktree, &["rev-parse", "--absolute-git-dir"]).await {
        // git's own administrative directory for this worktree, which the
        // daemon made a moment ago; never a path the commit chose.
        Ok(dir) if dir.ok => {
            let lock = Path::new(dir.stdout.trim_end()).join("index.lock");
            if std::fs::symlink_metadata(&lock).is_ok_and(|m| m.is_file()) {
                let _ = std::fs::remove_file(&lock);
            }
        }
        _ => {}
    }
    if !matches!(crate::git::git_with(worktree, FILL, &off(), crate::git::GIT_TIMEOUT).await, Ok(f) if f.ok) {
        tracing::warn!(worktree = %worktree.display(), "a cut-short LFS hydration couldn't be undone");
    }
    listed
}

/// The NUL-separated paths `git ... -z` printed.
fn paths_of(stdout: &[u8]) -> Vec<String> {
    stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}

/// The first line of every LFS pointer, in the spelling git-lfs writes and
/// the older one it reads.
const POINTER_PREFIXES: [&[u8]; 2] =
    [b"version https://git-lfs.github.com/spec/v1", b"version https://hawser.github.com/spec/v1"];

/// A pointer file is a few lines; git-lfs's own limit on reading one is 1,024
/// bytes.
const POINTER_MAX: u64 = 1024;

/// Which of `paths` (relative to `worktree`) are LFS pointer files now.
///
/// From the files themselves, not the store: it is ground truth whether the
/// cause was a hydration that ran out, an object the store lacks, or a store
/// the helper's hash check rejected, it needs no git, and it notices an agent
/// that ran `git lfs pull` itself. A path that is gone, a directory, a
/// symlink, or anything beyond [`POINTER_MAX`] bytes is not one. Every
/// directory on the way is opened with `O_NOFOLLOW`
/// (`beneath::open_dir_beneath`), and so is the file, so a link an agent
/// swapped in is refused rather than read through.
pub async fn pointers(worktree: &Path, paths: &[String]) -> Vec<String> {
    let (worktree, paths) = (worktree.to_path_buf(), paths.to_vec());
    tokio::task::spawn_blocking(move || {
        paths.into_iter().filter(|path| is_pointer(&worktree, path)).collect()
    })
    .await
    .unwrap_or_default()
}

fn is_pointer(worktree: &Path, relative: &str) -> bool {
    use std::io::Read;
    use rustix::fs::{FileType, Mode, OFlags};
    let relative = Path::new(relative);
    let Ok((dirs, name)) = crate::beneath::split(relative) else { return false };
    let Ok(dir) = crate::beneath::open_dir_beneath(worktree, dirs, false) else { return false };
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let Ok(fd) = rustix::fs::openat(&dir, name, flags, Mode::empty()) else { return false };
    let Ok(stat) = rustix::fs::fstat(&fd) else { return false };
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile || stat.st_size as u64 > POINTER_MAX {
        return false;
    }
    let mut head = Vec::new();
    if std::fs::File::from(fd).take(POINTER_MAX).read_to_end(&mut head).is_err() {
        return false;
    }
    POINTER_PREFIXES.iter().any(|p| head.starts_with(p))
}

/// Try again to replace `recorded` pointers with their content, on a worktree
/// an agent may be working in. Answers the paths still pointer files.
///
/// **Not [`hydrate`].** Its failure path (`index.lock` removed, then `reset
/// --hard`) would wipe an agent's work, and its `rm --cached` window lets an
/// agent's `git commit` record the large files as deleted. This writes
/// through a throwaway index instead: `read-tree HEAD` makes one with no stat
/// data, so every entry reads as changed, and `checkout-index -f` writes the
/// files through the filter.
///
/// **The worktree's real index is never written**, not even to refresh it:
/// it is only read (`diff-index`). An `update-index` there, however short,
/// could land between an agent's `git add` and the check before it and put
/// HEAD's entry back over the agent's staged one, or make the agent's `git
/// commit` fail on `index.lock`. The cost is that the real index still holds
/// a written file's pointer size, so `git status` calls it modified;
/// `change_set` drops such a path when `git diff` finds no difference, which
/// is a read too.
///
/// Only paths that are pointers now and whose index entry is still HEAD's are
/// rewritten: a path the agent edited, staged or removed is theirs. Each
/// batch is checked for pointers again right before it is written, since the
/// batches before it may have taken minutes. A failed or timed-out write is
/// undone the same way with LFS off, which writes the pointer back over a
/// half-written file. The caller holds the repository's lock.
pub async fn rehydrate(worktree: &Path, recorded: &[String]) -> Vec<String> {
    let still = pointers(worktree, recorded).await;
    if still.is_empty() || helper().is_none() {
        return still;
    }
    let changed = crate::git::git_bytes(worktree, &["diff-index", "--cached", "--name-only", "-z", "HEAD"]).await;
    let Ok(changed) = changed else { return still };
    if !changed.ok {
        return still;
    }
    let changed: std::collections::HashSet<String> = paths_of(&changed.stdout).into_iter().collect();
    let untouched: Vec<&str> = still.iter().map(String::as_str).filter(|p| !changed.contains(*p)).collect();
    if untouched.is_empty() {
        return still;
    }
    let dir = match crate::git::git(worktree, &["rev-parse", "--absolute-git-dir"]).await {
        Ok(dir) if dir.ok => PathBuf::from(dir.stdout.trim_end()),
        _ => return still,
    };
    let index = dir.join(format!("fc-lfs-{}.index", std::process::id()));
    let deadline = tokio::time::Instant::now() + hydrate_limit();
    let wrote = rewrite(worktree, &index, &untouched, &[], deadline, true).await;
    #[cfg(test)]
    if let (Ok(()), Some(agent)) = (&wrote, WRITTEN.with(std::cell::Cell::get)) {
        agent(worktree);
    }
    if let Err(attempted) = wrote {
        tracing::warn!("Git LFS files couldn't be downloaded; they stay pointers");
        // With LFS off, whatever the cut-short write left becomes the pointer
        // again: only the batches it got to.
        let undo = tokio::time::Instant::now() + crate::git::GIT_TIMEOUT;
        let attempted: Vec<&str> = attempted.iter().map(String::as_str).collect();
        if rewrite(worktree, &index, &attempted, &off(), undo, false).await.is_err() {
            tracing::warn!(worktree = %worktree.display(), "a cut-short LFS retry couldn't be undone");
        }
    }
    let _ = std::fs::remove_file(&index);
    pointers(worktree, recorded).await
}

/// How many paths one `checkout-index` is given: an argument list has a limit.
const BATCH: usize = 200;

/// `read-tree HEAD` into `index`, then `checkout-index -f` of `paths` against
/// it, with `pins` after the usual ones, all within `deadline`. With
/// `recheck`, each batch keeps only the paths that are still pointers right
/// before it is written, so a file an agent filled meanwhile is left alone.
/// A failure answers every path a `checkout-index` was started on, for the
/// undo.
async fn rewrite(
    worktree: &Path,
    index: &Path,
    paths: &[&str],
    pins: &[crate::git_guard::Pin],
    deadline: tokio::time::Instant,
    recheck: bool,
) -> Result<(), Vec<String>> {
    let left = || deadline.saturating_duration_since(tokio::time::Instant::now());
    let read = crate::git::git_with_index(worktree, &["read-tree", "HEAD"], &[], left(), index).await;
    if !matches!(read, Ok(r) if r.ok) {
        return Err(Vec::new());
    }
    let mut attempted = Vec::new();
    for batch in paths.chunks(BATCH) {
        let batch: Vec<String> = batch.iter().map(|p| p.to_string()).collect();
        let batch = if recheck { pointers(worktree, &batch).await } else { batch };
        if batch.is_empty() {
            continue;
        }
        attempted.extend(batch.iter().cloned());
        let mut args = vec!["checkout-index", "-f", "-q", "--"];
        args.extend(batch.iter().map(String::as_str));
        let wrote = crate::git::git_with_index(worktree, &args, pins, left(), index).await;
        if !matches!(wrote, Ok(w) if w.ok) {
            return Err(attempted);
        }
    }
    Ok(())
}

/// `base` (the `PATH` git is otherwise handed) with the helper's directory
/// first, so the bare [`NAME`] in [`pins`] finds this helper before any other
/// of that name. `base` as it was when there is no helper.
pub fn path(base: Option<OsString>) -> Option<OsString> {
    path_for(helper(), base)
}

fn path_for(helper: Option<&Helper>, base: Option<OsString>) -> Option<OsString> {
    let Some(helper) = helper else { return base };
    let mut out = helper.dir.clone().into_os_string();
    if let Some(base) = base.filter(|b| !b.is_empty()) {
        out.push(":");
        out.push(base);
    }
    Some(out)
}

#[cfg(test)]
#[path = "git_lfs_pointer_tests.rs"]
mod pointer_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path, text: &[u8]) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_helper_is_found_beside_the_daemon_or_above_a_test_binary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("deps")).unwrap();
        executable(&root.join("farcoolerd"), b"\x7fELF");
        executable(&root.join("deps/test-1234"), b"\x7fELF");
        assert_eq!(locate(&root.join("farcoolerd")), None, "not there yet");

        executable(&root.join(NAME), b"\x7fELF");
        let want = Some(Helper { dir: root.clone(), program: root.join(NAME) });
        assert_eq!(locate(&root.join("farcoolerd")), want);
        assert_eq!(locate(&root.join("deps/test-1234")), want);

        // Through a symlink to the daemon, as `~/.local/bin` might hold it.
        let elsewhere = tempfile::tempdir().unwrap();
        let link = elsewhere.path().join("farcoolerd-canary");
        std::os::unix::fs::symlink(root.join("farcoolerd"), &link).unwrap();
        assert_eq!(locate(&link), want);

        // A script can't run inside the allowlist, so it isn't one.
        executable(&root.join(NAME), b"#!/bin/sh\n");
        assert_eq!(locate(&root.join("farcoolerd")), None);
    }

    fn fixture_git(dir: &Path, args: &[&str]) {
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
    }

    /// A hydration that runs out of time, here a 16 MB object against a
    /// limit a debug build can't stream it in, leaves a worktree all the
    /// same: with the pointer, clean, and no lock behind. Then, given time,
    /// the same worktree hydrates, so the first one really was cut short.
    #[tokio::test]
    async fn a_hydration_that_runs_out_still_leaves_a_worktree() {
        use sha2::{Digest, Sha256};
        if helper().is_none() {
            // `--lib` builds no binaries, and `helper()` is cached for the
            // process, so this test can't build one for itself after other
            // tests have asked. A full run (and CI's) builds it; there, its
            // absence is a failure, never a pass that ran nothing.
            assert!(
                std::env::var_os("CI").is_none(),
                "no {NAME} beside the test binary; `cargo test -p farcooler-daemon` builds it"
            );
            eprintln!(
                "SKIP a_hydration_that_runs_out_still_leaves_a_worktree: no {NAME} beside the test binary \
                 (`--lib` builds none; run `cargo test -p farcooler-daemon` or `cargo build -p farcooler-daemon --bins`)"
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        fixture_git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join(".gitattributes"), "*.bin filter=lfs -text\n").unwrap();
        let big: Vec<u8> = (0..16u32 << 20).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect();
        let oid: String = Sha256::digest(&big).iter().map(|b| format!("{b:02x}")).collect();
        let pointer = format!("version https://git-lfs.github.com/spec/v1\noid sha256:{oid}\nsize {}\n", big.len());
        std::fs::write(repo.join("big.bin"), &pointer).unwrap();
        let store = repo.join(".git/lfs/objects").join(&oid[0..2]).join(&oid[2..4]);
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join(&oid), &big).unwrap();
        fixture_git(&repo, &["add", "-A"]);
        fixture_git(&repo, &["commit", "-q", "-m", "base"]);

        // How long the stream takes depends on the machine: a quick one can finish
        // inside any fixed limit. So start at 150 ms and, while a worktree still came
        // out hydrated, make a fresh one with half the limit. A limit near zero
        // always runs out, so this ends, and every assertion below holds for the
        // worktree that was cut short.
        let mut limit = std::time::Duration::from_millis(150);
        let mut attempt = 0;
        let wt = loop {
            let wt = root.join(format!("wt{attempt}"));
            LIMIT.with(|l| l.set(Some(limit)));
            let made = crate::git::create_worktree(&repo, &format!("feature{attempt}"), "HEAD", &wt).await;
            LIMIT.with(|l| l.set(None));
            made.expect("the worktree is made, hydrated or not");
            if std::fs::read(wt.join("big.bin")).unwrap() == pointer.as_bytes() {
                break wt;
            }
            assert!(limit > std::time::Duration::from_micros(100), "never ran out, even at {limit:?}");
            limit /= 2;
            attempt += 1;
        };
        assert_eq!(std::fs::read(wt.join("big.bin")).unwrap(), pointer.as_bytes(), "back to its pointer");
        assert!(!crate::change_set::working_tree(&wt).await.unwrap().is_dirty(), "and clean");
        let git_dir = std::fs::read_to_string(wt.join(".git")).unwrap();
        let git_dir = Path::new(git_dir.trim_start_matches("gitdir: ").trim_end());
        assert!(!git_dir.join("index.lock").exists(), "no lock left behind");

        hydrate(&wt).await;
        assert!(std::fs::read(wt.join("big.bin")).unwrap() == big, "given time, it hydrates");
        assert!(!crate::change_set::working_tree(&wt).await.unwrap().is_dirty());
    }

    #[test]
    fn a_directory_with_a_colon_is_no_path_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("a:b");
        std::fs::create_dir(&root).unwrap();
        executable(&root.join("farcoolerd"), b"\x7fELF");
        executable(&root.join(NAME), b"\x7fELF");
        assert_eq!(locate(&root.join("farcoolerd")), None);
    }

    #[test]
    fn with_a_helper_lfs_is_the_helper_and_without_one_it_is_off() {
        let helper = Helper { dir: "/opt/fc".into(), program: "/opt/fc/farcooler-lfs-filter".into() };
        let pin = |k: &str, v: &str| (OsString::from(k), OsString::from(v));
        assert_eq!(
            pins_for(Some(&helper)),
            [
                pin("filter.lfs.process", NAME),
                pin("filter.lfs.clean", ""),
                pin("filter.lfs.smudge", ""),
                pin("filter.lfs.required", "false"),
            ]
        );
        assert_eq!(pins_for(None)[0], pin("filter.lfs.process", ""));

        assert_eq!(path_for(Some(&helper), Some("/usr/bin:/bin".into())), Some("/opt/fc:/usr/bin:/bin".into()));
        assert_eq!(path_for(Some(&helper), None), Some("/opt/fc".into()));
        assert_eq!(path_for(None, Some("/usr/bin".into())), Some("/usr/bin".into()));
    }
}

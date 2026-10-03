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
/// nothing here fails the worktree.
///
/// The `filter=lfs` files are removed, then a `checkout` of those paths
/// writes them again from the index through the filter and records them in
/// the index, so status sees them clean. One that fails or runs out was killed mid-write, so what it leaves
/// is undone: its `index.lock` removed (nothing else writes a worktree this
/// new) and the fill run again, which puts every file it didn't finish back
/// to its pointer. An object the store doesn't have stays a pointer.
pub async fn hydrate(worktree: &Path) {
    if helper().is_none() {
        return;
    }
    let listed = match crate::git::git_bytes(worktree, &["ls-files", "-z", "--", LFS_PATHS]).await {
        Ok(l) if l.ok && !l.stdout.is_empty() => l.stdout,
        _ => return,
    };
    // git rewrites only what it sees as changed, and a pointer the fill just
    // wrote matches the index; gone, it's written again through the filter.
    for path in listed.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = worktree.join(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(path));
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            let _ = std::fs::remove_file(&path);
        }
    }
    let limit = hydrate_limit();
    match crate::git::git_with(worktree, &["checkout", "-q", "--", LFS_PATHS], &[], limit).await {
        Ok(out) if out.ok => return,
        Ok(out) => tracing::warn!(stderr = %out.stderr, "Git LFS files couldn't be hydrated; they stay pointers"),
        Err(_) => tracing::warn!(?limit, "Git LFS files took too long to hydrate; they stay pointers"),
    }
    match crate::git::git(worktree, &["rev-parse", "--absolute-git-dir"]).await {
        Ok(dir) if dir.ok => {
            let _ = std::fs::remove_file(Path::new(dir.stdout.trim_end()).join("index.lock"));
        }
        _ => {}
    }
    if !matches!(crate::git::git_with(worktree, FILL, &off(), crate::git::GIT_TIMEOUT).await, Ok(f) if f.ok) {
        tracing::warn!(worktree = %worktree.display(), "a cut-short LFS hydration couldn't be undone");
    }
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
        assert!(helper().is_some(), "no {NAME} beside the test binary; `cargo test -p farcooler-daemon` builds it");
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

        let wt = root.join("wt");
        LIMIT.with(|l| l.set(Some(std::time::Duration::from_millis(150))));
        let made = crate::git::create_worktree(&repo, "feature", "HEAD", &wt).await;
        LIMIT.with(|l| l.set(None));
        made.expect("the worktree is made, hydrated or not");
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

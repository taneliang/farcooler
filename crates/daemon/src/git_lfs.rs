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

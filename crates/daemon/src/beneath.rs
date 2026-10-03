//! Directories inside a worktree, opened so that a write can't leave it.
//!
//! The launch writes two kinds of file into a worktree an agent is working
//! in: a hooks file (`service::install_project_hook_file`) and the codex
//! manager skill (`skill_install::install_file_beneath`). Both refuse a path
//! with a symbolic link on it (`skill_install::crosses_a_symlink`), but that
//! check runs before several gits, and an agent in the worktree can swap a
//! directory for a link in the meantime. A write by path would then follow
//! it out of the worktree.
//!
//! So the directory a file goes in is opened here, one component at a time
//! from the worktree, with `O_NOFOLLOW` on each, and the file is then read,
//! written and renamed relative to that descriptor. A link swapped in at any
//! point is refused, not followed, and so is one whose name only folds to the
//! one asked for (case, or NFD against NFC, on APFS).
//!
//! The worktree itself is opened the ordinary way. It is the path the daemon
//! made or was given, not one a commit chose, and an agent confined to it
//! can't rename it.

use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::path::{Component, Path};

use rustix::fs::{Mode, OFlags};

/// Open `dirs`, a relative path of directories, beneath `root`, making the
/// missing ones when `create` is set, and never through a link.
pub(crate) fn open_dir_beneath(root: &Path, dirs: &Path, create: bool) -> std::io::Result<OwnedFd> {
    let mut at = rustix::fs::open(root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    for component in dirs.components() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a plain relative path"));
        };
        if create {
            match rustix::fs::mkdirat(&at, name, Mode::from_raw_mode(0o777)) {
                Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(e.into()),
            }
        }
        at = rustix::fs::openat(&at, name, dir_flags(), Mode::empty())?;
    }
    Ok(at)
}

/// `relative` split into the directories above the file and its name.
pub(crate) fn split(relative: &Path) -> std::io::Result<(&Path, &OsStr)> {
    match (relative.parent(), relative.file_name()) {
        (Some(dirs), Some(name)) => Ok((dirs, name)),
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no file name")),
    }
}

/// A directory, never through a link.
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

/// Whether the volume holding `dir` folds case, as APFS does by default and
/// ext4 doesn't: a probe file made in `dir` is looked up by its uppercase
/// spelling. A link planted as `.AGENTS` names `.agents` only where this
/// says yes; elsewhere it is an unrelated directory, and a test of a
/// case-variant link has to know which it is testing.
#[cfg(test)]
pub(crate) fn folds_case(dir: &Path) -> bool {
    let probe = dir.join(".folds-case-probe");
    std::fs::write(&probe, "").expect("write the case probe");
    let folds = std::fs::symlink_metadata(dir.join(".FOLDS-CASE-PROBE")).is_ok();
    std::fs::remove_file(&probe).expect("remove the case probe");
    folds
}

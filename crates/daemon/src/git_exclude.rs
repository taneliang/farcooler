//! A line in a repository's `info/exclude`, written only into that file.
//!
//! The codex manager skill is untracked, so the daemon tells git to ignore it
//! (`service::install_project_skill`). The file it appends to is named by git
//! (`rev-parse --git-common-dir`), and git reads that from the worktree's
//! `.git`, which an agent working there can rewrite. A plain append by path
//! then follows wherever that and any symlink on the way lead: an agent could
//! have the daemon append our two lines to any file the user can write.
//!
//! So the write is held to the repository's own directory:
//!
//! - **The git dir is checked to belong to this worktree.** A main checkout's
//!   `.git` must be a real directory and the common dir itself. A linked
//!   worktree's git dir must be `<common>/worktrees/<name>`, and its `gitdir`
//!   file must point back at this worktree's `.git`. That file is git's, in
//!   the common dir, so a `.git` rewritten to name another repository fails
//!   the check. A submodule or `--separate-git-dir` checkout has a `.git`
//!   file instead, which proves nothing (it is the agent's to rewrite), so
//!   its git dir's own `core.worktree` must name this worktree (`points_back`).
//! - **Every directory is opened without following a link.** The common dir is
//!   resolved once, then opened one component at a time from `/` with
//!   `O_NOFOLLOW`, and `info` and `exclude` are opened from its descriptor. A
//!   link swapped in after the resolution, a linked `info`, a linked `exclude`,
//!   and their case-folded spellings on APFS are refused, not followed.
//! - **Only a regular file of the user's with one link is appended to.** A hard
//!   link to a file elsewhere passes `O_NOFOLLOW`, so the link count is
//!   checked. A missing file is made with `O_EXCL`, which never opens one that
//!   appeared in the meantime.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use rustix::fs::{FileType, FlockOperation, Mode, OFlags};

use crate::git;

/// The comment above our lines, written once however many follow it.
const WHOSE: &str = "# Far Cooler's manager skill for codex";

/// The most of an `info/exclude` or a `gitdir` file this reads: 1 MiB.
const MAX_READ: u64 = 1024 * 1024;

/// Add `/relative` to the repository's `info/exclude`, unless a line already
/// says exactly that, and say whether the line is there now.
///
/// `info/exclude` rather than `.gitignore`: it lives in the repository's own
/// directory, is never committed, and is shared by every worktree of it,
/// which is why the directory comes from `--git-common-dir` and not from the
/// worktree's `.git`, a file in a linked worktree. The leading `/` anchors the
/// pattern at a worktree's root, and the whole path names our file and nothing
/// beside it, so a file of the owner's own in the same directory is still
/// reported. An exclude line hides nothing git already tracks.
///
/// `false` whenever the file can't be shown to be this repository's own (see
/// the module comment): the caller then leaves the skill out.
pub async fn exclude_locally(worktree: &Path, relative: &str, deadline: tokio::time::Instant) -> bool {
    let args = ["rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir"];
    let Ok(out) = git::git_bytes_by(deadline, worktree, &args).await else {
        return false;
    };
    if !out.ok {
        return false;
    }
    // Bytes, not text: a path that isn't UTF-8 must name the directory git
    // reads, not a lossy copy of it. Exactly two lines, or a path held a
    // newline and the answer can't be split.
    let text = out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout);
    let lines: Vec<&[u8]> = text.split(|b| *b == b'\n').collect();
    let [git_dir, common] = lines[..] else { return false };
    let (git_dir, common) = (Path::new(OsStr::from_bytes(git_dir)), Path::new(OsStr::from_bytes(common)));
    match append_line(worktree, git_dir, common, &format!("/{relative}")) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(worktree = %worktree.display(), error = %e, "refusing to write this repository's info/exclude");
            false
        }
    }
}

/// The write, given what git said: `git_dir` and `common` as absolute paths.
fn append_line(worktree: &Path, git_dir: &Path, common: &Path, line: &str) -> std::io::Result<()> {
    let common_fd = open_beneath_root(&std::fs::canonicalize(common)?)?;
    belongs_to(worktree, git_dir, &common_fd)?;

    // `info` may be missing: `git init --template=` makes none.
    match rustix::fs::mkdirat(&common_fd, "info", Mode::from_raw_mode(0o755)) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(e) => return Err(e.into()),
    }
    let info = rustix::fs::openat(&common_fd, "info", dir_flags(), Mode::empty())?;
    let file = open_exclude(&info)?;
    let stat = rustix::fs::fstat(&file)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(refused("info/exclude is not a regular file"));
    }
    if stat.st_nlink != 1 {
        return Err(refused("info/exclude has another name, a hard link"));
    }
    if stat.st_uid != rustix::process::geteuid().as_raw() {
        return Err(refused("info/exclude belongs to another user"));
    }
    // Two launches at once must not both append.
    rustix::fs::flock(&file, FlockOperation::LockExclusive)?;
    let mut file = std::fs::File::from(file);
    let mut bytes = Vec::new();
    (&mut file).take(MAX_READ + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_READ {
        return Err(refused("info/exclude is too large"));
    }
    let existing = String::from_utf8(bytes).map_err(|_| refused("info/exclude is not UTF-8"))?;
    if existing.lines().any(|l| l.trim_end() == line) {
        return Ok(());
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    let said = existing.lines().any(|l| l.trim_end() == WHOSE);
    let comment = if said { String::new() } else { format!("{WHOSE}\n") };
    // `O_APPEND`: the write lands at the end whatever the read left.
    file.write_all(format!("{separator}{comment}{line}\n").as_bytes())
}

/// `exclude` in `info`, opened for appending and never through a link, made
/// when it's missing.
fn open_exclude(info: &OwnedFd) -> std::io::Result<OwnedFd> {
    // `O_NONBLOCK`: `O_NOFOLLOW` says nothing about a FIFO, which would hang
    // the open.
    let flags = OFlags::RDWR | OFlags::APPEND | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    for _ in 0..2 {
        match rustix::fs::openat(info, "exclude", flags, Mode::empty()) {
            Err(rustix::io::Errno::NOENT) => {}
            opened => return Ok(opened?),
        }
        match rustix::fs::openat(info, "exclude", flags | OFlags::CREATE | OFlags::EXCL, Mode::from_raw_mode(0o644)) {
            // Somebody made it between the two opens: open theirs, by the rules.
            Err(rustix::io::Errno::EXIST) => {}
            created => return Ok(created?),
        }
    }
    Err(refused("info/exclude kept changing"))
}

/// Refuse unless `git_dir`, the git dir git named for `worktree`, is the
/// repository's own record of it, in the common dir open as `common`.
fn belongs_to(worktree: &Path, git_dir: &Path, common: &OwnedFd) -> std::io::Result<()> {
    let tree = rustix::fs::open(worktree, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    let git_dir = std::fs::canonicalize(git_dir)?;
    let git_fd = open_beneath_root(&git_dir)?;
    if same(&git_fd, common)? {
        // A main checkout: its `.git` is the common dir, as a directory and
        // not a link to one.
        return match rustix::fs::openat(&tree, ".git", dir_flags(), Mode::empty()) {
            Ok(dot_git) if same(&dot_git, common)? => Ok(()),
            Ok(_) => Err(refused("this .git is not the repository git names")),
            // A submodule or a `--separate-git-dir` checkout: `.git` is a
            // file, and its own git dir is its common dir.
            Err(rustix::io::Errno::NOTDIR) => points_back(&tree, &git_fd, &git_dir),
            Err(e) => Err(e.into()),
        };
    }
    // A linked worktree: `<common>/worktrees/<name>`, opened from the common
    // dir, and its `gitdir` names this worktree's `.git`.
    let name = git_dir.file_name().ok_or_else(|| refused("the git dir has no name"))?;
    let worktrees = rustix::fs::openat(common, "worktrees", dir_flags(), Mode::empty())?;
    let admin = rustix::fs::openat(&worktrees, name, dir_flags(), Mode::empty())?;
    if !same(&admin, &git_fd)? {
        return Err(refused("the git dir is not one of the repository's worktrees"));
    }
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let file = rustix::fs::openat(&admin, "gitdir", flags, Mode::empty())?;
    if FileType::from_raw_mode(rustix::fs::fstat(&file)?.st_mode) != FileType::RegularFile {
        return Err(refused("gitdir is not a regular file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::from(file).take(MAX_READ).read_to_end(&mut bytes)?;
    let named = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    // Relative to the admin dir under `worktree.useRelativePaths`.
    let named = git_dir.join(OsStr::from_bytes(named));
    if named.file_name() != Some(OsStr::new(".git")) {
        return Err(refused("gitdir doesn't name a .git"));
    }
    let named_tree = named.parent().ok_or_else(|| refused("gitdir names no worktree"))?;
    let named_tree = rustix::fs::open(named_tree, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    if same(&named_tree, &tree)? { Ok(()) } else { Err(refused("the repository records another worktree here")) }
}

/// For a worktree whose `.git` is a file: refuse unless the git dir's own
/// config makes this worktree its own.
///
/// The `.git` file is the agent's to rewrite, so it proves nothing: git
/// followed it to `git_dir`, and an agent could have named any repository's.
/// The git dir's `config` is git's own and sits outside the worktree. A
/// submodule's records `core.worktree`, which must name this directory. A
/// `--separate-git-dir` checkout records nothing, so it is accepted only when
/// its git dir is a non-bare repository that isn't some checkout's own `.git`
/// directory: the two shapes an agent could borrow are a bare repository and
/// a plain `.git`. The file, the config and every directory on the way are
/// opened without following a link.
fn points_back(tree: &OwnedFd, git_fd: &OwnedFd, git_dir: &Path) -> std::io::Result<()> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let dot_git = rustix::fs::openat(tree, ".git", flags, Mode::empty())?;
    if FileType::from_raw_mode(rustix::fs::fstat(&dot_git)?.st_mode) != FileType::RegularFile {
        return Err(refused("this .git is neither a directory nor a regular file"));
    }
    let config = rustix::fs::openat(git_fd, "config", flags, Mode::empty())?;
    if FileType::from_raw_mode(rustix::fs::fstat(&config)?.st_mode) != FileType::RegularFile {
        return Err(refused("the git dir's config is not a regular file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::from(config).take(MAX_READ).read_to_end(&mut bytes)?;
    let (worktree, bare) = core_settings(&String::from_utf8_lossy(&bytes))?;
    let Some(named) = worktree else {
        if bare || git_dir.file_name() == Some(OsStr::new(".git")) {
            return Err(refused("the git dir records no worktree and isn't a separate git dir"));
        }
        return Ok(());
    };
    // Relative to the git dir, as git reads it. Resolved only to compare
    // which directory it is, never to write there.
    let named = std::fs::canonicalize(git_dir.join(named))?;
    let named = rustix::fs::open(&named, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    if same(&named, tree)? { Ok(()) } else { Err(refused("the git dir records another worktree")) }
}

/// `core.worktree` (unquoted) and `core.bare` from the text of a git config.
/// Only a `[core]` section's own lines; git's escapes and includes are not
/// read, so a value with a backslash or a quote is a refusal.
fn core_settings(config: &str) -> std::io::Result<(Option<String>, bool)> {
    let (mut in_core, mut worktree, mut bare) = (false, None, false);
    for line in config.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            in_core = header.trim_end_matches(']').trim().eq_ignore_ascii_case("core");
        } else if in_core && let Some((key, value)) = line.split_once('=') {
            let (key, value) = (key.trim(), value.trim());
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            if value.contains(['\\', '"']) {
                return Err(refused("the git dir's config has a value this doesn't read"));
            }
            if key.eq_ignore_ascii_case("worktree") {
                worktree = Some(value.to_string());
            } else if key.eq_ignore_ascii_case("bare") {
                bare = value.eq_ignore_ascii_case("true");
            }
        }
    }
    Ok((worktree, bare))
}

/// Open an absolute, resolved directory one component at a time from `/`,
/// refusing a link at any of them.
fn open_beneath_root(path: &Path) -> std::io::Result<OwnedFd> {
    let mut at = rustix::fs::open("/", OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => at = rustix::fs::openat(&at, name, dir_flags(), Mode::empty())?,
            _ => return Err(refused("not a resolved absolute path")),
        }
    }
    Ok(at)
}

/// A directory, never through a link.
fn dir_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
}

/// Whether two descriptors are the same file.
fn same(a: &OwnedFd, b: &OwnedFd) -> std::io::Result<bool> {
    let (a, b) = (rustix::fs::fstat(a)?, rustix::fs::fstat(b)?);
    Ok(a.st_dev == b.st_dev && a.st_ino == b.st_ino)
}

fn refused(why: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, why.to_string())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod gitfile_tests;

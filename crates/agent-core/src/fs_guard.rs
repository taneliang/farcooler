//! Where an agent is allowed to read and write.
//!
//! Resolution before comparison, always. A prefix check on the path as written
//! passes `worktree/escape/passwd` when `escape` is a symlink to `/etc`, which
//! is the whole attack. The deepest existing ancestor is canonicalized and the
//! remaining components are appended, so a file that does not exist yet is
//! still judged by where it would actually land.
//!
//! "Does not exist" means no directory entry at all. A symlink whose target is
//! missing also fails `canonicalize`, but it is an entry, and a write through
//! it creates its target, wherever that is. So it is refused rather than taken
//! for a new name.
//!
//! Judging a path is not enough on its own: the tree can change between the
//! check and the open. `open_confined` closes that gap by walking the judged
//! path from a descriptor held on the worktree, one component at a time, with
//! `O_NOFOLLOW` on every one, so a symlink swapped in after the check is an
//! error rather than a redirect.
//!
//! What this does not catch is a hard link inside the worktree to a file
//! outside it. A repository cannot ship one (git has no hard links), so making
//! one takes a process already running as the user.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{Mode, OFlags};
use rustix::io::Errno;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FsGuardError {
    #[error("path resolves outside the worktree")]
    Escapes,
    #[error("worktree path could not be resolved")]
    BadWorktree,
}

/// Resolve `requested` and return it only if it lands inside `worktree`.
pub fn confine(worktree: &Path, requested: &Path) -> Result<PathBuf, FsGuardError> {
    let root = std::fs::canonicalize(worktree).map_err(|_| FsGuardError::BadWorktree)?;
    confine_in(&root, requested)
}

/// `confine`, with the worktree already canonical.
fn confine_in(root: &Path, requested: &Path) -> Result<PathBuf, FsGuardError> {
    let absolute =
        if requested.is_absolute() { requested.to_path_buf() } else { root.join(requested) };

    // Canonicalize the deepest ancestor that exists, then re-append the rest.
    // `canonicalize` on a missing path fails outright, and every file creation
    // is a missing path.
    let mut existing = absolute.as_path();
    let mut tail: Vec<Component<'_>> = Vec::new();
    let resolved_head = loop {
        match std::fs::canonicalize(existing) {
            Ok(p) => break p,
            Err(_) => {
                // Only a name with no entry behind it is new. A dangling or
                // looping symlink, or one in a directory that cannot be read,
                // has an entry, and where it leads cannot be known: refused.
                match std::fs::symlink_metadata(existing) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    _ => return Err(FsGuardError::Escapes),
                }
                match existing.parent() {
                    Some(parent) => {
                        if let Some(name) = existing.components().next_back() {
                            tail.push(name);
                        }
                        existing = parent;
                    }
                    None => return Err(FsGuardError::Escapes),
                }
            }
        }
    };

    let mut resolved = resolved_head;
    for component in tail.into_iter().rev() {
        match component {
            // A `..` that survived to here would climb out of the resolved
            // head, so it is refused rather than normalized away.
            Component::ParentDir => return Err(FsGuardError::Escapes),
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }

    if resolved.starts_with(root) { Ok(resolved) } else { Err(FsGuardError::Escapes) }
}

/// What a confined open is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Read an existing file.
    Read,
    /// Read and write a file, creating it and any missing parent directories.
    Write,
}

/// Why a confined open failed: the path was refused, or the filesystem said no.
///
/// Kept apart because they mean different things to the agent. "Outside the
/// worktree" is a request it must not repeat; "no such file" or "permission
/// denied" is an ordinary answer about a path it was allowed to ask for.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Refused(#[from] FsGuardError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A file opened inside the worktree.
#[derive(Debug)]
pub struct Confined {
    pub file: File,
    /// Where it is, resolved: what `confine` returned.
    pub path: PathBuf,
    /// Whether this open created it. Only ever true for `Access::Write`.
    pub created: bool,
}

/// `confine`, then open what it judged without following any symlink.
///
/// Every component below the worktree is opened with `openat` relative to the
/// directory before it, with `O_NOFOLLOW`. The path `confine` returns has no
/// symlinks left in it, so any symlink found on the way was put there after
/// the check, and is refused. Only a regular file is opened; a FIFO is opened
/// with `O_NONBLOCK` so that refusing it cannot hang.
pub fn open_confined(
    worktree: &Path,
    requested: &Path,
    access: Access,
) -> Result<Confined, OpenError> {
    let root = std::fs::canonicalize(worktree).map_err(|_| FsGuardError::BadWorktree)?;
    let path = confine_in(&root, requested)?;
    let (file, created) = open_beneath(&root, &path, access)?;
    Ok(Confined { file: File::from(file), path, created })
}

fn open_beneath(root: &Path, path: &Path, access: Access) -> Result<(OwnedFd, bool), OpenError> {
    let relative = path.strip_prefix(root).map_err(|_| FsGuardError::Escapes)?;
    let mut names: Vec<&OsStr> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => names.push(name),
            _ => return Err(FsGuardError::Escapes.into()),
        }
    }
    let Some((leaf, parents)) = names.split_last() else {
        // The worktree itself, which is a directory and not a file.
        return Err(io::Error::from(io::ErrorKind::IsADirectory).into());
    };

    let directory = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    // The worktree itself is canonical, and opened the ordinary way.
    let mut held = rustix::fs::open(root, directory, Mode::empty()).map_err(refusal)?;
    for name in parents {
        if access == Access::Write {
            match rustix::fs::mkdirat(&held, *name, Mode::from_raw_mode(0o777)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(e) => return Err(refusal(e)),
            }
        }
        held = rustix::fs::openat(&held, *name, directory | OFlags::NOFOLLOW, Mode::empty())
            .map_err(refusal)?;
    }

    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let (file, created) = match access {
        Access::Read => (rustix::fs::openat(&held, *leaf, flags, Mode::empty()), false),
        Access::Write => {
            let flags = (flags - OFlags::RDONLY) | OFlags::RDWR;
            match rustix::fs::openat(&held, *leaf, flags, Mode::empty()) {
                Err(Errno::NOENT) => (
                    rustix::fs::openat(
                        &held,
                        *leaf,
                        flags | OFlags::CREATE | OFlags::EXCL,
                        Mode::from_raw_mode(0o666),
                    ),
                    true,
                ),
                opened => (opened, false),
            }
        }
    };
    let file = file.map_err(refusal)?;
    let kind = rustix::fs::FileType::from_raw_mode(rustix::fs::fstat(&file).map_err(refusal)?.st_mode);
    if kind != rustix::fs::FileType::RegularFile {
        return Err(io::Error::other("not a regular file").into());
    }
    Ok((file, created))
}

/// An `openat` failure, with a symlink met under `O_NOFOLLOW` read as what it
/// is: something that changed after the check, and would have led elsewhere.
///
/// `ELOOP` is what both Linux and macOS give for a symlink as the last
/// component. As a directory component (`O_DIRECTORY` too) macOS gives
/// `ENOTDIR`, which is also what a plain file in that place gives; either way
/// nothing was opened.
fn refusal(e: Errno) -> OpenError {
    if e == Errno::LOOP { FsGuardError::Escapes.into() } else { io::Error::from(e).into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn worktree() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("farcooler-guard-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("src"));
        fs::canonicalize(&dir).expect("temp worktree")
    }

    #[test]
    fn a_path_inside_the_worktree_is_allowed() {
        let wt = worktree();
        let ok = confine(&wt, &wt.join("src/main.rs")).expect("inside is allowed");
        assert!(ok.starts_with(&wt));
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_allowed_inside() {
        // Every create goes through this. Requiring the file to exist would
        // make the capability useless for new files.
        let wt = worktree();
        assert!(confine(&wt, &wt.join("src/brand_new.rs")).is_ok());
    }

    #[test]
    fn dot_dot_cannot_climb_out() {
        let wt = worktree();
        let err = confine(&wt, &wt.join("../../etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn an_absolute_path_elsewhere_is_refused() {
        let wt = worktree();
        let err = confine(&wt, std::path::Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_symlink_pointing_out_is_refused() {
        // The one that a naive prefix check on the unresolved string misses,
        // and the reason resolution has to happen before comparison.
        let wt = worktree();
        let link = wt.join("escape");
        let _ = fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", &link).expect("symlink");
        let err = confine(&wt, &link.join("passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_symlink_that_is_itself_the_requested_path_is_refused() {
        // Distinct from the directory-symlink case above: here the symlink
        // *is* the whole requested path, so `canonicalize` resolves it on the
        // first attempt with an empty tail, and the escape check still has to
        // catch it — the comparison can't rely on the tail-reappend loop ever
        // running.
        let wt = worktree();
        let link = wt.join("direct");
        let _ = fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", &link).expect("symlink");
        let err = confine(&wt, &link).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_sibling_directory_with_a_prefix_matching_name_is_refused() {
        // `starts_with` must compare path components, not raw strings. A
        // string-prefix check would let this through, because the text
        // "farcooler-guard-123-evil" starts with the text
        // "farcooler-guard-123" even though the paths are unrelated siblings.
        let wt = worktree();
        let sibling = PathBuf::from(format!("{}-evil", wt.display()));
        let _ = fs::create_dir_all(&sibling);
        let sibling = fs::canonicalize(&sibling).expect("sibling worktree");
        let err = confine(&wt, &sibling.join("file.txt")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_dot_dot_cancelled_out_by_a_real_directory_is_still_allowed() {
        // `src/../src/x` is not an escape attempt — the `..` is consumed by
        // canonicalizing the existing ancestor, not by the tail-reappend
        // refusal. This guards against the escape fix becoming so strict it
        // rejects ordinary paths a caller never wrote by hand but that a
        // library might construct.
        let wt = worktree();
        let ok = confine(&wt, &wt.join("src/../src/new_file.rs")).expect("resolves back inside");
        assert!(ok.starts_with(&wt));
    }

    /// A directory outside every test worktree, which never exists, for
    /// symlinks to point into.
    fn outside(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("farcooler-guard-outside-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn relink(target: &Path, link: &Path) {
        let _ = fs::remove_file(link);
        std::os::unix::fs::symlink(target, link).expect("symlink");
    }

    #[test]
    fn a_dangling_symlink_pointing_out_is_refused() {
        // A repository can ship this link. `canonicalize` fails on it because
        // its target is missing, so it used to be taken for a name that does
        // not exist yet, re-appended under the worktree, and passed. A write
        // then followed it and created the target outside.
        let wt = worktree();
        let away = outside("dangling");
        let link = wt.join("dangling");
        relink(&away.join("missing"), &link);
        assert_eq!(confine(&wt, &link), Err(FsGuardError::Escapes));
        assert_eq!(confine(&wt, Path::new("dangling")), Err(FsGuardError::Escapes));
    }

    #[test]
    fn a_chain_of_symlinks_ending_outside_is_refused() {
        let wt = worktree();
        let away = outside("chain");
        relink(&away.join("missing"), &wt.join("chain-2"));
        relink(&wt.join("chain-2"), &wt.join("chain-1"));
        assert_eq!(confine(&wt, &wt.join("chain-1")), Err(FsGuardError::Escapes));
    }

    #[test]
    fn a_name_under_a_dangling_directory_symlink_is_refused() {
        // `dir/new/file.rs` where `dir` points at a directory that does not
        // exist: every component fails to resolve, and creating the parents
        // would make them outside.
        let wt = worktree();
        let away = outside("dir");
        relink(&away, &wt.join("dir-link"));
        let err = confine(&wt, &wt.join("dir-link/new/file.rs")).unwrap_err();
        assert_eq!(err, FsGuardError::Escapes);
    }

    #[test]
    fn an_absolute_path_outside_that_does_not_exist_is_refused() {
        let wt = worktree();
        let away = outside("absolute");
        assert_eq!(confine(&wt, &away.join("new.rs")), Err(FsGuardError::Escapes));
    }

    #[test]
    fn dot_dot_after_a_missing_directory_cannot_climb_out() {
        let wt = worktree();
        let err = confine(&wt, Path::new("missing/../../outside.rs")).unwrap_err();
        assert_eq!(err, FsGuardError::Escapes);
    }

    #[test]
    fn a_symlink_that_stays_inside_still_resolves() {
        // Refusing symlinks outright would break a repository that links one
        // of its own files to another. Those resolve, and land inside.
        let wt = worktree();
        fs::write(wt.join("src/real.rs"), "").unwrap();
        relink(&wt.join("src/real.rs"), &wt.join("inside-link"));
        let ok = confine(&wt, &wt.join("inside-link")).expect("an inside link is allowed");
        assert_eq!(ok, wt.join("src/real.rs"));
    }

    #[test]
    fn a_symlink_swapped_in_after_the_check_is_not_followed() {
        // The race: `confine` judges a name that does not exist yet, and a
        // symlink appears there before the open. Following it is the escape.
        let wt = worktree();
        let away = outside("swap");
        fs::create_dir_all(&away).unwrap();
        let _ = fs::remove_file(wt.join("swapped"));
        let judged = confine(&wt, Path::new("swapped")).expect("a new name inside");
        relink(&away.join("planted"), &wt.join("swapped"));
        let err = open_beneath(&wt, &judged, Access::Write).unwrap_err();
        assert!(matches!(err, OpenError::Refused(FsGuardError::Escapes)), "{err:?}");
        assert!(!away.join("planted").exists());
    }

    #[test]
    fn a_directory_swapped_for_a_symlink_after_the_check_is_not_followed() {
        let wt = worktree();
        let away = outside("swap-dir");
        fs::create_dir_all(&away).unwrap();
        let _ = fs::remove_file(wt.join("swap-dir"));
        let judged = confine(&wt, Path::new("swap-dir/file.rs")).expect("a new name inside");
        relink(&away, &wt.join("swap-dir"));
        assert!(open_beneath(&wt, &judged, Access::Write).is_err());
        assert!(!away.join("file.rs").exists());
    }

    #[test]
    fn a_write_creates_its_parent_directories_inside() {
        let wt = worktree();
        let _ = fs::remove_dir_all(wt.join("made"));
        let opened = open_confined(&wt, Path::new("made/deeper/new.rs"), Access::Write)
            .expect("created inside");
        assert!(opened.created);
        assert_eq!(opened.path, wt.join("made/deeper/new.rs"));
        assert!(wt.join("made/deeper/new.rs").is_file());
        let again = open_confined(&wt, Path::new("made/deeper/new.rs"), Access::Write).unwrap();
        assert!(!again.created, "it exists now");
    }

    #[test]
    fn a_read_of_a_missing_file_is_an_io_error_not_a_refusal() {
        let wt = worktree();
        let err = open_confined(&wt, Path::new("src/absent.rs"), Access::Read).unwrap_err();
        assert!(
            matches!(&err, OpenError::Io(e) if e.kind() == io::ErrorKind::NotFound),
            "{err:?}"
        );
    }

    #[test]
    fn a_fifo_is_refused_without_hanging() {
        let wt = worktree();
        let fifo = wt.join("fifo");
        let _ = fs::remove_file(&fifo);
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
        assert!(made.success());
        assert!(open_confined(&wt, &fifo, Access::Read).is_err());
    }
}

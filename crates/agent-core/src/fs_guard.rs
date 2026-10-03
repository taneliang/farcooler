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
//! Before anything on disk is looked at, a path is judged by its spelling: no
//! `..`, and an absolute path must be written under the worktree. Otherwise
//! whether `/elsewhere/x/../../worktree/a` is allowed would depend on whether
//! `/elsewhere/x` exists, and the answer would tell the agent.
//!
//! Judging a path is not enough on its own: the tree can change between the
//! check and the open. `Worktree` closes that gap by walking the judged path
//! from a descriptor held on the worktree, one component at a time, with
//! `O_NOFOLLOW` on every one, so a symlink swapped in after the check is
//! refused rather than followed.
//!
//! A write never changes a file in place. It goes to a new file beside the
//! target and is renamed over it, so a hard link inside the worktree to a file
//! outside it (pnpm's `node_modules` is made of them) is replaced, and the
//! file outside is left alone. That also makes the write atomic: a full disk
//! leaves the old file whole, not truncated.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

/// The most an agent may read in one request, and the most of a file's old
/// text a write keeps for its diff: 16 MiB.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FsGuardError {
    #[error("path resolves outside the worktree")]
    Escapes,
    #[error("worktree path could not be resolved")]
    BadWorktree,
}

/// Why a confined read or write failed.
///
/// Refusal is kept apart from the rest because it means something different
/// to the agent. "Outside the worktree" is a request it must not repeat; "no
/// such file" is an ordinary answer about a path it was allowed to ask for.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error(transparent)]
    Refused(#[from] FsGuardError),
    #[error("not a regular file")]
    NotAFile,
    #[error("larger than the limit")]
    TooLarge,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Resolve `requested` and return it only if it lands inside `worktree`.
///
/// Fails closed: a path that could not be judged at all, for instance behind
/// a directory that cannot be read, is refused too. `Worktree` tells the two
/// apart.
pub fn confine(worktree: &Path, requested: &Path) -> Result<PathBuf, FsGuardError> {
    let root = std::fs::canonicalize(worktree).map_err(|_| FsGuardError::BadWorktree)?;
    let given = std::path::absolute(worktree).unwrap_or_else(|_| root.clone());
    confine_in(&given, &root, requested).map_err(|e| match e {
        OpenError::Refused(e) => e,
        _ => FsGuardError::Escapes,
    })
}

/// `confine`, with the worktree as it was spelled and as it resolved.
fn confine_in(given: &Path, root: &Path, requested: &Path) -> Result<PathBuf, OpenError> {
    // Judged by spelling first, touching nothing. See the module doc.
    if requested.components().any(|c| c == Component::ParentDir) {
        return Err(FsGuardError::Escapes.into());
    }
    let absolute = if requested.is_absolute() {
        if !requested.starts_with(root) && !requested.starts_with(given) {
            return Err(FsGuardError::Escapes.into());
        }
        requested.to_path_buf()
    } else {
        root.join(requested)
    };

    // Canonicalize the deepest ancestor that exists, then re-append the rest.
    // `canonicalize` on a missing path fails outright, and every file creation
    // is a missing path.
    let mut existing = absolute.as_path();
    let mut tail: Vec<Component<'_>> = Vec::new();
    let resolved_head = loop {
        match std::fs::canonicalize(existing) {
            Ok(p) => break p,
            Err(unresolved) => {
                // Only a name with no entry behind it is new. `ENOTDIR` counts:
                // a name under a regular file is not there either, and the
                // open will say so. A dangling or looping symlink has an
                // entry, and where it leads cannot be known: refused.
                match std::fs::symlink_metadata(existing) {
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                        ) => {}
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return Err(FsGuardError::Escapes.into());
                    }
                    Ok(_) => return Err(unresolved.into()),
                    Err(e) => return Err(e.into()),
                }
                match existing.parent() {
                    Some(parent) => {
                        if let Some(name) = existing.components().next_back() {
                            tail.push(name);
                        }
                        existing = parent;
                    }
                    None => return Err(FsGuardError::Escapes.into()),
                }
            }
        }
    };

    let mut resolved = resolved_head;
    for component in tail.into_iter().rev() {
        match component {
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }

    if resolved.starts_with(root) { Ok(resolved) } else { Err(FsGuardError::Escapes.into()) }
}

/// A worktree an agent reads and writes in, held open by descriptor.
///
/// Opened once, so every later request walks from the same directory even if
/// something renames a directory above it.
#[derive(Debug)]
pub struct Worktree {
    given: PathBuf,
    root: PathBuf,
    fd: OwnedFd,
}

/// What a write replaced.
#[derive(Debug)]
pub struct Written {
    /// Where it is, resolved: what `confine` returned.
    pub path: PathBuf,
    /// Whether the file is new.
    pub created: bool,
    /// The file's bytes before, when it existed and was no more than
    /// `MAX_BYTES`.
    pub before: Option<Vec<u8>>,
}

/// The directories a write walked through, and which of them it made.
struct Walk {
    dirs: Vec<OwnedFd>,
    made: Vec<usize>,
}

impl Worktree {
    pub fn open(path: &Path) -> Result<Self, FsGuardError> {
        let root = std::fs::canonicalize(path).map_err(|_| FsGuardError::BadWorktree)?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let fd = rustix::fs::open(&root, flags, Mode::empty())
            .map_err(|_| FsGuardError::BadWorktree)?;
        let given = std::path::absolute(path).unwrap_or_else(|_| root.clone());
        Ok(Self { given, root, fd })
    }

    /// The worktree, resolved.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `confine`, telling a refusal apart from a path that could not be
    /// looked at.
    pub fn confine(&self, requested: &Path) -> Result<PathBuf, OpenError> {
        confine_in(&self.given, &self.root, requested)
    }

    /// Read a regular file of at most `MAX_BYTES`.
    pub fn read(&self, requested: &Path) -> Result<(PathBuf, Vec<u8>), OpenError> {
        let path = self.confine(requested)?;
        let bytes = self.read_judged(&path)?;
        Ok((path, bytes))
    }

    /// Replace a file's contents, creating it and its parent directories.
    pub fn write(&self, requested: &Path, contents: &[u8]) -> Result<Written, OpenError> {
        let path = self.confine(requested)?;
        let (created, before) = self.write_judged(&path, contents)?;
        Ok(Written { path, created, before })
    }

    /// The read, for a path already judged. Split out so a test can change
    /// the tree between the judgement and the open.
    fn read_judged(&self, path: &Path) -> Result<Vec<u8>, OpenError> {
        let names = self.names(path)?;
        let (leaf, parents) = names.split_last().ok_or(io::Error::from(io::ErrorKind::IsADirectory))?;
        let walk = self.walk(parents, false)?;
        let at = self.at(&walk);
        let file = rustix::fs::openat(at, *leaf, read_flags(), Mode::empty()).map_err(leaf_refusal)?;
        match read_bounded(file)? {
            Some(bytes) => Ok(bytes),
            None => Err(OpenError::TooLarge),
        }
    }

    /// The write, for a path already judged.
    fn write_judged(
        &self,
        path: &Path,
        contents: &[u8],
    ) -> Result<(bool, Option<Vec<u8>>), OpenError> {
        let names = self.names(path)?;
        let (leaf, parents) = names.split_last().ok_or(io::Error::from(io::ErrorKind::IsADirectory))?;
        let walk = self.walk(parents, true)?;
        let outcome = replace(self.at(&walk), leaf, contents);
        if outcome.is_err() {
            self.undo(&walk, &names);
        }
        outcome
    }

    /// The components of a judged path below the root.
    fn names<'p>(&self, path: &'p Path) -> Result<Vec<&'p OsStr>, OpenError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| FsGuardError::Escapes)?;
        relative
            .components()
            .map(|c| match c {
                Component::Normal(name) => Ok(name),
                _ => Err(FsGuardError::Escapes.into()),
            })
            .collect()
    }

    /// Open each directory in turn from the one before, making it first when
    /// `create` is set.
    fn walk(&self, names: &[&OsStr], create: bool) -> Result<Walk, OpenError> {
        let mut walk = Walk { dirs: Vec::new(), made: Vec::new() };
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        for (i, name) in names.iter().enumerate() {
            let at = self.at(&walk);
            let made = create
                && match rustix::fs::mkdirat(at, *name, Mode::from_raw_mode(0o777)) {
                    Ok(()) => true,
                    Err(Errno::EXIST) => false,
                    Err(e) => {
                        self.undo(&walk, names);
                        return Err(io::Error::from(e).into());
                    }
                };
            let opened = rustix::fs::openat(at, *name, flags, Mode::empty())
                .map_err(|e| directory_refusal(at, name, e));
            if made {
                walk.made.push(i);
            }
            match opened {
                Ok(dir) => walk.dirs.push(dir),
                Err(refused) => {
                    self.undo(&walk, names);
                    return Err(refused);
                }
            }
        }
        Ok(walk)
    }

    /// The deepest directory a walk reached.
    fn at<'a>(&'a self, walk: &'a Walk) -> BorrowedFd<'a> {
        walk.dirs.last().map(|d| d.as_fd()).unwrap_or(self.fd.as_fd())
    }

    /// Remove the directories a failed write made, deepest first. Only empty
    /// ones go: `AT_REMOVEDIR` never removes anything somebody else put there.
    fn undo(&self, walk: &Walk, names: &[&OsStr]) {
        for &i in walk.made.iter().rev() {
            let parent = if i == 0 { self.fd.as_fd() } else { walk.dirs[i - 1].as_fd() };
            let _ = rustix::fs::unlinkat(parent, names[i], AtFlags::REMOVEDIR);
        }
    }
}

/// Read a file, or a FIFO without hanging on it.
fn read_flags() -> OFlags {
    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK
}

/// A regular file's bytes, or `None` past `MAX_BYTES`.
fn read_bounded(file: OwnedFd) -> Result<Option<Vec<u8>>, OpenError> {
    let stat = rustix::fs::fstat(&file).map_err(io::Error::from)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(OpenError::NotAFile);
    }
    // Told by its size first, so a sparse file of any size costs nothing. The
    // `take` is for one that grows while it is read.
    if stat.st_size as u64 > MAX_BYTES as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    File::from(file).take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= MAX_BYTES).then_some(bytes))
}

/// Write `contents` to a new file in `dir` and rename it over `leaf`.
///
/// The new file keeps the old one's permission bits, so a script stays
/// executable; set-id bits are not carried over.
fn replace(
    dir: BorrowedFd<'_>,
    leaf: &OsStr,
    contents: &[u8],
) -> Result<(bool, Option<Vec<u8>>), OpenError> {
    let (created, before, mode) = match rustix::fs::openat(dir, leaf, read_flags(), Mode::empty()) {
        Ok(file) => {
            let mode = rustix::fs::fstat(&file).map_err(io::Error::from)?.st_mode & 0o777;
            (false, read_bounded(file)?, Some(mode))
        }
        Err(Errno::NOENT) => (true, None, None),
        Err(e) => return Err(leaf_refusal(e)),
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let temp = format!(
        ".farcooler-write-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let file = rustix::fs::openat(dir, temp.as_str(), flags, Mode::from_raw_mode(0o666))
        .map_err(io::Error::from)?;
    let written = (|| -> io::Result<()> {
        if let Some(mode) = mode {
            rustix::fs::fchmod(&file, Mode::from_raw_mode(mode))?;
        }
        let mut file = File::from(file);
        file.write_all(contents)?;
        file.sync_all()?;
        rustix::fs::renameat(dir, temp.as_str(), dir, leaf)?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = rustix::fs::unlinkat(dir, temp.as_str(), AtFlags::empty());
        return Err(e.into());
    }
    Ok((created, before))
}

/// A leaf `openat` failure, with a symlink met under `O_NOFOLLOW` read as what
/// it is: something that changed after the check, and would have led
/// elsewhere. Linux and macOS both say `ELOOP` for it.
fn leaf_refusal(e: Errno) -> OpenError {
    if e == Errno::LOOP { FsGuardError::Escapes.into() } else { io::Error::from(e).into() }
}

/// A directory `openat` failure, read the same way on every platform.
///
/// With `O_DIRECTORY | O_NOFOLLOW`, Linux says `ELOOP` for a symlink and macOS
/// says `ENOTDIR`, which a plain file there gives too. So `ENOTDIR` is looked
/// at again, without following, to tell the two apart.
fn directory_refusal(at: BorrowedFd<'_>, name: &OsStr, e: Errno) -> OpenError {
    if e == Errno::NOTDIR {
        if let Ok(stat) = rustix::fs::statat(at, name, AtFlags::SYMLINK_NOFOLLOW) {
            if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                return FsGuardError::Escapes.into();
            }
        }
    }
    leaf_refusal(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh worktree with a `src/` in it, its own for each test, so no
    /// test can see another's files.
    fn worktree() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("farcooler-guard-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).expect("temp worktree");
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
    fn any_dot_dot_is_refused_even_one_that_comes_back_inside() {
        // `src/../src/x` used to be allowed, by canonicalizing the existing
        // ancestor. The same rule allowed `/elsewhere/x/../../worktree/a`
        // exactly when `/elsewhere/x` exists, which told the agent whether it
        // did. Adapters send absolute paths, and none has been seen to send
        // `..`, so refusing it outright costs nothing.
        let wt = worktree();
        let err = confine(&wt, &wt.join("src/../src/new_file.rs")).unwrap_err();
        assert_eq!(err, FsGuardError::Escapes);
    }

    #[test]
    fn whether_a_directory_outside_exists_cannot_be_probed() {
        let wt = worktree();
        let real = outside("probe");
        fs::create_dir_all(real.join("x")).unwrap();
        let back = |dir: &Path| dir.join("x/../..").join(wt.file_name().unwrap()).join("src/a.rs");
        let present = confine(&wt, &back(&real));
        let absent = confine(&wt, &back(&outside("probe-missing")));
        assert_eq!(present, Err(FsGuardError::Escapes));
        assert_eq!(present, absent, "the answer does not depend on what exists outside");
        // Nor through a symlink outside that leads back in.
        relink(&wt, &real.join("into-wt"));
        let err = confine(&wt, &real.join("into-wt/src/a.rs")).unwrap_err();
        assert_eq!(err, FsGuardError::Escapes);
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

    fn open(wt: &Path) -> Worktree {
        Worktree::open(wt).expect("open the worktree")
    }

    fn refused(err: &OpenError) -> bool {
        matches!(err, OpenError::Refused(FsGuardError::Escapes))
    }

    fn kind(err: &OpenError) -> Option<io::ErrorKind> {
        match err {
            OpenError::Io(e) => Some(e.kind()),
            _ => None,
        }
    }

    #[test]
    fn a_symlink_swapped_in_for_a_new_name_after_the_check_is_not_followed() {
        // The race: `confine` judges a name that does not exist yet, and a
        // symlink appears there before the open. Following it is the escape.
        let wt = worktree();
        let away = outside("swap");
        fs::create_dir_all(&away).unwrap();
        let tree = open(&wt);
        let judged = tree.confine(Path::new("swapped")).expect("a new name inside");
        relink(&away.join("planted"), &wt.join("swapped"));
        let err = tree.write_judged(&judged, b"x").unwrap_err();
        assert!(refused(&err), "{err:?}");
        assert!(!away.join("planted").exists());
    }

    #[test]
    fn a_file_swapped_for_a_symlink_after_the_check_is_not_read() {
        let wt = worktree();
        let away = outside("swap-read");
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("secret"), "SECRET").unwrap();
        fs::write(wt.join("src/a.rs"), "inside").unwrap();
        let tree = open(&wt);
        let judged = tree.confine(Path::new("src/a.rs")).expect("inside");
        fs::remove_file(wt.join("src/a.rs")).unwrap();
        relink(&away.join("secret"), &wt.join("src/a.rs"));
        let err = tree.read_judged(&judged).unwrap_err();
        assert!(refused(&err), "{err:?}");
    }

    #[test]
    fn a_directory_swapped_for_a_symlink_after_the_check_is_refused_on_every_platform() {
        // Linux says `ELOOP` here and macOS `ENOTDIR`; both are a refusal.
        let wt = worktree();
        let away = outside("swap-dir");
        fs::create_dir_all(&away).unwrap();
        let tree = open(&wt);
        let judged = tree.confine(Path::new("swap-dir/file.rs")).expect("a new name inside");
        relink(&away, &wt.join("swap-dir"));
        let err = tree.write_judged(&judged, b"x").unwrap_err();
        assert!(refused(&err), "{err:?}");
        assert!(!away.join("file.rs").exists());
        let err = tree.read_judged(&judged).unwrap_err();
        assert!(refused(&err), "{err:?}");
    }

    #[test]
    fn a_name_under_a_regular_file_is_not_called_outside() {
        // It is inside, and simply cannot exist. Telling the agent it is
        // outside the worktree would be false.
        let wt = worktree();
        fs::write(wt.join("src/file"), "").unwrap();
        let err = open(&wt).write(Path::new("src/file/x.rs"), b"x").unwrap_err();
        assert_eq!(kind(&err), Some(io::ErrorKind::NotADirectory), "{err:?}");
        // Nor is one behind a directory that cannot be searched.
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir(wt.join("src/locked")).unwrap();
        fs::set_permissions(wt.join("src/locked"), fs::Permissions::from_mode(0o000)).unwrap();
        // Root searches it anyway, and then there is nothing to show.
        let enforced = fs::read_dir(wt.join("src/locked")).is_err();
        let err = open(&wt).write(Path::new("src/locked/x.rs"), b"x");
        fs::set_permissions(wt.join("src/locked"), fs::Permissions::from_mode(0o700)).unwrap();
        if enforced {
            let err = err.unwrap_err();
            assert_eq!(kind(&err), Some(io::ErrorKind::PermissionDenied), "{err:?}");
        }
        // But under a link to a regular file outside, it is outside.
        let away = outside("under-file");
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("file"), "").unwrap();
        relink(&away.join("file"), &wt.join("src/link"));
        let err = open(&wt).write(Path::new("src/link/x.rs"), b"x").unwrap_err();
        assert!(refused(&err), "{err:?}");
    }

    #[test]
    fn a_write_through_a_hard_link_leaves_the_file_outside_alone() {
        // pnpm's `node_modules` is hard links into a store every project
        // shares. Writing in place would change the file for all of them.
        let wt = worktree();
        let away = outside("hard");
        fs::create_dir_all(&away).unwrap();
        fs::write(away.join("shared.js"), "SHARED").unwrap();
        fs::hard_link(away.join("shared.js"), wt.join("src/shared.js")).unwrap();
        let written = open(&wt).write(Path::new("src/shared.js"), b"EDITED").expect("allowed");
        assert_eq!(written.before.as_deref(), Some(&b"SHARED"[..]));
        assert_eq!(fs::read_to_string(away.join("shared.js")).unwrap(), "SHARED");
        assert_eq!(fs::read_to_string(wt.join("src/shared.js")).unwrap(), "EDITED");
    }

    #[test]
    fn a_write_keeps_the_files_permissions_and_leaves_no_temporary_file() {
        use std::os::unix::fs::PermissionsExt;
        let wt = worktree();
        let script = wt.join("src/run.sh");
        fs::write(&script, "old").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).unwrap();
        open(&wt).write(Path::new("src/run.sh"), b"new").expect("allowed");
        assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o750);
        let names: Vec<_> = fs::read_dir(wt.join("src")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("run.sh")]);
    }

    #[test]
    fn a_write_creates_its_parent_directories_inside() {
        let wt = worktree();
        let tree = open(&wt);
        let written = tree.write(Path::new("made/deeper/new.rs"), b"x").expect("created inside");
        assert!(written.created);
        assert_eq!(written.path, wt.join("made/deeper/new.rs"));
        assert_eq!(fs::read_to_string(wt.join("made/deeper/new.rs")).unwrap(), "x");
        let again = tree.write(Path::new("made/deeper/new.rs"), b"y").unwrap();
        assert!(!again.created, "it exists now");
        assert_eq!(again.before.as_deref(), Some(&b"x"[..]));
    }

    #[test]
    fn a_failed_write_leaves_no_directories_behind() {
        // A name too long for the filesystem fails after the directories
        // above it were made, at the leaf and partway down. Either way they
        // are taken away again.
        let wt = worktree();
        let tree = open(&wt);
        let long = "n".repeat(300);
        for judged in [format!("new-a/new-b/{long}"), format!("new-a/new-b/{long}/leaf")] {
            let err = tree.write_judged(&wt.join(judged), b"x").unwrap_err();
            assert!(matches!(err, OpenError::Io(_)), "{err:?}");
            assert!(!wt.join("new-a").exists(), "the directories it made are gone");
        }
        // One that was already there stays.
        let err = tree.write_judged(&wt.join(format!("src/new-c/{long}")), b"x").unwrap_err();
        assert!(matches!(err, OpenError::Io(_)), "{err:?}");
        assert!(wt.join("src").is_dir() && !wt.join("src/new-c").exists());
    }

    #[test]
    fn a_read_of_a_missing_file_is_an_io_error_not_a_refusal() {
        let wt = worktree();
        let err = open(&wt).read(Path::new("src/absent.rs")).unwrap_err();
        assert_eq!(kind(&err), Some(io::ErrorKind::NotFound), "{err:?}");
    }

    #[test]
    fn a_read_past_the_limit_is_refused_as_too_large() {
        let wt = worktree();
        // Sparse and enormous: refused without reading a byte of it.
        let file = File::create(wt.join("src/huge")).unwrap();
        file.set_len(1 << 40).unwrap();
        let err = open(&wt).read(Path::new("src/huge")).unwrap_err();
        assert!(matches!(err, OpenError::TooLarge), "{err:?}");
        let file = File::create(wt.join("src/big")).unwrap();
        file.set_len(MAX_BYTES as u64 + 1).unwrap();
        let err = open(&wt).read(Path::new("src/big")).unwrap_err();
        assert!(matches!(err, OpenError::TooLarge), "{err:?}");
        let file = File::create(wt.join("src/limit")).unwrap();
        file.set_len(MAX_BYTES as u64).unwrap();
        assert_eq!(open(&wt).read(Path::new("src/limit")).unwrap().1.len(), MAX_BYTES);
    }

    #[test]
    fn a_fifo_is_refused_without_hanging() {
        let wt = worktree();
        let fifo = wt.join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
        assert!(made.success());
        let err = open(&wt).read(&fifo).unwrap_err();
        assert!(matches!(err, OpenError::NotAFile), "{err:?}");
        let err = open(&wt).write(&fifo, b"x").unwrap_err();
        assert!(matches!(err, OpenError::NotAFile), "{err:?}");
    }

    #[test]
    fn a_worktree_reached_through_a_symlink_is_written_where_it_really_is() {
        let wt = worktree();
        let link = outside("root-link");
        relink(&wt, &link);
        let tree = open(&link);
        tree.write(&link.join("src/a.rs"), b"x").expect("spelled the way it was given");
        tree.write(Path::new("src/b.rs"), b"y").expect("relative");
        assert_eq!(fs::read_to_string(wt.join("src/a.rs")).unwrap(), "x");
        assert_eq!(fs::read_to_string(wt.join("src/b.rs")).unwrap(), "y");
    }
}

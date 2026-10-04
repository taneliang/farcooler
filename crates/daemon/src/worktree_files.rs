//! A worktree's files, read-only: one directory's entries, or one file's text
//! (ov-189).
//!
//! What the Files tab reads. Every path is relative to the worktree's root
//! and walked from a descriptor held on the root, one component at a time,
//! with `O_NOFOLLOW` on each (`beneath::open_dir_beneath`), so:
//!
//! - **`..`, an absolute path and an empty component are refused** before
//!   anything is opened: only plain names are walked.
//! - **A symbolic link is never followed,** at any depth. One in a directory
//!   on the way is refused. One at the end is answered as a link, with its
//!   target's text and none of its contents: a link an agent planted pointing
//!   at `~/.ssh` shows as exactly that, and a client that wants to follow one
//!   asks again for the path it names, which walks the same way.
//! - **Only a regular file is read,** checked on the descriptor that is read
//!   from, and opened non-blocking, so a FIFO planted under a file's name
//!   can't hang the daemon.
//!
//! The threat this answers is a path an agent wrote (untrusted, possibly
//! prompt-injected) opening something outside its worktree in one tap. It is
//! not a wall against the device holder, who could already run `cat` in a
//! terminal; so it is `control`, the scope that can read a terminal screen,
//! and not `read` (`rpc::scope_of`).
//!
//! The same reads serve a runner's extra read-only folders (ov-232,
//! `read_only_folders`), asked for by name, with every check above and the
//! folder itself walked from `/` without following a link (`Base::Folder`).
//!
//! Nothing is hidden by name except `.git`, which is the repository's own
//! record and not a file of the work. A `.env` is shown like any other file:
//! the owner's ruling, 3 Oct, since the transport is ssh.

use std::ffi::OsStr;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use rustix::fs::{AtFlags, FileType, Mode, OFlags};

use crate::beneath::{open_dir_beneath, split};
use crate::service::Service;

/// The most of one file a client is sent: 512 KiB.
///
/// Half the control envelope (`MAX_CONTROL_ENVELOPE_BYTES`, 1 MiB), so the
/// text and its framing always fit one reply, and enough for every source
/// file anyone reads in a viewer. Past it the client says how big the file is
/// and offers an editor.
pub const MAX_READ_BYTES: u64 = 512 * 1024;

/// How far into a file a NUL byte makes it binary: git's own 8,000.
const BINARY_SNIFF: usize = 8000;

/// The most entries one directory's listing carries.
///
/// A `node_modules` or a build folder can hold tens of thousands; a person
/// scrolling a tree reads none of them past the first screens, and the reply
/// has to fit the envelope. The listing says when it stopped.
pub const MAX_ENTRIES: usize = 5000;

/// What a listed name is, read without following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Link,
    /// A FIFO, a socket, a device: listed so the tree is honest, never read.
    Other,
}

/// One name in a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// A file's size in bytes; zero for the rest.
    pub size: u64,
    /// A link's target, as written in the link. Empty for the rest.
    pub link_target: String,
}

/// A directory's entries: directories first, then the rest, each by name
/// without regard to case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// More than `MAX_ENTRIES` were there, and only the first were kept.
    pub truncated: bool,
}

/// One file, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The whole file, as UTF-8.
    Text { text: String, size: u64 },
    /// A NUL in its first 8,000 bytes, or not UTF-8.
    Binary { size: u64 },
    /// Over `MAX_READ_BYTES`; nothing of it is sent.
    TooLarge { size: u64 },
    /// The path's last component is a link: its target, and nothing read.
    Link { target: String },
}

/// Why a read was refused, before or while walking.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a plain relative path: `..`, absolute, or empty for a file.
    NotRelative,
    /// Nothing by that name, or a link on the way (which is refused, not
    /// followed, and reads to a client the same as a name that isn't there).
    NotFound,
    /// A directory where a file was asked for, or the reverse, or a FIFO,
    /// socket or device.
    WrongKind,
    /// Anything else the filesystem said.
    Io(std::io::ErrorKind),
}

/// `relative`, if it is only plain names. Empty is the root.
fn plain(relative: &str) -> std::result::Result<&Path, Refusal> {
    let path = Path::new(relative);
    // `Path::components` drops a `.` and collapses `//`, so the string is
    // checked too: a client that sends either has a bug worth hearing about.
    if relative.starts_with('/') || relative.split('/').any(|c| c == ".." || c == ".") {
        return Err(Refusal::NotRelative);
    }
    if path.components().all(|c| matches!(c, Component::Normal(_))) {
        Ok(path)
    } else {
        Err(Refusal::NotRelative)
    }
}

fn refusal(e: std::io::Error) -> Refusal {
    match e.raw_os_error() {
        // `O_NOFOLLOW` on a link: `ELOOP` on Linux and macOS both, and
        // `ENOTDIR` when a link to a file sits where a directory was asked.
        Some(code)
            if code == rustix::io::Errno::LOOP.raw_os_error()
                || code == rustix::io::Errno::NOENT.raw_os_error()
                || code == rustix::io::Errno::NOTDIR.raw_os_error() =>
        {
            Refusal::NotFound
        }
        _ if e.kind() == std::io::ErrorKind::InvalidInput => Refusal::NotRelative,
        _ => Refusal::Io(e.kind()),
    }
}

fn errno(e: rustix::io::Errno) -> Refusal {
    refusal(e.into())
}

/// Where a walk starts.
#[derive(Debug, Clone, Copy)]
pub enum Base<'a> {
    /// A worktree's root, opened the ordinary way (`beneath`'s reason: the
    /// daemon made it or was given it, and an agent in it can't rename it).
    Worktree(&'a Path),
    /// An extra read-only folder's real path (ov-232,
    /// `read_only_folders`), walked from `/` with `O_NOFOLLOW` at every
    /// component: a link swapped in at the folder or above it since the
    /// daemon started is refused, not followed.
    Folder(&'a Path),
}

/// The directory at `relative` beneath `base`, never through a link.
fn open_dir(base: Base, relative: &Path) -> std::result::Result<OwnedFd, Refusal> {
    match base {
        Base::Worktree(root) => open_dir_beneath(root, relative, false).map_err(refusal),
        Base::Folder(real) => {
            let below_root = real.strip_prefix("/").map_err(|_| Refusal::NotRelative)?;
            open_dir_beneath(Path::new("/"), &below_root.join(relative), false).map_err(refusal)
        }
    }
}

/// What `relative`, a directory in the worktree at `root`, holds.
pub fn list(root: &Path, relative: &str) -> std::result::Result<Listing, Refusal> {
    list_in(Base::Worktree(root), relative)
}

/// What `relative`, a directory beneath `base`, holds. A worktree's own
/// `.git` at its root is left out; a folder's is not, being nobody's record.
pub fn list_in(base: Base, relative: &str) -> std::result::Result<Listing, Refusal> {
    let dir = open_dir(base, plain(relative)?)?;
    let hide_git = matches!(base, Base::Worktree(_)) && relative.is_empty();
    let mut read = rustix::fs::Dir::read_from(&dir).map_err(errno)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    while let Some(next) = read.read() {
        let entry = next.map_err(errno)?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." || (hide_git && raw == b".git") {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            truncated = true;
            break;
        }
        let name = OsStr::from_bytes(raw);
        // Read again by name rather than trusting `d_type`, which some
        // filesystems leave unknown; and without following, so a link is
        // listed as one.
        let Ok(stat) = rustix::fs::statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
            // Gone between the listing and the look: not there.
            continue;
        };
        let kind = match FileType::from_raw_mode(stat.st_mode) {
            FileType::RegularFile => EntryKind::File,
            FileType::Directory => EntryKind::Directory,
            FileType::Symlink => EntryKind::Link,
            _ => EntryKind::Other,
        };
        let link_target = match kind {
            EntryKind::Link => rustix::fs::readlinkat(&dir, name, Vec::new())
                .map(|t| String::from_utf8_lossy(t.as_bytes()).into_owned())
                .unwrap_or_default(),
            _ => String::new(),
        };
        entries.push(Entry {
            name: String::from_utf8_lossy(raw).into_owned(),
            kind,
            size: if kind == EntryKind::File { stat.st_size.max(0) as u64 } else { 0 },
            link_target,
        });
    }
    entries.sort_by(|a, b| {
        let dir = |e: &Entry| e.kind != EntryKind::Directory;
        dir(a).cmp(&dir(b)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.name.cmp(&b.name))
    });
    Ok(Listing { entries, truncated })
}

/// The file at `relative` in the worktree at `root`.
pub fn read(root: &Path, relative: &str) -> std::result::Result<Content, Refusal> {
    read_in(Base::Worktree(root), relative)
}

/// The file at `relative` beneath `base`.
pub fn read_in(base: Base, relative: &str) -> std::result::Result<Content, Refusal> {
    let path = plain(relative)?;
    if relative.is_empty() {
        return Err(Refusal::NotRelative);
    }
    let (dirs, name) = split(path).map_err(refusal)?;
    let dir = open_dir(base, dirs)?;
    let stat = rustix::fs::statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Symlink => {
            let target = rustix::fs::readlinkat(&dir, name, Vec::new()).map_err(errno)?;
            return Ok(Content::Link { target: String::from_utf8_lossy(target.as_bytes()).into_owned() });
        }
        FileType::RegularFile => {}
        _ => return Err(Refusal::WrongKind),
    }
    // `O_NOFOLLOW`: a link swapped in since the look is refused, not read.
    // `O_NONBLOCK`: and a FIFO swapped in doesn't hang the open.
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    let fd = rustix::fs::openat(&dir, name, flags, Mode::empty()).map_err(errno)?;
    let held = rustix::fs::fstat(&fd).map_err(errno)?;
    if FileType::from_raw_mode(held.st_mode) != FileType::RegularFile {
        return Err(Refusal::WrongKind);
    }
    let size = held.st_size.max(0) as u64;
    if size > MAX_READ_BYTES {
        return Ok(Content::TooLarge { size });
    }
    let mut bytes = Vec::new();
    std::fs::File::from(fd)
        .take(MAX_READ_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Refusal::Io(e.kind()))?;
    // It grew between the stat and the read.
    if bytes.len() as u64 > MAX_READ_BYTES {
        return Ok(Content::TooLarge { size: bytes.len() as u64 });
    }
    let size = bytes.len() as u64;
    if bytes.iter().take(BINARY_SNIFF).any(|b| *b == 0) {
        return Ok(Content::Binary { size });
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Content::Text { text, size }),
        Err(_) => Ok(Content::Binary { size }),
    }
}

// ---------------------------------------------------------------------------
// The two RPCs
// ---------------------------------------------------------------------------

impl From<Refusal> for DomainError {
    fn from(r: Refusal) -> Self {
        match r {
            Refusal::NotRelative => DomainError::InvalidArgument { what: "path" },
            Refusal::NotFound => DomainError::NotFound,
            Refusal::WrongKind => DomainError::InvalidArgument { what: "kind" },
            Refusal::Io(_) => DomainError::OperationFailed,
        }
    }
}

/// Where a request's path is walked from, owned so it can cross to the
/// blocking pool.
enum Start {
    Worktree(std::path::PathBuf),
    Folder(std::path::PathBuf),
}

impl Start {
    fn base(&self) -> Base<'_> {
        match self {
            Start::Worktree(root) => Base::Worktree(root),
            Start::Folder(real) => Base::Folder(real),
        }
    }
}

/// The worktree's root on disk, by id; or, when `folder` is set, that
/// configured read-only folder, by its exact name (ov-232). Both at once is
/// refused, so a client can't mean one and be answered from the other.
fn start_of(svc: &Service, worktree_id: &[u8], folder: &str) -> Result<Start> {
    if !folder.is_empty() {
        if !worktree_id.is_empty() {
            return Err(DomainError::InvalidArgument { what: "folder" });
        }
        let found = crate::read_only_folders::find(svc.read_only_folders(), folder).ok_or(DomainError::NotFound)?;
        return Ok(Start::Folder(found.real().to_path_buf()));
    }
    let id = uuid::Uuid::from_slice(worktree_id).map_err(|_| DomainError::NotFound)?;
    Ok(Start::Worktree(std::path::PathBuf::from(svc.store.get_worktree(id)?.worktree_path)))
}

/// Off the runtime's threads: a directory on a slow disk or a network mount
/// must not stall every other client's call.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> std::result::Result<T, Refusal> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|_| DomainError::OperationFailed)?.map_err(DomainError::from)
}

/// `worktree.list_dir`.
pub async fn list_dir(svc: &Service, req: &pb::WorktreeDirRequest) -> Result<pb::WorktreeDir> {
    let start = start_of(svc, &req.worktree_id, &req.folder)?;
    let path = req.path.clone();
    let listing = blocking(move || list_in(start.base(), &path)).await?;
    Ok(pb::WorktreeDir {
        path: req.path.clone(),
        entries: listing
            .entries
            .into_iter()
            .map(|e| pb::WorktreeDirEntry {
                name: e.name,
                kind: (match e.kind {
                    EntryKind::File => pb::WorktreeEntryKind::File,
                    EntryKind::Directory => pb::WorktreeEntryKind::Directory,
                    EntryKind::Link => pb::WorktreeEntryKind::Link,
                    EntryKind::Other => pb::WorktreeEntryKind::Other,
                }) as i32,
                size: e.size,
                link_target: e.link_target,
            })
            .collect(),
        truncated: listing.truncated,
    })
}

/// `worktree.read_file`.
pub async fn read_file(svc: &Service, req: &pb::WorktreeFileRequest) -> Result<pb::WorktreeFile> {
    let start = start_of(svc, &req.worktree_id, &req.folder)?;
    let path = req.path.clone();
    let content = blocking(move || read_in(start.base(), &path)).await?;
    let mut file = pb::WorktreeFile { path: req.path.clone(), ..Default::default() };
    let state = match content {
        Content::Text { text, size } => {
            file.text = text;
            file.size = size;
            pb::WorktreeFileState::Text
        }
        Content::Binary { size } => {
            file.size = size;
            pb::WorktreeFileState::Binary
        }
        Content::TooLarge { size } => {
            file.size = size;
            pb::WorktreeFileState::TooLarge
        }
        Content::Link { target } => {
            file.link_target = target;
            pb::WorktreeFileState::Link
        }
    };
    file.state = state as i32;
    Ok(file)
}

#[cfg(test)]
mod tests;

//! A read-only inventory of every key in `authorized_keys`, inside the fence
//! and outside it (ov-510, design ov-480 §3).
//!
//! The fence is the part of the file Far Cooler writes. sshd obeys the whole
//! file, so a key added by hand above the block, or planted later by a user
//! with a shell, reaches the runner just as surely and is invisible to
//! `parse`. This lists all of them so the directory can say which belong to
//! whom.
//!
//! **Nothing here writes.** It does not take the lock, make a backup, create
//! `.ssh`, or touch the file's mode or times: it opens the directory without
//! `mkdir`, opens the file `O_RDONLY`, and reads. A damaged fence is an error
//! rather than a guess, as everywhere else in this crate.

use std::path::Path;

use sha2::Digest as _;

use super::{entry_from_line, fence_span, scope_word, FenceError, Markers};
use farcooler_protocol::v1::Scope;

/// One key in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// `SHA256:…`, as `ssh-keygen -lf` prints it.
    pub fingerprint: String,
    /// The key's comment. Whatever the line says; nothing authenticates it.
    pub comment: String,
    /// Whether the line sits between the fence markers.
    pub inside: bool,
    /// What the line grants through Far Cooler: `read`, `control`,
    /// `host_admin`, or `shell` for a Key B line of ours. `other` for any line
    /// Far Cooler did not write, which sshd treats as it likes.
    pub scope: &'static str,
}

/// Every key in the file, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub entries: Vec<Listed>,
    /// Lines that are not blank, not comments, and not a key this could read
    /// (a malformed line, an unsupported key type). Counted so a runner with
    /// one is not reported as clean; their text is never sent.
    pub unparsed: u32,
}

impl Inventory {
    /// A fingerprint of the whole inventory, order-independent: the same set
    /// of keys hashes the same however the file is arranged. This is what a
    /// heartbeat carries in place of the list.
    pub fn digest(&self) -> String {
        let mut lines: Vec<String> = self
            .entries
            .iter()
            .map(|e| format!("{}\t{}\t{}\t{}", e.fingerprint, u8::from(e.inside), e.scope, e.comment))
            .collect();
        lines.sort();
        let mut hash = sha2::Sha256::new();
        hash.update(format!("unparsed {}\n", self.unparsed));
        for line in &lines {
            hash.update(line.as_bytes());
            hash.update(b"\n");
        }
        hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// The inventory of a file's contents. Pure.
pub fn inventory_of(contents: &str, markers: Markers<'_>) -> Result<Inventory, FenceError> {
    let lines: Vec<&str> = contents.lines().collect();
    let span = fence_span(&lines, markers)?;
    let mut found = Inventory::default();
    for (at, line) in lines.iter().enumerate() {
        let text = line.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let inside = span.is_some_and(|(begin, end)| at > begin && at < end);
        let entry = entry_from_line(line);
        if entry.fingerprint.is_empty() {
            found.unparsed += 1;
            continue;
        }
        let scope = if entry.client_id.is_empty() {
            "other"
        } else if entry.shell_access {
            "shell"
        } else if entry.scope == Scope::Unspecified {
            "other"
        } else {
            scope_word(entry.scope)
        };
        found.entries.push(Listed { fingerprint: entry.fingerprint, comment: entry.label, inside, scope });
    }
    Ok(found)
}

/// The inventory of the file at `path`, without writing anything.
///
/// A file that isn't there, or a `.ssh` that isn't, is an empty inventory: the
/// ordinary state of a runner nobody has enrolled a device on. The same
/// ownership and mode checks as `read` apply, and refuse the same way.
pub fn inventory(path: &Path, markers: Markers<'_>) -> Result<Inventory, FenceError> {
    let name = path.file_name().ok_or(FenceError::Missing)?;
    let directory = match super::open_dir(path.parent().ok_or(FenceError::Missing)?, false) {
        Ok(directory) => directory,
        Err(FenceError::Missing) => return Ok(Inventory::default()),
        Err(e) => return Err(e),
    };
    match super::read_at(&directory, name)? {
        Some((text, _)) => inventory_of(&text, markers),
        None => Ok(Inventory::default()),
    }
}

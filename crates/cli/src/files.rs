//! `farcooler files`: a worktree's files, read-only (ov-189).
//!
//! What the Mac's Files tab reads, through this CLI as it reads everything
//! else, so an agent or a script can read the same way. Paths are relative to
//! the worktree's root; the runner refuses `..`, an absolute path and any
//! symbolic link on the way (`crates/daemon/src/worktree_files.rs`).

use clap::Subcommand;
use farcooler_client::files_json::{dir_json, file_json};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;

use crate::tasks::{DispatchLink, Refused};
use crate::{Fallible, connect_to, expect_value, req, with};

#[derive(Subcommand)]
pub enum FilesCmd {
    /// One directory's entries: directories first, then files.
    Ls {
        worktree: String,
        /// Relative to the worktree's root. The root when left out.
        #[arg(default_value = "")]
        path: String,
    },
    /// One file's text, whole, up to 512 KiB.
    Cat { worktree: String, path: String },
    /// One directory of an extra read-only folder, by the name `status
    /// --json` gives it under `readOnlyFolders` (ov-232).
    FolderLs {
        folder: String,
        /// Relative to the folder's root. The root when left out.
        #[arg(default_value = "")]
        path: String,
    },
    /// One file of an extra read-only folder, whole, up to 512 KiB.
    FolderCat { folder: String, path: String },
}

/// Which files a request reads: a worktree, by what the user typed for it, or
/// an extra folder, by its name.
enum Place {
    Worktree(String),
    Folder(String),
}

impl FilesCmd {
    /// Whether it reads an extra folder, which a runner needs
    /// `read_only_folders` for.
    fn reads_folder(&self) -> bool {
        matches!(self, FilesCmd::FolderLs { .. } | FilesCmd::FolderCat { .. })
    }
}

/// The two ids a request names its place by: `worktree_id` and `folder`.
/// Never both (the runner refuses that).
async fn place_ids<L: DispatchLink>(link: &mut L, place: Place) -> Result<(bytes::Bytes, String), Box<dyn std::error::Error>> {
    match place {
        Place::Worktree(w) => Ok((crate::id_bytes(crate::resolve_worktree_id(link, &w).await?), String::new())),
        Place::Folder(name) => Ok((bytes::Bytes::new(), name)),
    }
}

/// `worktree.list_dir`'s request for `place`.
pub(crate) fn dir_request(worktree_id: bytes::Bytes, folder: String, path: String) -> pb::WorktreeDirRequest {
    pb::WorktreeDirRequest { worktree_id, folder, path, ..Default::default() }
}

/// `worktree.read_file`'s request for `place`.
pub(crate) fn file_request(worktree_id: bytes::Bytes, folder: String, path: String) -> pb::WorktreeFileRequest {
    pb::WorktreeFileRequest { worktree_id, folder, path, ..Default::default() }
}

pub async fn files(runner: Option<&str>, cmd: FilesCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    files_over(&mut link, cmd, json).await
}

async fn files_over<L: DispatchLink>(link: &mut L, cmd: FilesCmd, json: bool) -> Fallible {
    if !link.capabilities().iter().any(|c| c == capability::WORKTREE_FILES) {
        return Err(Refused::new("this runner needs an update to show a worktree's files".into(), None).into());
    }
    if cmd.reads_folder() && !link.capabilities().iter().any(|c| c == capability::READ_ONLY_FOLDERS) {
        return Err(Refused::new("this runner needs an update to show its extra folders".into(), None).into());
    }
    answer(link, cmd, json).await.map_err(|e| match e.downcast::<ClientError>() {
        Ok(err) => Box::new(refusal(*err)) as Box<dyn std::error::Error>,
        Err(other) => other,
    })
}

async fn answer<L: DispatchLink>(link: &mut L, cmd: FilesCmd, json: bool) -> Fallible {
    match cmd {
        cmd @ (FilesCmd::Ls { .. } | FilesCmd::FolderLs { .. }) => {
            let (place, path) = match cmd {
                FilesCmd::Ls { worktree, path } => (Place::Worktree(worktree), path),
                FilesCmd::FolderLs { folder, path } => (Place::Folder(folder), path),
                _ => unreachable!(),
            };
            let (id, folder) = place_ids(link, place).await?;
            let payload = dir_request(id, folder, path);
            let r = link.call(with(req("worktree.list_dir"), request::Payload::WorktreeDir(payload))).await?;
            let result::Value::WorktreeDir(d) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                println!("{}", serde_json::to_string(&dir_json(&d))?);
                return Ok(());
            }
            for e in &d.entries {
                match pb::WorktreeEntryKind::try_from(e.kind) {
                    Ok(pb::WorktreeEntryKind::Directory) => println!("{}/", e.name),
                    Ok(pb::WorktreeEntryKind::Link) => println!("{} -> {}", e.name, e.link_target),
                    _ => println!("{}", e.name),
                }
            }
            if d.truncated {
                println!("(only the first {} entries)", d.entries.len());
            }
        }
        cmd @ (FilesCmd::Cat { .. } | FilesCmd::FolderCat { .. }) => {
            let (place, path) = match cmd {
                FilesCmd::Cat { worktree, path } => (Place::Worktree(worktree), path),
                FilesCmd::FolderCat { folder, path } => (Place::Folder(folder), path),
                _ => unreachable!(),
            };
            let (id, folder) = place_ids(link, place).await?;
            let payload = file_request(id, folder, path);
            let r = link.call(with(req("worktree.read_file"), request::Payload::WorktreeFile(payload))).await?;
            let result::Value::WorktreeFile(f) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                println!("{}", serde_json::to_string(&file_json(&f))?);
                return Ok(());
            }
            match pb::WorktreeFileState::try_from(f.state) {
                Ok(pb::WorktreeFileState::Text) => print!("{}", f.text),
                Ok(pb::WorktreeFileState::Binary) => eprintln!("a binary file, {} bytes", f.size),
                Ok(pb::WorktreeFileState::TooLarge) => eprintln!("too large to send, {} bytes", f.size),
                Ok(pb::WorktreeFileState::Link) => eprintln!("a link to {}", f.link_target),
                _ => eprintln!("this runner answered in a way this CLI doesn't know"),
            }
        }
    }
    Ok(())
}

/// The runner's refusal in this CLI's words, keeping its code.
fn refusal(err: ClientError) -> Refused {
    let (code, what) = match err {
        ClientError::Daemon { code, what, .. } => (code, what),
        other => return crate::tasks::refusal(other, ""),
    };
    let said = match (farcooler_core::error::word_for(code), what.as_str()) {
        ("not-found", _) => "nothing by that name in this worktree, or a link on the way to it",
        ("invalid-argument", "path") => "name a path inside the worktree, relative to its root, without `..`",
        ("invalid-argument", "kind") => "that's a directory, or not a file that can be read",
        ("scope-denied", _) => "this client isn't allowed to read files on this runner",
        _ => "the runner couldn't read that",
    };
    Refused::naming(said.into(), code, what)
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;

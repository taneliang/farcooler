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
}

pub async fn files(runner: Option<&str>, cmd: FilesCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    files_over(&mut link, cmd, json).await
}

async fn files_over<L: DispatchLink>(link: &mut L, cmd: FilesCmd, json: bool) -> Fallible {
    if !link.capabilities().iter().any(|c| c == capability::WORKTREE_FILES) {
        return Err(Refused::new("this runner needs an update to show a worktree's files".into(), None).into());
    }
    answer(link, cmd, json).await.map_err(|e| match e.downcast::<ClientError>() {
        Ok(err) => Box::new(refusal(*err)) as Box<dyn std::error::Error>,
        Err(other) => other,
    })
}

async fn answer<L: DispatchLink>(link: &mut L, cmd: FilesCmd, json: bool) -> Fallible {
    match cmd {
        FilesCmd::Ls { worktree, path } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
            let payload = pb::WorktreeDirRequest { worktree_id: crate::id_bytes(id), path };
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
        FilesCmd::Cat { worktree, path } => {
            let id = crate::resolve_worktree_id(link, &worktree).await?;
            let payload = pb::WorktreeFileRequest { worktree_id: crate::id_bytes(id), path };
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

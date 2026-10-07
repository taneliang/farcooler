//! A compose's images, uploaded before the compose that names them (ov-393).
//!
//! A request travels in one control envelope, capped at 1 MiB, so a Retina
//! screenshot can't ride inside `terminal.compose`. A runner with
//! `compose_upload` takes each image first through `terminal.paste_file`
//! with `stage`: chunked like any paste, kept here under its transfer id,
//! and nothing typed. The compose names it (`AgentPromptBlock.staged_image`)
//! and reads it once: every staged image a compose names is deleted as the
//! compose reads it, whatever comes of the send, since a client sends again
//! by uploading again. One nobody names is swept within the hour.
//!
//! What a compose types is its own copy in the paste directory, `compose-…`
//! (`Watcher::write_images`), whose path claude opens as it's pasted. That
//! copy goes at once when the send is refused before its path was pasted,
//! and within a day otherwise (`KEEP_COMPOSED`), not the week a paste keeps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{AgentPromptBlock, agent_prompt_block::Content};

use super::{Accepted, Stored, accept};

/// How long a staged image waits for the compose that names it: the client
/// composes as soon as its uploads finish, so anything older was abandoned.
pub const KEEP_STAGED: Duration = Duration::from_secs(60 * 60);

/// How long a composed image's copy survives its send. Swept hourly of
/// awake time (`super::SWEEP_EVERY`), so gone within a day while the runner
/// is awake; long enough for claude to have read it, which it does as its
/// path is pasted.
pub const KEEP_COMPOSED: Duration = Duration::from_secs(23 * 60 * 60);

/// What a composed image's copy is called before its id.
pub(crate) const COMPOSED_PREFIX: &str = "compose-";

/// Whether a file in the paste directory is a composed image's copy.
pub(crate) fn is_composed(name: &str) -> bool {
    name.starts_with(COMPOSED_PREFIX)
}

/// The longest transfer id taken: a v7 uuid is 16 bytes.
const LONGEST_ID: usize = 64;

/// Where staged images wait: a dotted subdirectory of the paste directory,
/// swept on its own clock.
pub fn staged_dir_in(root: &Path) -> Result<PathBuf> {
    let d = crate::paths::pastes_dir_in(root)?.join(".staged");
    std::fs::create_dir_all(&d).map_err(|_| DomainError::OperationFailed)?;
    Ok(d)
}

/// The staged file for `transfer_id`: its hex, so an id can never name a
/// separator, a dot-dot or a NUL.
fn staged_path(root: &Path, transfer_id: &[u8]) -> Result<PathBuf> {
    if transfer_id.is_empty() || transfer_id.len() > LONGEST_ID {
        return Err(DomainError::InvalidArgument { what: "transfer id" });
    }
    let hex: String = transfer_id.iter().map(|b| format!("{b:02x}")).collect();
    Ok(staged_dir_in(root)?.join(hex))
}

/// Accept one chunk of a staged image: `put_chunk`'s rules, and the finished
/// file kept under its transfer id rather than named and typed.
pub async fn put_chunk(root: &Path, transfer_id: &[u8], total_size: u64, offset: u64, chunk: &[u8]) -> Result<Stored> {
    let path = staged_path(root, transfer_id)?;
    match accept(&crate::paths::pastes_incoming_dir_in(root)?, transfer_id, total_size, offset, chunk).await? {
        Accepted::Partial { stored } => Ok(Stored::Partial { stored }),
        Accepted::Whole { partial, stored } => {
            tokio::fs::rename(&partial, &path).await.map_err(|e| {
                tracing::warn!(error = %e, "couldn't keep a staged image");
                DomainError::OperationFailed
            })?;
            Ok(Stored::Complete { path, stored })
        }
    }
}

/// The most images in one message.
pub(crate) const MOST_IMAGES: usize = 10;

/// A compose's images in the order its blocks name them, each a claimed
/// MIME type and bytes: those it carries, and those staged, read and
/// deleted. Refused as `images` past `MOST_IMAGES`, before anything is read;
/// as `images_too_large` past `MAX_COMPOSE_IMAGE_BYTES` carried or
/// `MAX_COMPOSE_UPLOAD_BYTES` in all, and `image_too_large` for one past a
/// paste's `MAX_PASTE_FILE_BYTES`, each counted as it's read, never from a
/// size read earlier; as `image` for a staged one that isn't here (used, or
/// swept). Every staged one named is deleted either way, an id that isn't
/// one included.
pub(crate) async fn images(root: &Path, blocks: &[AgentPromptBlock]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut used = Used(Vec::new());
    let mut staged = Vec::new();
    for block in blocks {
        if let Some(Content::StagedImage(id)) = &block.content {
            let path = staged_path(root, id);
            if let Ok(path) = &path {
                used.0.push(path.clone());
            }
            staged.push(path);
        }
    }
    let staged = staged.into_iter().collect::<Result<Vec<_>>>()?;
    let count = blocks.iter().filter(|b| matches!(b.content, Some(Content::Image(_) | Content::StagedImage(_)))).count();
    if count > MOST_IMAGES {
        return Err(DomainError::InvalidArgument { what: "images" });
    }
    let carried: usize = blocks
        .iter()
        .filter_map(|b| match &b.content {
            Some(Content::Image(i)) => Some(i.data.len()),
            _ => None,
        })
        .sum();
    if carried > farcooler_protocol::MAX_COMPOSE_IMAGE_BYTES {
        return Err(DomainError::Conflict { what: "images_too_large" });
    }
    let mut total = carried;
    let mut staged = staged.iter();
    let mut out = Vec::new();
    for block in blocks {
        match &block.content {
            Some(Content::Image(i)) => out.push((i.mime_type.clone(), i.data.to_vec())),
            Some(Content::StagedImage(_)) => {
                let path = staged.next().ok_or(DomainError::OperationFailed)?;
                let room = farcooler_protocol::MAX_COMPOSE_UPLOAD_BYTES - total;
                let bytes = read_capped(path, (farcooler_protocol::MAX_PASTE_FILE_BYTES as usize).min(room)).await?;
                total += bytes.len();
                out.push((String::new(), bytes));
            }
            _ => {}
        }
    }
    drop(used);
    Ok(out)
}

/// `path`'s bytes, at most `cap` of them: past it, `image_too_large` for a
/// file over a paste's limit, else `images_too_large` for the message's.
async fn read_capped(path: &Path, cap: usize) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await.map_err(|_| DomainError::InvalidArgument { what: "image" })?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes).await.map_err(|_| DomainError::InvalidArgument { what: "image" })?;
    if bytes.len() > cap {
        let one = bytes.len() as u64 > farcooler_protocol::MAX_PASTE_FILE_BYTES;
        return Err(DomainError::Conflict { what: if one { "image_too_large" } else { "images_too_large" } });
    }
    Ok(bytes)
}

/// Staged images a compose named, deleted when it's done with them.
struct Used(Vec<PathBuf>);

impl Drop for Used {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
#[path = "staged_tests.rs"]
mod tests;

//! `farcooler worktree hydrate-lfs`: try again to download the large files a
//! worktree still holds as pointers (ov-199).
//!
//! A worktree of a Git LFS repository hydrates its large files best effort
//! when it is made. An object missing from the runner's LFS store, or a
//! hydration that ran out of time, leaves the pointer file. This asks the
//! runner to try again, without touching the worktree's index or anything the
//! agent changed, and says how many are still pointers. The Mac's Try Again
//! runs it.

use farcooler_protocol::v1::{self as pb, result};

use crate::tasks::{DispatchLink, Refused, refused};
use crate::{Fallible, expect_value, find_worktree, list_worktrees, req_for, short_bytes, uuid_of};

/// What a runner without `lfs_pointers` is told.
const NO_LFS: &str = "this runner's Far Cooler is older than large-file retries. update it and try again";

/// Ask the runner to try again for `worktree`, and print what's left.
pub(crate) async fn hydrate<L: DispatchLink>(link: &mut L, worktree: &str, json: bool) -> Fallible {
    let all = list_worktrees(link).await?;
    let ws = find_worktree(&all, worktree)?;
    let left = hydrate_lfs(link, uuid_of(&ws.id)).await?;
    if json {
        println!("{}", serde_json::json!({ "lfs_pointers": left }));
    } else if left == 0 {
        println!("{}  every large file is downloaded", short_bytes(&ws.id));
    } else {
        println!("{}  {left} large files still aren't downloaded", short_bytes(&ws.id));
    }
    Ok(())
}

/// `worktree.hydrate_lfs`, answering the count of pointers left.
pub(crate) async fn hydrate_lfs<L: DispatchLink>(
    link: &mut L,
    worktree: uuid::Uuid,
) -> Result<u32, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == farcooler_protocol::capability::LFS_POINTERS) {
        return Err(Box::new(Refused::new(NO_LFS.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))));
    }
    let mut r = req_for("worktree.hydrate_lfs", worktree);
    r.required_capabilities.push(farcooler_protocol::capability::LFS_POINTERS.to_string());
    let answer = link.call(r).await.map_err(|e| refused(e, "the runner couldn't try again. try again"))?;
    match expect_value(answer.value)? {
        result::Value::Worktree(w) => Ok(w.lfs_pointers),
        _ => Err(crate::daemon_link::UNREADABLE.into()),
    }
}

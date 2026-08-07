//! The five worktree-row actions, shared between `crates/client` (which
//! iOS's FFI bridge calls through `Session`) and `crates/cli` (which macOS's
//! `DaemonClient.swift` shells out to). Both wrap the same
//! `farcooler_transport::Client` around a connection each side establishes
//! its own way — a system `ssh` subprocess and local-daemon auto-start for
//! the CLI, an in-process `russh` client with no auto-start for the mobile
//! core — so these functions are generic over the transport rather than over
//! either connection type, and never try to unify how the connection itself
//! was made.

use farcooler_protocol::v1::{
    Repository, RepositoryRegister, RepositoryRoot, RepositoryRootAdd, request, result,
};
use farcooler_transport::{Client, ClientError};
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

/// What asking the daemon to remove a worktree came back with.
///
/// Not folded into `ClientError`: a confirmation prompt is an expected
/// domain outcome a caller branches on, not a transport failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveWorktreeOutcome {
    Removed,
    ConfirmationRequired,
}

async fn call<R, W>(
    client: &mut Client<R, W>,
    method: &str,
    target: Uuid,
    payload: Option<request::Payload>,
) -> Result<Option<result::Value>, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request(method);
    request.target_resource_id = Some(bytes::Bytes::copy_from_slice(target.as_bytes()));
    if let Some(p) = payload {
        request.payload = Some(p);
    }
    Ok(client.call(request).await?.value)
}

pub async fn hide_workspace<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
) -> Result<(), ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    call(client, "workspace.hide", workspace, None).await.map(|_| ())
}

pub async fn unhide_workspace<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
) -> Result<(), ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    call(client, "workspace.unhide", workspace, None).await.map(|_| ())
}

/// Remove a worktree. `confirm` must be the workspace's exact task name,
/// unless the worktree is clean, in which case it may be empty.
///
/// Forwarded rather than checked here: the daemon refuses a mismatch itself,
/// so this is a courtesy and the daemon's own check is what actually
/// protects the files.
pub async fn remove_worktree<R, W>(
    client: &mut Client<R, W>,
    workspace: Uuid,
    confirm: &str,
) -> Result<RemoveWorktreeOutcome, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let payload = request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
        typed_confirmation: confirm.to_string(),
    });
    match call(client, "workspace.remove_worktree", workspace, Some(payload)).await {
        Ok(_) => Ok(RemoveWorktreeOutcome::Removed),
        Err(ClientError::Daemon { code, .. })
            if code == farcooler_protocol::v1::ErrorCode::ConfirmationRequired as i32 =>
        {
            Ok(RemoveWorktreeOutcome::ConfirmationRequired)
        }
        Err(other) => Err(other),
    }
}

/// Send an image into a terminal, and return the path it landed at on the host.
///
/// Here rather than in `Session` for the reason this module exists: the CLI and
/// the mobile core reach the daemon through different connections but must
/// paste identically. The chunking, the ceiling and the refusal to resume are
/// the parts that must not differ.
///
/// `progress` is called after each chunk with (sent, total) — enough to draw a
/// ring, and nothing here depends on what it does with it.
pub async fn paste_image<R, W>(
    client: &mut Client<R, W>,
    terminal: Uuid,
    mime: &str,
    image: &[u8],
    mut progress: impl FnMut(u64, u64),
) -> Result<String, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let total = image.len() as u64;
    if total == 0 || total > farcooler_protocol::MAX_IMAGE_PASTE_BYTES {
        return Err(ClientError::WrongResult {
            expected: "an image within the size limit",
            got: "one that is empty or too large",
        });
    }

    // One id for the whole transfer, so a second paste into the same pane
    // cannot append to this one's bytes. v7 like every other id here: the
    // daemon only hexes it into a filename, so the timestamp prefix costs
    // nothing and keeps partials sorted by when they started.
    let transfer_id = farcooler_protocol::ids::new_id();
    let mut offset: u64 = 0;

    for chunk in image.chunks(farcooler_protocol::IMAGE_PASTE_CHUNK_BYTES) {
        let payload =
            request::Payload::TerminalImagePut(farcooler_protocol::v1::TerminalImagePut {
                terminal_id: bytes::Bytes::copy_from_slice(terminal.as_bytes()),
                transfer_id: transfer_id.clone(),
                mime: mime.to_string(),
                total_size: total,
                offset,
                chunk: bytes::Bytes::copy_from_slice(chunk),
            });
        match call(client, "terminal.paste_image", terminal, Some(payload)).await? {
            Some(result::Value::TerminalImagePut(r)) => {
                offset = r.stored;
                progress(r.stored, total);
                if let Some(path) = r.path {
                    return Ok(path);
                }
            }
            _ => {
                return Err(ClientError::WrongResult {
                    expected: "terminal_image_put",
                    got: "something else",
                });
            }
        }
    }

    // Every chunk was accepted and the daemon never said it was finished, so
    // the two sides disagree about how big the image is.
    Err(ClientError::WrongResult {
        expected: "a finished image",
        got: "a transfer that never completed",
    })
}

/// Allowlist a folder Far Cooler may operate under. Returns the new root.
pub async fn add_repository_root<R, W>(
    client: &mut Client<R, W>,
    absolute_path: &str,
) -> Result<RepositoryRoot, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request("repository_root.add");
    request.payload = Some(request::Payload::RepositoryRootAdd(RepositoryRootAdd {
        absolute_path: absolute_path.to_string(),
        typed_confirmation: String::new(),
    }));
    match client.call(request).await?.value {
        Some(result::Value::RepositoryRoot(root)) => Ok(root),
        _ => Err(ClientError::WrongResult { expected: "repository_root", got: "something else" }),
    }
}

/// Register an existing repository inside an already-allowlisted root.
/// `relative_path` is the repository's absolute path — the daemon resolves
/// it against whichever registered root covers it, same as the CLI's own
/// `repo register` has always done.
pub async fn register_repository<R, W>(
    client: &mut Client<R, W>,
    relative_path: &str,
) -> Result<Repository, ClientError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut request = farcooler_transport::request("repository.register");
    request.payload = Some(request::Payload::RepositoryRegister(RepositoryRegister {
        relative_path: relative_path.to_string(),
    }));
    match client.call(request).await?.value {
        Some(result::Value::Repository(repo)) => Ok(repo),
        _ => Err(ClientError::WrongResult { expected: "repository", got: "something else" }),
    }
}

//! Rule 1: the daemon opens no network listener. This is the only
//! network-adjacent entry point it has, and it is a filesystem-permissioned
//! Unix socket, never a TCP port.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::net::UnixListener;

use crate::Handler;
use crate::connection::{Connection, HandshakeConfig, serve_connection};

pub struct UnixListenerServer {
    listener: UnixListener,
    path: PathBuf,
}

impl UnixListenerServer {
    /// Rule 5: binds `path` with mode 0600 under a user-only (0700) parent
    /// directory. Bind-then-chmod leaves a brief window at default
    /// permissions; the 0700 parent covers it, since nothing but the owner
    /// can even traverse into the directory during that window.
    pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        // A stale socket left by a crashed daemon must not block a fresh bind.
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, path })
    }

    pub fn local_path(&self) -> &Path {
        &self.path
    }

    /// Accepts connections until the listener itself errors. Each connection
    /// gets its own handshake and its own rule-4 accounting, so one slow
    /// client cannot stall another.
    pub async fn serve<H>(&self, cfg: HandshakeConfig, handler: H) -> std::io::Result<()>
    where
        H: Handler + Clone,
    {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let handler = handler.clone();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                let (read_half, write_half) = stream.into_split();
                let mut conn = Connection::new(read_half, write_half);
                if let Err(err) = serve_connection(&mut conn, &cfg, &handler).await {
                    tracing::debug!(error = %err, "connection closed");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn socket_and_parent_directory_are_owner_only() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("nested").join("overnight.sock");
        let server = UnixListenerServer::bind(&sock_path).unwrap();

        let meta = std::fs::metadata(server.local_path()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600, "socket must be mode 0600");

        let parent_meta = std::fs::metadata(sock_path.parent().unwrap()).unwrap();
        assert_eq!(parent_meta.permissions().mode() & 0o777, 0o700, "parent dir must be user-only");
    }

    #[tokio::test]
    async fn rebinding_removes_a_stale_socket() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("overnight.sock");
        let first = UnixListenerServer::bind(&sock_path).unwrap();
        // Simulate a crash: drop the listener but leave the file behind.
        drop(first);
        assert!(sock_path.exists(), "the socket file itself outlives the listener");

        let second = UnixListenerServer::bind(&sock_path);
        assert!(second.is_ok(), "a stale socket file must not block a fresh bind");
    }
}

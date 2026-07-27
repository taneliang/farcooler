//! Private tmux server lifecycle.
//!
//! Overnight never mixes managed windows into the user's default tmux server and
//! never depends on the user's tmux configuration. It runs its own server on a
//! dedicated socket with minimal config:
//!
//! ```text
//! tmux -L overnight-<install-id> -f /dev/null
//! ```
//!
//! A workspace is a daemon grouping of TAGGED WINDOWS, not a tmux session. There
//! is one host-wide session.

use std::process::Stdio;

use overnight_core::{DomainError, Result, SCHEMA_VERSION, tags};
use tokio::process::Command;
use uuid::Uuid;

/// Display name of the single host-wide session. Addressed internally by its
/// stable tmux session id, never by this name.
pub const SESSION_NAME: &str = "overnight";

#[derive(Debug, Clone)]
pub struct TmuxServer {
    socket: String,
    daemon_id: Uuid,
}

/// Raw result of one tmux invocation.
#[derive(Debug)]
pub struct Output {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

impl TmuxServer {
    pub fn new(install_id: &str, daemon_id: Uuid) -> Self {
        Self { socket: format!("overnight-{install_id}"), daemon_id }
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    pub fn daemon_id(&self) -> Uuid {
        self.daemon_id
    }

    /// The raw recovery command shown to users for transparency. It reaches the
    /// same live session and is documented as bypassing writer-lease enforcement.
    pub fn recovery_command(&self) -> String {
        format!("tmux -L {} attach -t {}", self.socket, SESSION_NAME)
    }

    /// Run a tmux command against the private server.
    pub async fn run(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = Command::new("tmux");
        cmd.arg("-L").arg(&self.socket).arg("-f").arg("/dev/null");
        cmd.args(args);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let out = cmd.output().await.map_err(|e| {
            tracing::warn!(error = %e, "failed to spawn tmux");
            DomainError::TmuxUnavailable
        })?;

        Ok(Output {
            status: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// True when the private server is currently running.
    pub async fn is_running(&self) -> bool {
        self.run(&["has-session", "-t", SESSION_NAME]).await.map(|o| o.ok()).unwrap_or(false)
    }

    /// Ensure the host-wide session exists.
    ///
    /// The session contains managed terminal windows only and keeps no fake
    /// sentinel shell. The first terminal creates it; removing the last may let
    /// it exit, after which the daemon recreates it on demand.
    pub async fn ensure_session(&self) -> Result<()> {
        if self.is_running().await {
            return Ok(());
        }

        // `new-session -d` with a placeholder window; the caller renames and tags
        // the first real terminal into it. `-x/-y` give a sane detached size.
        let out = self
            .run(&[
                "new-session",
                "-d",
                "-s",
                SESSION_NAME,
                "-x",
                "120",
                "-y",
                "40",
                "-n",
                "overnight-bootstrap",
            ])
            .await?;

        if !out.ok() && !out.stderr.contains("duplicate session") {
            tracing::warn!(stderr = %out.stderr, "failed to create host session");
            return Err(DomainError::TmuxUnavailable);
        }

        // Tag the session so a foreign server can never be mistaken for ours.
        self.set_session_option(tags::DAEMON_ID, &self.daemon_id.to_string()).await?;
        self.set_session_option(tags::SCHEMA_VERSION, &SCHEMA_VERSION.to_string()).await?;

        // Size a window to its most recently active client, which is the same
        // rule the protocol uses for the size controller.
        let _ = self.run(&["set-option", "-t", SESSION_NAME, "window-size", "latest"]).await;
        Ok(())
    }

    async fn set_session_option(&self, key: &str, value: &str) -> Result<()> {
        let out = self.run(&["set-option", "-t", SESSION_NAME, key, value]).await?;
        if !out.ok() {
            tracing::warn!(key, stderr = %out.stderr, "failed to set session tag");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Kill the private server entirely. Test and uninstall use only; ordinary
    /// workspace removal never uses `kill-session`.
    pub async fn kill_server(&self) -> Result<()> {
        let _ = self.run(&["kill-server"]).await;
        Ok(())
    }
}

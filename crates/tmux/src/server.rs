//! Private tmux server lifecycle.
//!
//! Overnight never mixes managed windows into the user's default tmux server and
//! never depends on the user's tmux configuration. It runs its own server on a
//! dedicated socket with minimal config:
//!
//! ```text
//! tmux -L overnight-<install-id> -f <overnight-managed.conf>
//! ```
//!
//! A workspace is a daemon grouping of TAGGED WINDOWS, not a tmux session. There
//! is one host-wide session.

use std::path::PathBuf;
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
    config_path: PathBuf,
}

/// Overnight's own minimal tmux configuration.
///
/// This is NOT the user's config: the server starts with `-f` pointing here, so
/// nothing in `~/.tmux.conf` can change managed behaviour, and these two options
/// are in force from the very first window rather than being applied afterwards.
///
/// `remain-on-exit` is the load-bearing one. Applied post-hoc it would race a
/// command that exits immediately, and that terminal would derive `lost` when it
/// actually exited cleanly.
const MANAGED_CONFIG: &str = "\
set -g remain-on-exit on
set -g window-size latest
set -g status off
";

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
        let config_path = std::env::temp_dir().join(format!("overnight-{install_id}.tmux.conf"));
        Self::with_config(install_id, daemon_id, config_path)
    }

    pub fn with_config(install_id: &str, daemon_id: Uuid, config_path: PathBuf) -> Self {
        Self { socket: format!("overnight-{install_id}"), daemon_id, config_path }
    }

    /// Write the managed config if it is not already in place.
    fn ensure_config(&self) -> Result<()> {
        if self.config_path.exists() {
            return Ok(());
        }
        std::fs::write(&self.config_path, MANAGED_CONFIG).map_err(|e| {
            tracing::warn!(error = %e, "could not write managed tmux config");
            DomainError::TmuxUnavailable
        })
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
        format!(
            "tmux -L {} -f {} attach -t {}",
            self.socket,
            self.config_path.display(),
            SESSION_NAME
        )
    }

    /// Run a tmux command against the private server.
    pub async fn run(&self, args: &[&str]) -> Result<Output> {
        self.ensure_config()?;

        let mut cmd = Command::new("tmux");
        cmd.arg("-L").arg(&self.socket).arg("-f").arg(&self.config_path);
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

    /// Tag the session and set its size policy once it exists.
    ///
    /// The session contains managed terminal windows only and keeps NO fake
    /// sentinel shell. Creating a placeholder window would squat the session's
    /// base index and make the first real terminal fail with "index 0 in use",
    /// so the first terminal creates the session instead. See
    /// `create_terminal_window`.
    pub(crate) async fn tag_session(&self) -> Result<()> {
        self.set_session_option(tags::DAEMON_ID, &self.daemon_id.to_string()).await?;
        self.set_session_option(tags::SCHEMA_VERSION, &SCHEMA_VERSION.to_string()).await?;

        // `remain-on-exit` and `window-size latest` come from MANAGED_CONFIG so
        // they are in force from the first window, not applied afterwards.
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

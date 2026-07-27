//! Managed window and pane operations.
//!
//! Every command targets stable session, window, or pane IDs rather than names
//! or indexes. Names, indexes, and PID values are display or diagnostic data
//! only and never establish identity.

use overnight_core::{DomainError, Result, SCHEMA_VERSION, inventory::TaggedPane, tags};
use uuid::Uuid;

use crate::server::{SESSION_NAME, TmuxServer};

/// A window created for one terminal, addressed by its stable tmux ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWindow {
    pub window_id: String,
    pub pane_id: String,
}

impl TmuxServer {
    /// Create a tagged window running `command` with its working directory set
    /// to the workspace worktree.
    ///
    /// The working directory is passed as tmux's validated `-c` argument rather
    /// than as `cd` text, so no path is ever interpolated into a shell string.
    pub async fn create_terminal_window(
        &self,
        workspace_id: Uuid,
        terminal_id: Uuid,
        title: &str,
        worktree: &str,
        command: &str,
    ) -> Result<ManagedWindow> {
        self.ensure_session().await?;

        let out = self
            .run(&[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{window_id} #{pane_id}",
                "-t",
                SESSION_NAME,
                "-n",
                title,
                "-c",
                worktree,
                command,
            ])
            .await?;

        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "new-window failed");
            return Err(DomainError::TmuxUnavailable);
        }

        let line = out.stdout.trim();
        let mut parts = line.split_whitespace();
        let (Some(window_id), Some(pane_id)) = (parts.next(), parts.next()) else {
            tracing::warn!(line, "unparsable new-window output");
            return Err(DomainError::TmuxUnavailable);
        };

        let win = ManagedWindow {
            window_id: window_id.to_string(),
            pane_id: pane_id.to_string(),
        };

        // Tag the window. Identity lives here, never in the name.
        for (k, v) in [
            (tags::DAEMON_ID, self.daemon_id().to_string()),
            (tags::WORKSPACE_ID, workspace_id.to_string()),
            (tags::TERMINAL_ID, terminal_id.to_string()),
            (tags::SCHEMA_VERSION, SCHEMA_VERSION.to_string()),
        ] {
            let out = self.run(&["set-option", "-w", "-t", &win.window_id, k, &v]).await?;
            if !out.ok() {
                tracing::warn!(tag = k, stderr = %out.stderr, "failed to tag window");
                return Err(DomainError::TmuxUnavailable);
            }
        }

        // Drop the bootstrap window once a real terminal exists.
        let _ = self.run(&["kill-window", "-t", &format!("{SESSION_NAME}:overnight-bootstrap")]).await;

        Ok(win)
    }

    /// Fresh inventory of every live pane carrying our exact tags.
    ///
    /// One bulk query, never one round trip per terminal, because derivation
    /// sits on the fleet-render path.
    pub async fn list_tagged_panes(&self) -> Result<Vec<TaggedPane>> {
        let fmt = format!(
            "#{{pane_id}}\t#{{window_id}}\t#{{pane_width}}\t#{{pane_height}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{{}}}",
            tags::DAEMON_ID,
            tags::WORKSPACE_ID,
            tags::TERMINAL_ID,
            tags::SCHEMA_VERSION
        );

        let out = self.run(&["list-panes", "-a", "-F", &fmt]).await?;
        if !out.ok() {
            // No server or no session is not an error: it means nothing is alive.
            if out.stderr.contains("no server running")
                || out.stderr.contains("no current session")
                || out.stderr.contains("error connecting")
            {
                return Ok(Vec::new());
            }
            tracing::warn!(stderr = %out.stderr, "list-panes failed");
            return Err(DomainError::TmuxUnavailable);
        }

        Ok(out.stdout.lines().filter_map(parse_pane_line).collect())
    }

    /// Kill exactly the window whose fresh tags match this terminal.
    ///
    /// Never `kill-session`, and never a name or index match.
    pub async fn kill_terminal_window(&self, terminal_id: Uuid) -> Result<bool> {
        let panes = self.list_tagged_panes().await?;
        let Some(p) = panes
            .iter()
            .find(|p| p.terminal_id == terminal_id && p.daemon_id == self.daemon_id())
        else {
            return Ok(false);
        };
        let out = self.run(&["kill-window", "-t", &p.window_id]).await?;
        Ok(out.ok())
    }

    /// Send exact bytes to a pane.
    pub async fn send_keys(&self, pane_id: &str, data: &str) -> Result<()> {
        let out = self.run(&["send-keys", "-t", pane_id, "-l", data]).await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "send-keys failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Resize the exact window backing a terminal.
    pub async fn resize_window(&self, window_id: &str, columns: u32, rows: u32) -> Result<()> {
        let out = self
            .run(&[
                "resize-window",
                "-t",
                window_id,
                "-x",
                &columns.to_string(),
                "-y",
                &rows.to_string(),
            ])
            .await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "resize-window failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Retained pane contents, used to resynchronize after a gap.
    pub async fn capture_pane(&self, pane_id: &str, lines: u32) -> Result<String> {
        let start = format!("-{lines}");
        let out = self.run(&["capture-pane", "-p", "-J", "-S", &start, "-t", pane_id]).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(out.stdout)
    }
}

/// Parse one `list-panes -F` line. A line missing our tags is not ours.
pub(crate) fn parse_pane_line(line: &str) -> Option<TaggedPane> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 8 {
        return None;
    }

    let daemon_id = Uuid::parse_str(f[4].trim()).ok()?;
    let workspace_id = Uuid::parse_str(f[5].trim()).ok()?;
    let terminal_id = Uuid::parse_str(f[6].trim()).ok()?;
    let schema_version: u32 = f[7].trim().parse().ok()?;

    Some(TaggedPane {
        daemon_id,
        workspace_id,
        terminal_id,
        schema_version,
        pane_id: f[0].trim().to_string(),
        window_id: f[1].trim().to_string(),
        columns: f[2].trim().parse().unwrap_or(0),
        rows: f[3].trim().parse().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(daemon: &str, ws: &str, term: &str) -> String {
        format!("%3\t@2\t120\t40\t{daemon}\t{ws}\t{term}\t1")
    }

    #[test]
    fn parses_a_fully_tagged_pane() {
        let d = Uuid::from_u128(1);
        let w = Uuid::from_u128(2);
        let t = Uuid::from_u128(3);
        let p = parse_pane_line(&line(&d.to_string(), &w.to_string(), &t.to_string())).unwrap();
        assert_eq!(p.daemon_id, d);
        assert_eq!(p.workspace_id, w);
        assert_eq!(p.terminal_id, t);
        assert_eq!(p.pane_id, "%3");
        assert_eq!(p.window_id, "@2");
        assert_eq!(p.columns, 120);
        assert_eq!(p.rows, 40);
    }

    #[test]
    fn untagged_pane_is_ignored_completely() {
        // A user's own pane on some other server has empty tag fields.
        assert!(parse_pane_line("%1\t@1\t80\t24\t\t\t\t").is_none());
    }

    #[test]
    fn partially_tagged_pane_is_not_identity() {
        let d = Uuid::from_u128(1).to_string();
        assert!(parse_pane_line(&format!("%1\t@1\t80\t24\t{d}\t\t\t1")).is_none());
    }

    #[test]
    fn truncated_line_is_ignored() {
        assert!(parse_pane_line("%1\t@1\t80").is_none());
    }

    #[test]
    fn non_uuid_tag_is_ignored() {
        assert!(parse_pane_line("%1\t@1\t80\t24\tnot-a-uuid\tx\ty\t1").is_none());
    }
}

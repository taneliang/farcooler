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
        // The first terminal creates the session. There is no sentinel window,
        // so nothing squats the base index.
        let session_exists = self.is_running().await;
        let target = format!("{SESSION_NAME}:");

        let out = if session_exists {
            self.run(&[
                "new-window",
                "-d",
                "-a", // next free index, never reuse a live one
                "-P",
                "-F",
                "#{window_id} #{pane_id}",
                "-t",
                &target,
                "-n",
                title,
                "-c",
                worktree,
                command,
            ])
            .await?
        } else {
            self.run(&[
                "new-session",
                "-d",
                "-s",
                SESSION_NAME,
                "-x",
                "120",
                "-y",
                "40",
                "-P",
                "-F",
                "#{window_id} #{pane_id}",
                "-n",
                title,
                "-c",
                worktree,
                command,
            ])
            .await?
        };

        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "failed to create managed window");
            return Err(DomainError::TmuxUnavailable);
        }

        if !session_exists {
            self.tag_session().await?;
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

        Ok(win)
    }

    /// Fresh inventory of every live pane carrying our exact tags.
    ///
    /// One bulk query, never one round trip per terminal, because derivation
    /// sits on the fleet-render path.
    pub async fn list_tagged_panes(&self) -> Result<Vec<TaggedPane>> {
        let fmt = format!(
            "#{{pane_id}}\t#{{window_id}}\t#{{pane_width}}\t#{{pane_height}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{pane_dead}}\t#{{pane_dead_status}}",
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

    /// Send exact input BYTES, given as hex.
    ///
    /// This is the real input path for a terminal client. The client computes
    /// the VT encoding for whatever the user pressed, arrows and control chords
    /// included, and those exact bytes reach the PTY. Nothing is interpreted as
    /// a tmux key name along the way, so a literal `Up` typed by a user is text
    /// and an actual arrow key is `1b5b41`.
    pub async fn send_bytes_hex(&self, pane_id: &str, hex: &str) -> Result<()> {
        if hex.is_empty() {
            return Ok(());
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) || hex.len() % 2 != 0 {
            return Err(DomainError::InvalidArgument { what: "hex payload" });
        }

        // tmux -H takes space-separated byte values.
        let bytes: Vec<String> =
            hex.as_bytes().chunks(2).map(|p| String::from_utf8_lossy(p).into_owned()).collect();

        let mut args: Vec<&str> = vec!["send-keys", "-t", pane_id, "-H"];
        args.extend(bytes.iter().map(|s| s.as_str()));

        let out = self.run(&args).await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "send-keys -H failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// The rendered visible screen, with SGR escape sequences preserved.
    ///
    /// tmux is already the terminal emulator: it has parsed the program's
    /// output and maintains the screen. Capturing the rendered result is why
    /// a client can open onto a running full-screen TUI rather than a blank
    /// screen it would have to wait for the program to repaint.
    pub async fn capture_screen(&self, pane_id: &str) -> Result<String> {
        let out = self.run(&["capture-pane", "-e", "-p", "-t", pane_id]).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(out.stdout)
    }

    /// Where the cursor is in the pane, zero-based as (column, row).
    ///
    /// A captured screen is text: it carries no cursor. Without asking tmux
    /// separately, a client that replays a capture leaves its caret wherever
    /// the last replayed character happened to end — which is the bottom of the
    /// screen, not where the user is typing.
    pub async fn cursor_position(&self, pane_id: &str) -> Result<(u32, u32)> {
        let out = self
            .run(&["display-message", "-p", "-t", pane_id, "#{cursor_x}\t#{cursor_y}"])
            .await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        parse_cursor(&out.stdout).ok_or(DomainError::TmuxUnavailable)
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

    /// Start streaming a pane's raw output into `command`'s stdin.
    ///
    /// This is the real terminal data plane. `pipe-pane` hands over the exact
    /// bytes the program wrote, escape sequences and all, which is what a VT
    /// emulator needs. Polling a rendered snapshot can never show a cursor
    /// moving or an animation, because it only ever sees the settled screen.
    ///
    /// `-O` is output only: nothing the user types is echoed back into the pipe.
    pub async fn pipe_pane_start(&self, pane_id: &str, command: &str) -> Result<()> {
        let out = self.run(&["pipe-pane", "-O", "-t", pane_id, command]).await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "pipe-pane failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Stop streaming. `pipe-pane` with no command detaches the pipe.
    pub async fn pipe_pane_stop(&self, pane_id: &str) -> Result<()> {
        let _ = self.run(&["pipe-pane", "-t", pane_id]).await?;
        Ok(())
    }

    /// Everything tmux still holds for a pane, scrollback included, with colour.
    ///
    /// Sent once before live streaming begins so a client opens onto the session
    /// as it already is rather than onto a blank screen.
    pub async fn capture_history(&self, pane_id: &str) -> Result<String> {
        let out = self.run(&["capture-pane", "-e", "-p", "-S", "-", "-t", pane_id]).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(out.stdout)
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

    // tmux renders `#{pane_dead}` as "1" when set and empty when not.
    let dead = f.get(8).map(|v| v.trim() == "1").unwrap_or(false);
    let dead_status = f.get(9).and_then(|v| v.trim().parse::<i32>().ok());

    Some(TaggedPane {
        daemon_id,
        workspace_id,
        terminal_id,
        schema_version,
        pane_id: f[0].trim().to_string(),
        window_id: f[1].trim().to_string(),
        columns: f[2].trim().parse().unwrap_or(0),
        rows: f[3].trim().parse().unwrap_or(0),
        dead,
        dead_status,
    })
}

/// Parse `display-message -p "#{cursor_x}\t#{cursor_y}"`.
fn parse_cursor(text: &str) -> Option<(u32, u32)> {
    let line = text.lines().next()?;
    let (x, y) = line.split_once('\t')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_position_is_parsed() {
        assert_eq!(parse_cursor("12\t7\n"), Some((12, 7)));
        assert_eq!(parse_cursor("0\t0"), Some((0, 0)));
    }

    #[test]
    fn a_cursor_reply_that_is_not_two_numbers_is_rejected() {
        // Guessing a position would put the caret somewhere the user is not
        // typing, which is worse than leaving it where the replay ended.
        assert_eq!(parse_cursor(""), None);
        assert_eq!(parse_cursor("12"), None);
        assert_eq!(parse_cursor("a\tb"), None);
    }

    fn line(daemon: &str, ws: &str, term: &str) -> String {
        format!("%3\t@2\t120\t40\t{daemon}\t{ws}\t{term}\t1\t\t")
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
        assert!(parse_pane_line("%1\t@1\t80\t24\t\t\t\t\t\t").is_none());
    }

    #[test]
    fn partially_tagged_pane_is_not_identity() {
        let d = Uuid::from_u128(1).to_string();
        assert!(parse_pane_line(&format!("%1\t@1\t80\t24\t{d}\t\t\t1\t\t")).is_none());
    }

    #[test]
    fn parses_a_dead_pane_with_its_exit_status() {
        let d = Uuid::from_u128(1).to_string();
        let w = Uuid::from_u128(2).to_string();
        let t = Uuid::from_u128(3).to_string();
        let p = parse_pane_line(&format!("%3\t@2\t120\t40\t{d}\t{w}\t{t}\t1\t1\t137")).unwrap();
        assert!(p.dead, "a retained pane reports itself dead");
        assert_eq!(p.dead_status, Some(137));
        assert!(!p.proves_life(), "a dead pane must never prove life");
    }

    #[test]
    fn a_live_pane_reports_itself_alive() {
        let d = Uuid::from_u128(1).to_string();
        let w = Uuid::from_u128(2).to_string();
        let t = Uuid::from_u128(3).to_string();
        let p = parse_pane_line(&format!("%3\t@2\t120\t40\t{d}\t{w}\t{t}\t1\t\t")).unwrap();
        assert!(!p.dead);
        assert!(p.proves_life());
    }

    #[test]
    fn truncated_line_is_ignored() {
        assert!(parse_pane_line("%1\t@1\t80").is_none());
    }

    #[test]
    fn non_uuid_tag_is_ignored() {
        assert!(parse_pane_line("%1\t@1\t80\t24\tnot-a-uuid\tx\ty\t1\t\t").is_none());
    }
}

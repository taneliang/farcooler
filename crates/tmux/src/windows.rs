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

        self.tag_window(&win.window_id, workspace_id).await?;
        self.tag_pane(&win.pane_id, terminal_id).await?;
        Ok(win)
    }

    /// Tag a window with what every pane in it shares.
    ///
    /// A window is a LAYOUT: one workspace, several terminals. So the daemon,
    /// the workspace and the schema live here and every pane inherits them in a
    /// format string, which is why `list-panes` can still read them per pane.
    async fn tag_window(&self, window_id: &str, workspace_id: Uuid) -> Result<()> {
        for (k, v) in [
            (tags::DAEMON_ID, self.daemon_id().to_string()),
            (tags::WORKSPACE_ID, workspace_id.to_string()),
            (tags::SCHEMA_VERSION, SCHEMA_VERSION.to_string()),
        ] {
            let out = self.run(&["set-option", "-w", "-t", window_id, k, &v]).await?;
            if !out.ok() {
                tracing::warn!(tag = k, stderr = %out.stderr, "failed to tag window");
                return Err(DomainError::TmuxUnavailable);
            }
        }
        Ok(())
    }

    /// Tag a pane with the one thing that is its own.
    ///
    /// A PANE option, not a window one, and that distinction is load-bearing.
    /// Terminal identity used to be a window option, which was correct only while
    /// every window held exactly one pane: window options are inherited, so the
    /// moment a window was split both panes reported the same terminal id and the
    /// inventory saw one terminal in two places.
    async fn tag_pane(&self, pane_id: &str, terminal_id: Uuid) -> Result<()> {
        let out = self
            .run(&["set-option", "-p", "-t", pane_id, tags::TERMINAL_ID, &terminal_id.to_string()])
            .await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "failed to tag pane");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Fresh inventory of every live pane carrying our exact tags.
    ///
    /// One bulk query, never one round trip per terminal, because derivation
    /// sits on the fleet-render path.
    pub async fn list_tagged_panes(&self) -> Result<Vec<TaggedPane>> {
        // Geometry comes along for the ride: it is the same query, and asking
        // tmux where a pane is costs nothing next to computing it twice.
        let fmt = format!(
            "#{{pane_id}}\t#{{window_id}}\t#{{pane_width}}\t#{{pane_height}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{pane_dead}}\t#{{pane_dead_status}}\t#{{pane_current_command}}\t#{{pane_left}}\t#{{pane_top}}\t#{{window_active}}\t#{{pane_active}}\t#{{window_zoomed_flag}}\t#{{pane_tty}}",
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

    /// The modes a pane's program has turned on.
    ///
    /// A captured screen is contents without modes, and modes are what a
    /// program set once, long before any of this session's clients attached.
    /// So a client that replays a capture believes the program wants no mouse,
    /// is not on the alternate screen, and sends ordinary arrow keys — and is
    /// wrong about all three for every full-screen program there is. tmux knows,
    /// because tmux is the emulator that parsed those sequences; this is asking
    /// it, so a replay can put a fresh emulator into the state the program
    /// believes it is talking to.
    pub async fn pane_modes(&self, pane_id: &str) -> Result<PaneModes> {
        let format = "#{alternate_on}\t#{mouse_standard_flag}\t#{mouse_button_flag}\t\
                      #{mouse_any_flag}\t#{mouse_sgr_flag}\t#{mouse_utf8_flag}\t\
                      #{cursor_flag}\t#{keypad_cursor_flag}\t#{keypad_flag}\t#{wrap_flag}";
        let out = self.run(&["display-message", "-p", "-t", pane_id, format]).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        parse_modes(&out.stdout).ok_or(DomainError::TmuxUnavailable)
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
    let command = f.get(10).map(|v| v.trim().to_string()).unwrap_or_default();
    let cell = |i: usize| f.get(i).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
    let flag = |i: usize| f.get(i).map(|v| v.trim() == "1").unwrap_or(false);

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
        command,
        left: cell(11),
        top: cell(12),
        window_active: flag(13),
        pane_active: flag(14),
        // `window_zoomed_flag` is a window property, so it is only meaningful
        // together with `pane_active`: the zoomed pane is the active one.
        zoomed: flag(15) && flag(14),
        tty: f.get(16).map(|v| v.trim().to_string()).unwrap_or_default(),
    })
}

/// The modes a pane's program has turned on, as tmux reports them.
///
/// Deliberately the flags rather than the escape sequences that set them: this
/// is what tmux knows, and turning it into sequences is the replay's job, not
/// the inventory's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneModes {
    pub alternate_screen: bool,
    pub mouse_standard: bool,
    pub mouse_button: bool,
    pub mouse_any: bool,
    pub mouse_sgr: bool,
    pub mouse_utf8: bool,
    pub cursor_visible: bool,
    pub application_cursor_keys: bool,
    pub application_keypad: bool,
    pub wrap: bool,
}

impl PaneModes {
    /// The sequences that put a fresh emulator into this state.
    ///
    /// The alternate screen comes first and everything else follows, because
    /// switching screens is what decides which screen the replay's clear and
    /// contents land on. Modes that are off are written as explicitly off
    /// rather than omitted: an emulator being reused for a second terminal
    /// would otherwise keep the first one's modes.
    pub fn restore_sequence(&self) -> String {
        let mut out = String::new();
        // Without ?1049h a full-screen program's redraws pile into the primary
        // screen's scrollback instead of replacing the screen, which is a
        // history that grows forever and a caret that jumps to the end of it.
        out.push_str(if self.alternate_screen { "\x1b[?1049h" } else { "\x1b[?1049l" });
        for (on, code) in [
            (self.mouse_standard, "1000"),
            (self.mouse_button, "1002"),
            (self.mouse_any, "1003"),
            (self.mouse_utf8, "1005"),
            (self.mouse_sgr, "1006"),
            (self.application_cursor_keys, "1"),
            (self.wrap, "7"),
            (self.cursor_visible, "25"),
        ] {
            out.push_str(&format!("\x1b[?{code}{}", if on { "h" } else { "l" }));
        }
        // Application keypad has no private-mode form; it is its own pair.
        out.push_str(if self.application_keypad { "\x1b=" } else { "\x1b>" });
        out
    }
}

/// Parse the tab-separated flags `pane_modes` asks for.
fn parse_modes(text: &str) -> Option<PaneModes> {
    let line = text.lines().next()?;
    let f: Vec<&str> = line.split('\t').map(str::trim).collect();
    if f.len() < 10 {
        return None;
    }
    let on = |i: usize| f[i] == "1";
    Some(PaneModes {
        alternate_screen: on(0),
        mouse_standard: on(1),
        mouse_button: on(2),
        mouse_any: on(3),
        mouse_sgr: on(4),
        mouse_utf8: on(5),
        cursor_visible: on(6),
        application_cursor_keys: on(7),
        application_keypad: on(8),
        wrap: on(9),
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
    fn pane_modes_are_parsed_in_order() {
        let m = parse_modes("1\t0\t0\t1\t1\t0\t1\t0\t0\t1\n").expect("parsed");
        assert!(m.alternate_screen);
        assert!(m.mouse_any, "any-event tracking is what a modern TUI asks for");
        assert!(m.mouse_sgr);
        assert!(!m.mouse_standard);
        assert!(m.cursor_visible);
        assert!(m.wrap);
    }

    #[test]
    fn a_short_mode_reply_is_rejected() {
        // Guessing would put an emulator into modes the program never asked
        // for, which is worse than replaying none of them.
        assert_eq!(parse_modes("1\t0"), None);
        assert_eq!(parse_modes(""), None);
    }

    #[test]
    fn restoring_modes_switches_screens_before_anything_else() {
        let m = PaneModes { alternate_screen: true, mouse_any: true, mouse_sgr: true, ..Default::default() };
        let s = m.restore_sequence();
        assert!(s.starts_with("\x1b[?1049h"), "the screen has to be chosen first: {s:?}");
        assert!(s.contains("\x1b[?1003h"));
        assert!(s.contains("\x1b[?1006h"));
    }

    #[test]
    fn modes_that_are_off_are_stated_rather_than_omitted() {
        // An emulator pointed at a second terminal would otherwise keep the
        // first one's modes and report mouse events nobody asked for.
        let s = PaneModes::default().restore_sequence();
        assert!(s.contains("\x1b[?1049l"));
        assert!(s.contains("\x1b[?1003l"));
        assert!(s.contains("\x1b>"));
    }

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

/// Which way a split runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Side by side. tmux's `-h`.
    Horizontal,
    /// Stacked. tmux's `-v`.
    Vertical,
}

impl Axis {
    fn flag(self) -> &'static str {
        match self {
            Axis::Horizontal => "-h",
            Axis::Vertical => "-v",
        }
    }
}

/// One of a window's layouts, as tmux names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

impl Preset {
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::EvenHorizontal => "even-horizontal",
            Preset::EvenVertical => "even-vertical",
            Preset::MainHorizontal => "main-horizontal",
            Preset::MainVertical => "main-vertical",
            Preset::Tiled => "tiled",
        }
    }
}

/// A window: one layout, holding one or more terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLayout {
    pub window_id: String,
    pub workspace_id: Uuid,
    pub name: String,
    pub active: bool,
    /// tmux's own layout description — the split tree, verbatim.
    ///
    /// Carried opaquely and handed straight back to `select-layout` to restore an
    /// arrangement. Overnight never parses it: it is tmux's format, tmux is the
    /// only thing that has to understand it, and a parser here would be a copy of
    /// tmux's tree that could drift from it.
    pub layout: String,
    pub index: u32,
}

/// Arrangement, delegated.
///
/// Every function here is one tmux command. That is the point: tmux has had split
/// trees, five named layouts, resizable dividers, zoom, and pane movement between
/// windows for twenty years, and it is already the authority for what is running.
/// Reimplementing the arrangement half in the daemon meant a second tree to keep
/// correct and a third copy of the geometry in every client that drew it.
///
/// So a WINDOW is a layout and a PANE is a terminal, and the daemon stores neither
/// — it asks. Nothing about an arrangement is durable, which is right: the panes
/// are processes, and if the server they live in dies there is no arrangement left
/// to restore them into.
impl TmuxServer {
    /// Split a pane, giving the new half a terminal of its own.
    ///
    /// `before` puts the new pane first, which is tmux's `-b`, and is what a drop
    /// on the left or top edge of a pane means.
    pub async fn split_pane(
        &self,
        target_pane: &str,
        axis: Axis,
        terminal_id: Uuid,
        worktree: &str,
        command: &str,
        before: bool,
    ) -> Result<String> {
        let mut args: Vec<&str> =
            vec!["split-window", axis.flag(), "-d", "-P", "-F", "#{pane_id}", "-t", target_pane];
        if before {
            args.push("-b");
        }
        args.extend_from_slice(&["-c", worktree, command]);

        let out = self.run(&args).await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "split-window failed");
            return Err(DomainError::TmuxUnavailable);
        }
        let pane_id = out.stdout.trim().to_string();
        if pane_id.is_empty() {
            return Err(DomainError::TmuxUnavailable);
        }
        self.tag_pane(&pane_id, terminal_id).await?;
        Ok(pane_id)
    }

    /// Every layout the daemon owns, across every workspace.
    pub async fn list_layouts(&self) -> Result<Vec<ManagedLayout>> {
        let fmt = format!(
            "#{{window_id}}\t#{{window_name}}\t#{{window_active}}\t#{{window_layout}}\t#{{window_index}}\t#{{{}}}\t#{{{}}}",
            tags::WORKSPACE_ID,
            tags::DAEMON_ID
        );
        let out = self.run(&["list-windows", "-a", "-F", &fmt]).await?;
        if !out.ok() {
            if out.stderr.contains("no server running")
                || out.stderr.contains("no current session")
                || out.stderr.contains("error connecting")
            {
                return Ok(Vec::new());
            }
            tracing::warn!(stderr = %out.stderr, "list-windows failed");
            return Err(DomainError::TmuxUnavailable);
        }

        let mine = self.daemon_id();
        Ok(out
            .stdout
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() < 7 {
                    return None;
                }
                if Uuid::parse_str(f[6].trim()).ok()? != mine {
                    return None;
                }
                Some(ManagedLayout {
                    window_id: f[0].trim().to_string(),
                    workspace_id: Uuid::parse_str(f[5].trim()).ok()?,
                    name: f[1].trim().to_string(),
                    active: f[2].trim() == "1",
                    layout: f[3].trim().to_string(),
                    index: f[4].trim().parse().unwrap_or(0),
                })
            })
            .collect())
    }

    /// Rearrange a window into one of tmux's five named layouts.
    pub async fn select_preset(&self, window_id: &str, preset: Preset) -> Result<()> {
        self.expect(&["select-layout", "-t", window_id, preset.as_str()], "select-layout").await
    }


    /// Cycle to the next named layout, tmux's `prefix Space`.
    pub async fn next_preset(&self, window_id: &str) -> Result<()> {
        self.expect(&["next-layout", "-t", window_id], "next-layout").await
    }

    /// Focus a pane. This is what decides where keystrokes go.
    pub async fn select_pane(&self, pane_id: &str) -> Result<()> {
        self.expect(&["select-pane", "-t", pane_id], "select-pane").await
    }

    /// Show a layout, and only that one, within its session.
    pub async fn select_window(&self, window_id: &str) -> Result<()> {
        self.expect(&["select-window", "-t", window_id], "select-window").await
    }

    /// Toggle tmux's own zoom on a pane.
    pub async fn toggle_zoom(&self, pane_id: &str) -> Result<()> {
        self.expect(&["resize-pane", "-Z", "-t", pane_id], "resize-pane -Z").await
    }

    /// Clear zoom if it is set, leaving it clear if it is not.
    pub async fn unzoom(&self, window_id: &str) -> Result<()> {
        // `-Z` toggles, so it is only safe to send when something is zoomed.
        let out = self
            .run(&["display-message", "-p", "-t", window_id, "#{window_zoomed_flag}"])
            .await?;
        if out.ok() && out.stdout.trim() == "1" {
            self.expect(&["resize-pane", "-Z", "-t", window_id], "resize-pane -Z").await?;
        }
        Ok(())
    }

    /// Move a pane into another window, beside a pane already there.
    ///
    /// tmux's `join-pane`, which is what a drop on a pane's edge is: the dragged
    /// terminal becomes a split of the target, on the side you dropped it.
    pub async fn join_pane(
        &self,
        source_pane: &str,
        target_pane: &str,
        axis: Axis,
        before: bool,
        terminal_id: Uuid,
    ) -> Result<()> {
        let mut args: Vec<&str> =
            vec!["join-pane", axis.flag(), "-s", source_pane, "-t", target_pane];
        if before {
            args.push("-b");
        }
        self.expect(&args, "join-pane").await?;
        // Re-tagged, because a pane that changes window changes which window's
        // options it inherits.
        //
        // A terminal whose id was recorded as a WINDOW option — which is every
        // terminal created before identity moved to the pane, and the only
        // arrangement that existed while a window held exactly one pane — loses
        // that id the instant it is joined somewhere else. The pane survives, the
        // process survives, and the daemon can no longer tell which terminal it
        // is, so the record derives as `lost` and its workspace as `error`.
        //
        // Setting it here makes the move self-healing: whatever the pane's
        // identity rested on before, it rests on the pane afterwards.
        self.tag_pane(source_pane, terminal_id).await
    }

    /// Swap two panes' positions without changing the arrangement.
    pub async fn swap_panes(&self, a: &str, b: &str) -> Result<()> {
        self.expect(&["swap-pane", "-s", a, "-t", b], "swap-pane").await
    }

    /// Pull a pane out into a layout of its own.
    ///
    /// Returns the new window. `-d` leaves the current layout on screen, because
    /// breaking a pane out is usually tidying rather than navigation.
    pub async fn break_pane(
        &self,
        pane_id: &str,
        workspace_id: Uuid,
        terminal_id: Uuid,
    ) -> Result<String> {
        let out = self
            .run(&["break-pane", "-d", "-P", "-F", "#{window_id}", "-s", pane_id])
            .await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "break-pane failed");
            return Err(DomainError::TmuxUnavailable);
        }
        let window_id = out.stdout.trim().to_string();
        if window_id.is_empty() {
            return Err(DomainError::TmuxUnavailable);
        }
        // A new window carries none of the old one's options, so both halves of
        // the identity have to be restated: the window's, and — for the same
        // reason as `join_pane` — the pane's.
        self.tag_window(&window_id, workspace_id).await?;
        self.tag_pane(pane_id, terminal_id).await?;
        Ok(window_id)
    }

    /// Set a pane's terminal tag from outside the crate.
    ///
    /// Exposed only for the startup repair: everything else that needs it does so
    /// as part of an operation that already owns the pane.
    pub async fn tag_pane_public(&self, pane_id: &str, terminal_id: Uuid) -> Result<()> {
        self.tag_pane(pane_id, terminal_id).await
    }

    /// Kill one pane.
    ///
    /// Distinct from `kill_terminal_window`, which took the whole window. That
    /// was equivalent while every window held one pane; now a window is a layout
    /// and killing it would take every terminal arranged in it.
    pub async fn kill_pane(&self, pane_id: &str) -> Result<bool> {
        let out = self.run(&["kill-pane", "-t", pane_id]).await?;
        Ok(out.ok())
    }

    /// Replace the process in a pane, keeping the pane.
    ///
    /// This is how pane mode is toggled. `kill-pane` plus `new-window` would
    /// give the terminal a new pane id — losing its tag, and its position in
    /// whatever layout the user had built — so a chat opening in one tile of
    /// four would rearrange the other three.
    ///
    /// `-k` kills whatever is running first; without it tmux refuses on a live
    /// pane. The working directory goes through tmux's validated `-c` rather
    /// than as `cd` text, for the same reason as `create_terminal_window`.
    pub async fn respawn_pane(&self, pane_id: &str, worktree: &str, command: &str) -> Result<()> {
        let out = self
            .run(&["respawn-pane", "-k", "-t", pane_id, "-c", worktree, command])
            .await?;
        if !out.ok() {
            tracing::warn!(pane = %pane_id, stderr = %out.stderr, "respawn-pane failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }

    /// Give a layout a name, which is what a client shows in its tab.
    pub async fn rename_layout(&self, window_id: &str, name: &str) -> Result<()> {
        self.expect(&["rename-window", "-t", window_id, name], "rename-window").await
    }

    /// Nudge a divider. `amount` is in cells.
    pub async fn resize_pane(&self, pane_id: &str, axis: Axis, amount: i32) -> Result<()> {
        let direction = match (axis, amount >= 0) {
            (Axis::Horizontal, true) => "-R",
            (Axis::Horizontal, false) => "-L",
            (Axis::Vertical, true) => "-D",
            (Axis::Vertical, false) => "-U",
        };
        let cells = amount.abs().to_string();
        self.expect(&["resize-pane", "-t", pane_id, direction, &cells], "resize-pane").await
    }

    /// How big a window currently is, as (columns, rows).
    ///
    /// Asked for rather than derived from the panes, because a layout's panes
    /// do not add up to their window: the dividers between them are columns and
    /// rows too, and reconstructing that arithmetic here would be a second,
    /// worse copy of the layout tree tmux already holds.
    pub async fn window_size(&self, window_id: &str) -> Result<(u32, u32)> {
        let out = self
            .run(&["display-message", "-p", "-t", window_id, "#{window_width}\t#{window_height}"])
            .await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        parse_cursor(&out.stdout).ok_or(DomainError::TmuxUnavailable)
    }

    /// Set a pane to an exact size, taking the difference from its siblings.
    ///
    /// The relative form above is what a human dragging a divider wants; this
    /// is what a client asking for a viewport wants, and the two are different
    /// enough that computing one from the other at every call site would just
    /// be this function written badly several times.
    pub async fn set_pane_size(&self, pane_id: &str, columns: u32, rows: u32) -> Result<()> {
        self.expect(
            &["resize-pane", "-t", pane_id, "-x", &columns.to_string(), "-y", &rows.to_string()],
            "resize-pane to an exact size",
        )
        .await
    }

    /// Run a command that is expected to succeed, and say so if it does not.
    async fn expect(&self, args: &[&str], what: &str) -> Result<()> {
        let out = self.run(args).await?;
        if !out.ok() {
            tracing::warn!(command = what, stderr = %out.stderr, "tmux command failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }
}

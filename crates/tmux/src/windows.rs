//! Managed window and pane operations.
//!
//! Every command targets stable session, window, or pane IDs rather than names
//! or indexes. Names, indexes, and PID values are display or diagnostic data
//! only and never establish identity.

use farcooler_core::{DomainError, Result, SCHEMA_VERSION, inventory::TaggedPane, tags};
use uuid::Uuid;

use crate::server::{SESSION_NAME, TmuxServer};

/// What every pane an open makes carries in its start command, before any
/// step of the open can fail: `<command> #farcooler-opening:<terminal id>`.
///
/// The mark is how the sweep tells a pane this daemon started and never
/// finished from one somebody else made. Nothing unsets `TMUX` in a pane, so
/// a `tmux split-window` typed in a Far Cooler terminal, an agent's teammate
/// mode or tmuxinator all add panes to our session, untagged, and those are
/// someone's work. They never carry this mark, because tmux records the
/// command a pane was started with and theirs is not ours. Untagged and
/// unmarked is never touched.
///
/// A trailing comment, so the command runs exactly as it would without it:
/// tmux hands a one-string command to `default-shell -c`, and sh, bash, zsh,
/// fish, dash and tcsh all read `#` after a space as the start of a comment.
/// It is in the start command and not in an option set afterwards because
/// that would be a second command, which is the very step that can fail.
pub const OPENING_MARK: &str = "#farcooler-opening:";

/// `command`, carrying the opening mark for `terminal_id`. See `OPENING_MARK`.
pub fn marked(command: &str, terminal_id: Uuid) -> String {
    format!("{command} {OPENING_MARK}{terminal_id}")
}

/// A pane an open started and never finished. See `unfinished_opens`.
///
/// The pid comes along because a pane id alone is only unique for one
/// server's life: a server that restarts numbers from `%0` again, and a sweep
/// that remembered `%0` from before would take the new one for the old.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnfinishedOpen {
    pub pane_id: String,
    pub pid: u32,
}

/// A window created for one terminal, addressed by its stable tmux ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWindow {
    pub window_id: String,
    pub pane_id: String,
}

impl TmuxServer {
    /// Create a tagged window running `command` with its working directory set
    /// to the worktree.
    ///
    /// The working directory is passed as tmux's validated `-c` argument rather
    /// than as `cd` text, so no path is ever interpolated into a shell string.
    pub async fn create_terminal_window(
        &self,
        worktree_id: Uuid,
        terminal_id: Uuid,
        title: &str,
        worktree: &str,
        command: &str,
    ) -> Result<ManagedWindow> {
        // The first terminal creates the session. There is no sentinel window,
        // so nothing squats the base index.
        let session_exists = self.is_running().await;
        let target = format!("{SESSION_NAME}:");
        let command = marked(command, terminal_id);
        let command = command.as_str();

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

        // The ids first, so that from here on a failure has a window to take
        // back. See `abandon`.
        let line = out.stdout.trim();
        let mut parts = line.split_whitespace();
        let (Some(window_id), Some(pane_id)) = (parts.next(), parts.next()) else {
            // No id to take back by. Not the session either, even one this
            // open made: other opens may have added windows to it since. The
            // pane carries `OPENING_MARK`, so the sweep finds it.
            tracing::warn!(line, "unparsable new-window output");
            return Err(DomainError::TmuxUnavailable);
        };

        let win = ManagedWindow {
            window_id: window_id.to_string(),
            pane_id: pane_id.to_string(),
        };

        // The session's tags are best effort: nothing reads them, and a
        // window that was made fine is not worth failing over them.
        if !session_exists && let Err(e) = self.tag_session().await {
            tracing::warn!(error = %e, "could not tag the session");
        }
        let tagged = async {
            self.tag_window(&win.window_id, worktree_id).await?;
            self.tag_pane(&win.pane_id, terminal_id).await
        }
        .await;
        if let Err(e) = tagged {
            // Only this open's own window, always. Even when this open made
            // the session, other opens may have added theirs since, and tmux
            // closes a session with its last window anyway.
            self.abandon(&["kill-window", "-t", &win.window_id]).await;
            return Err(e);
        }
        Ok(win)
    }

    /// Take back what a failed open made.
    ///
    /// An open is several commands, and the first one already started the
    /// process: once `new-window` has answered, a tag that then fails or times
    /// out leaves a live pane with no terminal id. The inventory names panes by
    /// that id, so nothing would ever see this one, and its terminal record is
    /// marked failed. The person is told the open failed while the program runs
    /// on out of sight. So the open closes what it made before it reports the
    /// failure, and `unfinished_opens` sweeps up after an open that never got as
    /// far as an id.
    ///
    /// Best effort. If tmux will not take this either, the sweep is the backstop.
    async fn abandon(&self, args: &[&str]) {
        match self.run(args).await {
            Ok(out) if out.ok() => {}
            Ok(out) => tracing::warn!(command = ?args, stderr = %out.stderr, "could not close a failed open"),
            Err(e) => tracing::warn!(command = ?args, error = %e, "could not close a failed open"),
        }
    }

    /// The panes an open started and never tagged.
    ///
    /// What `abandon` could not take back: an open whose client was cut off
    /// before tmux answered with the window's id, so nothing knows which window
    /// to close, or a close that failed in its turn. `list_tagged_panes` never
    /// returns such a pane, which is the whole problem, so this asks for them
    /// directly.
    ///
    /// All three must hold: in `SESSION_NAME` on this install's private
    /// socket, no terminal id, and `OPENING_MARK` in the command tmux started
    /// it with. The mark is what keeps a pane somebody added to our session by
    /// hand, untagged as it is, out of this list. The tag is read as a format,
    /// so a pane that inherits its terminal id from its window counts as
    /// tagged. An empty list, not an error, when no server is running.
    ///
    /// A pane being opened right now is unfinished for a moment too, so this is
    /// only a list. Deciding which have been unfinished too long to be an open
    /// in progress is the caller's job (`watch::unfinished_to_reap`).
    pub async fn unfinished_opens(&self) -> Result<Vec<UnfinishedOpen>> {
        // The start command last: it is the one field that may hold a tab.
        let fmt = format!(
            "#{{session_name}}\t#{{pane_id}}\t#{{pane_pid}}\t#{{{}}}\t#{{pane_start_command}}",
            tags::TERMINAL_ID
        );
        let out = self.run(&["list-panes", "-a", "-F", &fmt]).await?;
        if !out.ok() {
            if out.stderr.contains("no server running")
                || out.stderr.contains("no current session")
                || out.stderr.contains("error connecting")
            {
                return Ok(Vec::new());
            }
            tracing::warn!(stderr = %out.stderr, "list-panes failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(out
            .stdout
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(5, '\t');
                let (session, pane, pid, tag) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
                let started = fields.next().unwrap_or("");
                let ours = session == SESSION_NAME && pane.starts_with('%');
                (ours && tag.trim().is_empty() && started.contains(OPENING_MARK)).then(|| UnfinishedOpen {
                    pane_id: pane.to_string(),
                    pid: pid.trim().parse().unwrap_or(0),
                })
            })
            .collect())
    }

    /// Tag a window with what every pane in it shares.
    ///
    /// A window is a LAYOUT: one worktree, several terminals. So the daemon,
    /// the worktree and the schema live here and every pane inherits them in a
    /// format string, which is why `list-panes` can still read them per pane.
    async fn tag_window(&self, window_id: &str, worktree_id: Uuid) -> Result<()> {
        for (k, v) in [
            (tags::DAEMON_ID, self.daemon_id().to_string()),
            (tags::WORKTREE_ID, worktree_id.to_string()),
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
    ///
    /// A pane that has just died is read again until its exit settles. tmux
    /// sets `pane_dead` when the pty reports end of file, and `pane_dead_status`
    /// only once it has handled SIGCHLD — two separate trips round its event
    /// loop (tmux 3.4, `window_pane_error_callback` and `server_child_exited`).
    /// On Linux end of file can win, and a read landing between the two sees a
    /// dead pane with no exit code: an exit observed, and the one fact about it
    /// that matters, lost. A tight `list-panes` poll on Linux caught it on 16
    /// of 300 `exit 42` panes. Waiting here costs nothing in the common case,
    /// where no pane is in that state, and a few milliseconds when one is.
    ///
    /// The gap is not one loop pass on a busy machine. The exiting process
    /// closes its files and signals its parent as two steps, and can be
    /// descheduled between them: on tmux 3.4 the gap measured 13–51 ms with
    /// eight CPU hogs on four cores and 124–195 ms with thirty-two, and a
    /// 500 ms bound still lost a code on CI (run 37143698766).
    ///
    /// Bounded by `EXIT_SETTLE`, because a command can close its tty and keep
    /// running if it ignores the hangup that follows. Such a pane is reported
    /// dead without a status, as tmux sees it, and is not waited for again
    /// while it stays that way.
    pub async fn list_tagged_panes(&self) -> Result<Vec<TaggedPane>> {
        let mut panes = self.list_tagged_panes_once().await?;
        let started = std::time::Instant::now();
        let deadline = started + EXIT_SETTLE;
        let mut nudged = started;
        loop {
            let unsettled: std::collections::HashSet<String> =
                panes.iter().filter(|p| p.exit_unsettled()).map(|p| p.pane_id.clone()).collect();
            let waiting = {
                let mut gave_up = self.unsettled_exits.lock().expect("unsettled exits lock");
                // Forget panes that settled, respawned or went away, so a pane
                // that dies again later is waited for again.
                gave_up.retain(|id| unsettled.contains(id));
                if std::time::Instant::now() >= deadline {
                    gave_up.extend(unsettled);
                    false
                } else {
                    unsettled.iter().any(|id| !gave_up.contains(id))
                }
            };
            if !waiting {
                return Ok(panes);
            }
            // Waiting alone is not always enough: tmux can lose the wakeup for
            // SIGCHLD and leave the pane's process an unreaped zombie, with no
            // status, for as long as nothing else it started exits. Seen on
            // CI's tmux 3.4 in about one run of the live suite in ten: the
            // pane `Zs` under the server, nothing pending or blocked, and the
            // code still missing a second after a 3 s wait. Any child of the
            // server exiting makes it reap every zombie it has
            // (`waitpid(WAIT_ANY)` in `server_child_signal`), so a `run-shell`
            // of `true` is the nudge. That one recovered the code each time.
            //
            // `-b`, because without it the command waits on the very
            // reaping it is meant to cause.
            if nudged.elapsed() >= EXIT_NUDGE_EVERY {
                nudged = std::time::Instant::now();
                let _ = self.run(&["run-shell", "-b", "true"]).await;
            }
            tokio::time::sleep(EXIT_SETTLE_POLL).await;
            // A failed re-read is asked again rather than taken as the answer.
            // Returning the read in hand here handed back the very pane this
            // loop exists to wait for. The good read is kept, so the deadline
            // returns it rather than turning one pane's exit into an
            // unreadable inventory.
            if let Ok(again) = self.list_tagged_panes_once().await {
                panes = again;
            }
        }
    }

    /// One `list-panes`, as tmux answers it.
    async fn list_tagged_panes_once(&self) -> Result<Vec<TaggedPane>> {
        // Geometry comes along for the ride: it is the same query, and asking
        // tmux where a pane is costs nothing next to computing it twice.
        let fmt = format!(
            "#{{pane_id}}\t#{{window_id}}\t#{{pane_width}}\t#{{pane_height}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{{}}}\t#{{pane_dead}}\t#{{pane_dead_status}}\t#{{pane_current_command}}\t#{{pane_left}}\t#{{pane_top}}\t#{{window_active}}\t#{{pane_active}}\t#{{window_zoomed_flag}}\t#{{pane_tty}}\t#{{pane_dead_signal}}\t#{{pane_title}}",
            tags::DAEMON_ID,
            tags::WORKTREE_ID,
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

        let parsed: Vec<TaggedPane> = out.stdout.lines().filter_map(parse_pane_line).collect();

        // Lines arrived and none of them parsed.
        //
        // Worth saying out loud because it is indistinguishable from "nothing is
        // running" everywhere downstream: the snapshot is empty either way,
        // `derive_terminal` reports every terminal `Lost`, and the app looks
        // broken with nothing anywhere saying why. That is exactly how the
        // missing-locale bug hid — tmux sanitized the tab delimiter to `_`, every
        // line was dropped here in silence, and the symptom surfaced three layers
        // away as panes that never leave `starting`.
        if parsed.is_empty() && out.stdout.lines().any(|l| !l.trim().is_empty()) {
            tracing::warn!(
                lines = out.stdout.lines().count(),
                "tmux listed panes but none could be parsed; the delimiter or the tags have changed"
            );
        }
        Ok(parsed)
    }

    /// Kill exactly the window whose fresh tags match this terminal.
    ///
    /// Never `kill-session`, and never a name or index match.
    ///
    /// Not the way to stop or restart ONE terminal: a window is a layout, so
    /// this takes every other terminal arranged in it. `restart_terminal`
    /// called it and destroyed the siblings of every pane it restarted. Reach
    /// for `kill_pane` or `respawn_pane` instead; this stays for the case
    /// where the whole layout is genuinely the subject.
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

    /// Whether the pane's program has asked for bracketed paste.
    ///
    /// Deliberately not a tenth field on `pane_modes`. That returns the
    /// sequences that RESTORE a pane's modes in a fresh emulator, and every
    /// replaying client applies the whole string; adding bracketing to it would
    /// change what they all do, to serve one caller that asks this once, at the
    /// moment it pastes.
    ///
    /// `None` when this tmux can't say: `bracket_paste_flag` is new in tmux
    /// 3.7, and an older tmux renders a format it doesn't know as nothing,
    /// whether the program asked for bracketing or not (see `parse_flag`).
    pub async fn pane_bracketed_paste(&self, pane_id: &str) -> Result<Option<bool>> {
        let out = self
            .run(&["display-message", "-p", "-t", pane_id, "#{bracket_paste_flag}"])
            .await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        parse_flag(&out.stdout).ok_or(DomainError::TmuxUnavailable)
    }

    /// The pid of the program tmux started in the pane: the one `respawn-pane`
    /// replaces. Read to tell one program in a pane from the next, since a
    /// respawn keeps the pane's id and its `pipe-pane`.
    pub async fn pane_pid(&self, pane_id: &str) -> Result<u32> {
        let out = self.run(&["display-message", "-p", "-t", pane_id, "#{pane_pid}"]).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        out.stdout.trim().parse().map_err(|_| DomainError::TmuxUnavailable)
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

    /// Resize a window and keep every pane's share of it.
    ///
    /// `resize_window` alone lets tmux take the whole difference from the
    /// cells at the right and bottom edges: an agent at 78 columns beside two
    /// shells at 26, narrowed from 105 columns to 55, left the shells one
    /// column wide. So the split tree is read first, the resize happens, and
    /// the tree scaled to the new size is put back with `select-layout`
    /// (see `crate::layout`).
    ///
    /// The resize is the part that has to happen. If the layout cannot be
    /// read or scaled, tmux's own arrangement at the new size is left as it
    /// is, which is no worse than before.
    ///
    /// `select-layout` unzooms a window, so a zoomed one is zoomed again
    /// afterwards; tmux zooms the active pane, which is the one that was.
    pub async fn resize_window_keeping_shares(
        &self,
        window_id: &str,
        columns: u32,
        rows: u32,
    ) -> Result<()> {
        let before = self
            .run(&["display-message", "-p", "-t", window_id, "#{window_layout}\t#{window_zoomed_flag}"])
            .await?;
        self.resize_window(window_id, columns, rows).await?;
        if !before.ok() {
            return Ok(());
        }
        let (layout, zoomed) = before.stdout.trim().split_once('\t').unwrap_or((before.stdout.trim(), "0"));
        // Scaled to the size tmux actually gave the window, not the one asked
        // for: `select-layout` fits a layout of any other size back into the
        // window the way `resize-window` does, squeezing the edge again.
        //
        // From here on the resize has already happened, so nothing that fails
        // is an error for the viewport: tmux's own arrangement at the new size
        // stands, and the failure is logged. `expect` logs a command tmux
        // refused, but not one that never ran (a timeout), hence the log here.
        let (columns, rows) = match self.window_size(window_id).await {
            Ok(size) => size,
            Err(error) => {
                tracing::warn!(%error, window_id, "the resized window's size could not be read; its layout is tmux's own");
                return Ok(());
            }
        };
        let Some(scaled) = crate::layout::scale(layout, columns, rows) else { return Ok(()) };
        // Refused only if the panes changed between the read and now.
        if let Err(error) =
            self.expect(&["select-layout", "-t", window_id, &scaled], "select-layout a scaled layout").await
        {
            tracing::warn!(%error, window_id, "the scaled layout was refused; the resize stands without it");
            return Ok(());
        }
        if zoomed == "1"
            && let Err(error) = self.expect(&["resize-pane", "-Z", "-t", window_id], "resize-pane -Z").await
        {
            tracing::warn!(%error, window_id, "the window could not be zoomed again after a resize");
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


    /// The scrollback a pane still holds ABOVE its visible screen, with color.
    ///
    /// `-E -1` is what makes this the history and not the history plus the
    /// screen. tmux numbers the visible screen from zero and history upwards
    /// from minus one, so ending at `-1` stops exactly where `capture_screen`
    /// begins. Captured whole, a client would replay the current screen twice
    /// and the user would scroll up into a copy of what they were already
    /// looking at.
    ///
    /// `-J` unwraps. tmux stores a wrapped line hard-broken at the width it was
    /// written at, and a client is rarely that width; replayed as stored, every
    /// wrapped line arrives with a break in the wrong column and stays that way.
    /// Joined, it arrives as the one logical line it always was and the client's
    /// own emulator wraps it where its own edge is — which is the whole reason
    /// reflow lives in the emulator.
    pub async fn capture_scrollback(&self, pane_id: &str) -> Result<String> {
        let args = ["capture-pane", "-e", "-p", "-J", "-S", "-", "-E", "-1", "-t", pane_id];
        let out = self.run(&args).await?;
        if !out.ok() {
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(out.stdout)
    }

    /// The newest `lines` of that same scrollback, for a caller that can only
    /// carry so much of it.
    ///
    /// `-E -1` and `-J` for exactly the reasons above: `-1` stops where
    /// `capture_screen` begins, so the visible screen is not captured a second
    /// time and shown to the user as a copy of itself sitting above itself; and
    /// `-J` hands over the logical line rather than the line tmux happened to
    /// hard-wrap at the width it was written, because reflowing it at the
    /// reader's own width is the client emulator's job.
    ///
    /// Bounded, where `capture_scrollback` is not, because of who asks. That one
    /// feeds a stream that then stays open and pours out bytes for as long as
    /// the pane lives, so one large capture at the head of it changes nothing.
    /// This one answers a single request whose whole reply rides in one control
    /// envelope — `MAX_CONTROL_ENVELOPE_BYTES`, a megabyte — over a phone's
    /// link, and a pane sitting on tmux's full history limit is several
    /// megabytes of colored capture. Unbounded, the answer would be refused by
    /// the transport, or arrive after the user had given up scrolling.
    ///
    /// `-S -<lines>` counts back from the top of the screen, so what comes back
    /// is the part of the history nearest what the user is looking at — the
    /// lines a scroll gesture reaches first, rather than the oldest ones it
    /// would have to travel past everything else to see.
    pub async fn capture_scrollback_tail(&self, pane_id: &str, lines: u32) -> Result<String> {
        let start = format!("-{lines}");
        let args = ["capture-pane", "-e", "-p", "-J", "-S", &start, "-E", "-1", "-t", pane_id];
        let out = self.run(&args).await?;
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

/// How long a read waits for a dead pane's exit status to arrive.
///
/// Far longer than the gap is at rest, which is one pass of tmux's event loop,
/// because the gap grows with load (see `list_tagged_panes`) and 500 ms was
/// not enough on CI. Only a pane caught mid-exit pays for it, and only for as
/// long as its exit takes. A pane that never settles, one whose command closed
/// its tty and ignored the hangup, pays it once.
const EXIT_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
const EXIT_SETTLE_POLL: std::time::Duration = std::time::Duration::from_millis(10);
/// How long a dead pane goes without its status before tmux is made to reap.
///
/// Long enough that an exit which is merely slow settles on its own, so the
/// common case runs nothing; short enough that a lost wakeup costs a tenth
/// of a second rather than the whole `EXIT_SETTLE`.
const EXIT_NUDGE_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Parse one `list-panes -F` line. A line missing our tags is not ours.
pub(crate) fn parse_pane_line(line: &str) -> Option<TaggedPane> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 8 {
        return None;
    }

    let daemon_id = Uuid::parse_str(f[4].trim()).ok()?;
    let worktree_id = Uuid::parse_str(f[5].trim()).ok()?;
    let terminal_id = Uuid::parse_str(f[6].trim()).ok()?;
    let schema_version: u32 = f[7].trim().parse().ok()?;

    // tmux renders `#{pane_dead}` as "1" when set, "0" or empty when not.
    let dead = f.get(8).map(|v| v.trim() == "1").unwrap_or(false);
    let dead_status = f.get(9).and_then(|v| v.trim().parse::<i32>().ok());
    let command = f.get(10).map(|v| v.trim().to_string()).unwrap_or_default();
    let cell = |i: usize| f.get(i).and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0);
    let flag = |i: usize| f.get(i).map(|v| v.trim() == "1").unwrap_or(false);

    Some(TaggedPane {
        daemon_id,
        worktree_id,
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
        dead_signal: f.get(17).map(|v| v.trim()).filter(|v| !v.is_empty()).map(str::to_string),
        // Appended last on purpose. A title is user-controlled text and may
        // contain a tab; putting it at the end means such a title costs its own
        // value and not every field after it.
        title: f.get(18).map(|v| v.trim().to_string()).unwrap_or_default(),
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
        // The kitty keyboard protocol, cleared unconditionally: `pane_modes`
        // has no field for it, so a replay can never learn that a pane wants
        // it, and the only honest state to restore is off. Left set, a reused
        // emulator would keep reporting modified keys as CSI u to a program
        // that never negotiated it and has no parser for it.
        out.push_str("\x1b[=0;1u");
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

/// A single tmux flag format: `Some(Some(on))`, or `Some(None)` when tmux
/// doesn't know the flag.
///
/// **Empty is "this tmux can't say", not "off".** tmux renders every flag it
/// knows as `0` or `1`, and a format it doesn't know as nothing.
/// `bracket_paste_flag` arrived in tmux 3.7, so a 3.4 (Ubuntu 24.04's) renders
/// it empty even in a pane whose program turned bracketing on, while
/// rendering `alternate_on` and `wrap_flag` as `0` and `1`, measured on a real
/// 3.4. Each caller decides what not knowing means: `paste_path` pastes
/// unbracketed, and `answer_wake` asks the daemon's record of the pane's
/// output instead (`paste_mode`).
///
/// Anything that is neither a flag nor empty is refused, so a tmux
/// sanitizing its output to underscores in the C locale — the failure
/// `parse_modes` exists to catch — is an error rather than a guess.
fn parse_flag(text: &str) -> Option<Option<bool>> {
    match text.lines().next().unwrap_or("").trim() {
        "1" => Some(Some(true)),
        "0" => Some(Some(false)),
        "" => Some(None),
        _ => None,
    }
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
    fn a_restore_clears_the_keyboard_protocol_it_cannot_know_about() {
        // tmux does not report the kitty keyboard mode in `pane_modes`, so a
        // replay can never turn it on — which makes it all the more important to
        // turn it off. An emulator reused from a pane whose program had pushed
        // the protocol would otherwise keep encoding Shift-Enter as CSI u to a
        // program that never asked and cannot read it.
        let s = PaneModes::default().restore_sequence();
        assert!(s.contains("\x1b[=0;1u"), "the keyboard protocol must be reset: {s:?}");
    }

    #[test]
    fn a_bracketed_paste_flag_is_parsed_including_the_empty_form() {
        assert_eq!(parse_flag("1\n"), Some(Some(true)));
        assert_eq!(parse_flag("0\n"), Some(Some(false)));
        // A tmux older than 3.7 has no `bracket_paste_flag` and renders it as
        // nothing, even with bracketing on: unknown, not off, and not an error
        // either, which made every paste on such a host fail as "tmux is
        // unavailable".
        assert_eq!(parse_flag(""), Some(None));
        assert_eq!(parse_flag("\n"), Some(None));
        // Garbage is still refused: a C-locale tmux sanitizes its output to
        // underscores, and that is a broken reply rather than a false one.
        assert_eq!(parse_flag("_\n"), None);
        assert_eq!(parse_flag("yes\n"), None);
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
        assert_eq!(p.worktree_id, w);
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

    #[test]
    fn a_pane_line_carries_the_title() {
        let d = uuid::Uuid::nil();
        let line = format!(
            "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t\t\tclaude\t0\t0\t1\t1\t0\t/dev/ttys001\t\t◐ Write a haiku"
        );
        let p = parse_pane_line(&line).expect("a well-formed line parses");
        assert_eq!(p.title, "◐ Write a haiku");
        assert_eq!(p.tty, "/dev/ttys001");
    }

    #[test]
    fn a_pane_killed_by_a_signal_carries_the_signal() {
        let d = uuid::Uuid::nil();
        let line = format!(
            "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t1\t\tsh\t0\t0\t1\t1\t0\t/dev/ttys001\tkill\t"
        );
        let p = parse_pane_line(&line).expect("a well-formed line parses");
        assert!(p.dead);
        assert_eq!((p.dead_status, p.dead_signal.as_deref()), (None, Some("kill")));
        assert!(!p.exit_unsettled(), "a signal settles the exit as surely as a code");
    }

    #[test]
    fn a_dead_pane_with_neither_code_nor_signal_is_unsettled() {
        let d = uuid::Uuid::nil();
        let line = format!(
            "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t1\t\tsh\t0\t0\t1\t1\t0\t/dev/ttys001\t\t"
        );
        let p = parse_pane_line(&line).expect("a well-formed line parses");
        assert!(p.dead && p.exit_unsettled());
        assert!(!p.proves_life(), "an unsettled exit is still no proof of life");
    }

    /// A build that predates the title field must still parse.
    #[test]
    fn a_line_without_a_title_still_parses() {
        let d = uuid::Uuid::nil();
        let line = format!(
            "%1\t@0\t80\t24\t{d}\t{d}\t{d}\t1\t\t\tclaude\t0\t0\t1\t1\t0\t/dev/ttys001"
        );
        let p = parse_pane_line(&line).expect("a short line parses");
        assert_eq!(p.title, "");
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
    pub worktree_id: Uuid,
    pub name: String,
    pub active: bool,
    /// tmux's own layout description — the split tree, verbatim.
    ///
    /// Carried opaquely and handed straight back to `select-layout` to restore an
    /// arrangement. It is tmux's format and tmux is the authority on it; the one
    /// reader here is `crate::layout`, which scales it across a resize and
    /// writes it straight back, and keeps no tree of its own.
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
        let command = marked(command, terminal_id);
        args.extend_from_slice(&["-c", worktree, &command]);

        let out = self.run(&args).await?;
        if !out.ok() {
            tracing::warn!(stderr = %out.stderr, "split-window failed");
            return Err(DomainError::TmuxUnavailable);
        }
        let pane_id = out.stdout.trim().to_string();
        if pane_id.is_empty() {
            return Err(DomainError::TmuxUnavailable);
        }
        if let Err(e) = self.tag_pane(&pane_id, terminal_id).await {
            // The split is running a program nothing can name. See `abandon`.
            self.abandon(&["kill-pane", "-t", &pane_id]).await;
            return Err(e);
        }
        Ok(pane_id)
    }

    /// Every layout the daemon owns, across every worktree.
    pub async fn list_layouts(&self) -> Result<Vec<ManagedLayout>> {
        let fmt = format!(
            "#{{window_id}}\t#{{window_name}}\t#{{window_active}}\t#{{window_layout}}\t#{{window_index}}\t#{{{}}}\t#{{{}}}",
            tags::WORKTREE_ID,
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
                    worktree_id: Uuid::parse_str(f[5].trim()).ok()?,
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
        // is, so the record derives as `lost` and its worktree as `error`.
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
        worktree_id: Uuid,
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
        self.tag_window(&window_id, worktree_id).await?;
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

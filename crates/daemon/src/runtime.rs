//! Runtime operations: everything that speaks only to tmux.
//!
//! These never touch the database, and that is a load-bearing property rather
//! than an accident. tmux is the sole authority for what is live, and unlike a
//! database it is safe for several processes to read at once. So streaming a
//! terminal, sending it keystrokes, or resizing it does not have to be
//! serialised through the daemon: a client can do it directly, and the daemon
//! keeps its exclusive hold on the one thing that genuinely needs one owner —
//! durable intent.
//!
//! That split is why `overnight terminal stream` can run as its own process
//! while the daemon serves everything else.

use overnight_core::{DomainError, Result, inventory::RuntimeInventory, validate};
use overnight_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

use crate::paths;
use crate::service::make_fifo;

/// A handle to the live runtime, with no database behind it.
///
/// Cloning shares the same inventory view rather than making a second one, so
/// two handles cannot disagree about what is running.
#[derive(Clone)]
pub struct Runtime {
    pub tmux: TmuxServer,
    pub inventory: LiveInventory,
}

impl Runtime {
    /// Open at the user's runtime directory.
    pub async fn open() -> Result<Self> {
        Self::open_in(&paths::ensure_runtime_dir()?).await
    }

    /// Open at an explicit directory.
    ///
    /// Only the install id is read, because that is all it takes to find the
    /// right tmux server. No database is opened, so this can run beside a
    /// daemon without contending for one.
    pub async fn open_in(root: &std::path::Path) -> Result<Self> {
        let install_id = paths::load_or_create_install_id_in(root)?;
        let host_id = crate::service::stable_host_id(&install_id);
        let tmux = TmuxServer::new(&install_id, host_id);
        let inventory = LiveInventory::new(tmux.clone());
        inventory.refresh().await;
        Ok(Self { tmux, inventory })
    }

    /// Resolve a short terminal id against the LIVE panes.
    ///
    /// Runtime commands resolve against tmux rather than the database on
    /// purpose: these operate on a running terminal, so a name that matches
    /// only an archived record is not a match at all. It also means they need
    /// no database, which is what lets them run beside the daemon.
    pub fn resolve_terminal(&self, prefix: &str) -> Result<Uuid> {
        // Dashless, so a short id and a full hyphenated UUID both work.
        let needle = prefix.trim().to_lowercase().replace('-', "");
        let snapshot = self.inventory.snapshot();
        let mut matches: Vec<Uuid> = snapshot
            .panes
            .iter()
            .map(|p| p.terminal_id)
            .filter(|id| id.simple().to_string().ends_with(&needle))
            .collect();
        matches.sort();
        matches.dedup();

        match matches.len() {
            1 => Ok(matches[0]),
            // Ambiguity is refused rather than resolved by picking one: the
            // wrong terminal is the one your agent is running in.
            0 => Err(DomainError::NotFound),
            _ => Err(DomainError::InvalidArgument { what: "ambiguous terminal id" }),
        }
    }

    pub async fn send_input(&self, id: Uuid, data: &str) -> Result<()> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?
            .pane_id
            .clone();
        self.tmux.send_keys(&pane, data).await
    }

    /// Exact input bytes, hex encoded, to the live pane proving this terminal.
    pub async fn send_bytes_hex(&self, id: Uuid, hex: &str) -> Result<()> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?
            .pane_id
            .clone();
        self.tmux.send_bytes_hex(&pane, hex).await
    }

    /// The rendered visible screen with colour, plus the pane geometry so a
    /// client can size itself to what it is actually showing.
    pub async fn screen(&self, id: Uuid) -> Result<(String, u32, u32)> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();
        let text = self.tmux.capture_screen(&pane.pane_id).await?;
        Ok((text, pane.columns, pane.rows))
    }

    /// Stream a terminal's live output to this process's stdout.
    ///
    /// Emits the retained history first so the client opens onto the session as
    /// it already is, then hands over to a live pipe. Runs until the caller is
    /// killed or the pane goes away.
    pub async fn stream(&self, id: Uuid) -> Result<()> {
        use tokio::io::AsyncWriteExt;

        let snapshot = self.inventory.snapshot();
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?
            .clone();

        let mut stdout = tokio::io::stdout();

        // 1. Replay the VISIBLE screen only, not the whole scrollback.
        //
        // tmux stores scrollback as the lines were written, so history from when
        // the pane was a different width re-wraps in a client sized differently
        // and the screen arrives staggered. The visible screen is re-rendered by
        // tmux at the current size, so it always lands correctly. Deep history
        // is deliberately not replayed; the client accumulates its own scrollback
        // from the live stream onward.
        //
        // Re-read the pane geometry first so a resize that just happened has
        // settled before the capture.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;

        if let Ok(screen) = self.tmux.capture_screen(&pane.pane_id).await {
            // Home the cursor and clear, so the replay paints a clean screen.
            let _ = stdout.write_all(b"\x1b[H\x1b[2J").await;

            // capture-pane separates lines with a bare LF. To a terminal that is
            // line feed WITHOUT carriage return, so every line starts where the
            // previous one ended and the screen arrives as a staircase. The live
            // pipe does not need this because a pty already emits CRLF.
            let normalized = screen.trim_end().replace('\n', "\r\n");
            let _ = stdout.write_all(normalized.as_bytes()).await;

            // No trailing newline.
            //
            // The capture is exactly as many lines as the screen is tall
            // whenever the program fills it — which a full-screen agent always
            // does. One more line feed at the bottom row scrolls the whole
            // screen up by one: the top line is pushed into history, everything
            // appears one row too high, and the caret is left on a blank bottom
            // row. Both symptoms, one newline.

            // A captured screen is text and carries no cursor, so ask tmux
            // where it actually is. Without this the caret sits wherever the
            // last replayed character ended, which is the bottom-left corner,
            // not the prompt the user is typing into.
            if let Ok((column, row)) = self.tmux.cursor_position(&pane.pane_id).await {
                // The wire format is one-based.
                let _ = stdout.write_all(format!("\x1b[{};{}H", row + 1, column + 1).as_bytes()).await;
            }
            let _ = stdout.flush().await;
        }

        // 2. Live bytes through a fifo. tmux writes, we forward.
        let fifo = std::env::temp_dir().join(format!("overnight-stream-{}.fifo", Uuid::now_v7()));
        let fifo_str = fifo.to_string_lossy().to_string();
        make_fifo(&fifo_str)?;

        self.tmux
            .pipe_pane_start(&pane.pane_id, &format!("cat > '{fifo_str}'"))
            .await?;

        // Open the fifo READ-WRITE, not read-only.
        //
        // A read-only fifo reader gets EOF the instant the last writer closes,
        // and tmux's `cat` opens lazily, so a read-only open ends the stream
        // immediately on an idle terminal. Holding a write handle ourselves
        // means there is always at least one writer, so reads block for more
        // data instead of reporting end of stream.
        let file = tokio::task::spawn_blocking({
            let fifo = fifo.clone();
            move || std::fs::OpenOptions::new().read(true).write(true).open(&fifo)
        })
        .await
        .map_err(|_| DomainError::OperationFailed)?
        .map_err(|e| {
            tracing::warn!(error = %e, "could not open stream fifo");
            DomainError::OperationFailed
        })?;
        let file = tokio::fs::File::from_std(file);

        let mut reader = tokio::io::BufReader::new(file);
        let mut buf = vec![0u8; 16 * 1024];

        loop {
            use tokio::io::AsyncReadExt;
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Err(_) => break,
            }
        }

        let _ = self.tmux.pipe_pane_stop(&pane.pane_id).await;
        let _ = std::fs::remove_file(&fifo);
        Ok(())
    }

    /// Persistent input channel: read hex byte-runs from stdin, forward each.
    ///
    /// Spawning a process per keystroke costs a SQLite open and a full tmux
    /// inventory before a single byte moves, which is most of the latency a
    /// typist actually feels. This resolves the pane ONCE and then forwards,
    /// so the steady-state cost of a keystroke is one `send-keys`.
    pub async fn input_channel(&self, id: Uuid) -> Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let snapshot = self.inventory.snapshot();
        let mut pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?
            .pane_id
            .clone();

        let mut lines = BufReader::new(tokio::io::stdin()).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            let hex = line.trim();
            if hex.is_empty() {
                continue;
            }

            if self.tmux.send_bytes_hex(&pane, hex).await.is_err() {
                // The pane may have been replaced by a restart. Re-resolve once
                // rather than dropping the user's input on the floor.
                let fresh = self.inventory.refresh().await;
                match fresh.claimants(id).into_iter().find(|p| p.proves_life()) {
                    Some(p) => {
                        pane = p.pane_id.clone();
                        let _ = self.tmux.send_bytes_hex(&pane, hex).await;
                    }
                    None => break,
                }
            }
        }
        Ok(())
    }

    /// Resize the window backing a terminal to the viewer's geometry.
    pub async fn resize_terminal(&self, id: Uuid, columns: u32, rows: u32) -> Result<()> {
        let (columns, rows) = validate::clamp_size(columns, rows);
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();

        if pane.columns == columns && pane.rows == rows {
            return Ok(());
        }
        self.tmux.resize_window(&pane.window_id, columns, rows).await
    }

    pub async fn capture(&self, id: Uuid, lines: u32) -> Result<String> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .next()
            .ok_or(DomainError::NotFound)?
            .pane_id
            .clone();
        self.tmux.capture_pane(&pane, lines).await
    }
}

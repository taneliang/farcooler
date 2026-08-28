//! Runtime operations: everything that speaks only to tmux.
//!
//! These never touch the database, and that is a load-bearing property rather
//! than an accident. tmux is the sole authority for what is live, and unlike a
//! database it is safe for several processes to read at once. So streaming a
//! terminal, sending it keystrokes, or resizing it does not have to be
//! serialized through the daemon: a client can do it directly, and the daemon
//! keeps its exclusive hold on the one thing that genuinely needs one owner —
//! durable intent.
//!
//! That split is why `farcooler terminal stream` can run as its own process
//! while the daemon serves everything else.

use farcooler_core::{DomainError, Result, inventory::RuntimeInventory, validate};
use farcooler_tmux::{LiveInventory, TmuxServer};
use uuid::Uuid;

use crate::paths;

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
    /// only a stopped record is not a match at all. It also means they need
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

    /// The rendered visible screen with color, plus the pane geometry so a
    /// client can size itself to what it is actually showing.
    pub async fn screen(&self, id: Uuid) -> Result<(String, u32, u32)> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();
        let text = self.tmux.capture_screen(&pane.pane_id).await?;
        Ok((text, pane.columns, pane.rows))
    }

    /// A bounded slice of the pane's scrollback, as bytes a client can feed
    /// straight into its emulator above the screen. See `TerminalScreen.history`.
    ///
    /// Bytes rather than the capture, because assembling them is where the
    /// knowledge is: the alternate screen has no history to send, a bare LF is a
    /// staircase, and a capture that ends mid-color paints the client's next
    /// clear. That is `history_bytes`, and a stream's `replay` uses the same
    /// one — a client polling and a client streaming must not end up holding
    /// different scrollback for the same pane.
    ///
    /// Zero lines is not "all of it": it is the ordinary poll, and it costs
    /// nothing. Nothing is spawned, because the caller that asked for no history
    /// must pay exactly what it paid before this existed.
    pub async fn history(&self, id: Uuid, lines: u32) -> Result<Vec<u8>> {
        if lines == 0 {
            return Ok(Vec::new());
        }
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();

        // Both reads at once, for the reason `stream` gives above: they are two
        // `tmux` processes and neither depends on the other, so awaiting them in
        // turn is two spawns and two connects end to end where one will do.
        let (modes, scrollback) = tokio::join!(
            self.tmux.pane_modes(&pane.pane_id),
            self.tmux.capture_scrollback_tail(&pane.pane_id, lines),
        );
        Ok(history_bytes(modes.ok(), scrollback.ok().as_deref()))
    }

    /// The pane's modes, as the sequences that restore them.
    pub async fn pane_modes(&self, id: Uuid) -> Result<String> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();
        Ok(self.tmux.pane_modes(&pane.pane_id).await?.restore_sequence())
    }

    /// Whether the pane's program has asked for bracketed paste.
    ///
    /// Asked at paste time rather than carried in `pane_modes`, which is the
    /// replay string every client applies wholesale.
    pub async fn pane_bracketed_paste(&self, id: Uuid) -> Result<bool> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();
        self.tmux.pane_bracketed_paste(&pane.pane_id).await
    }

    /// Where the cursor sits, so a remote client can draw it in the right cell.
    ///
    /// Asked for separately from the screen because `capture-pane` does not carry
    /// it: the capture is what the pane HAS, and the cursor is where it is about
    /// to write. A client that guessed from the last non-blank cell would put it
    /// in the wrong place on any screen with trailing output.
    pub async fn cursor(&self, id: Uuid) -> Result<(u32, u32)> {
        let snapshot = self.inventory.snapshot();
        let pane = snapshot.claimants(id).into_iter().next().ok_or(DomainError::NotFound)?.clone();
        self.tmux.cursor_position(&pane.pane_id).await
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

        // 1. Replay the pane: its scrollback, then its visible screen.
        //
        // The scrollback used to be left out. The stated reason was width: tmux
        // stores history hard-wrapped at whatever the pane was when each line
        // was written, so replayed as stored it arrives staggered in a client
        // of any other size. True, and the wrong conclusion — `-J` unwraps it,
        // and reflowing a logical line at the reader's own width is exactly
        // what the client's emulator is for.
        //
        // What the omission actually cost was the scrollback itself. A client
        // opened with an empty history and accumulated one only from the live
        // stream onward, so scrolling up in a pane that had been running for an
        // hour moved by exactly one line — the single line alacritty pushes
        // when the replay's own `ESC[2J` clears the screen — and then stopped.
        // Resizing the window appeared to fix it, because reflowing to fewer
        // rows pushes real rows out of the active region and into history for
        // the first time. That is the bug, and it looked like a scroll bug.
        //
        // Nothing is waited for before the capture. There used to be a flat
        // 120ms sleep here, to let "a resize that just happened settle" — but
        // no resize is ever in flight at this point, because every client
        // AWAITS its own resize before it spawns this command at all (see
        // `TerminalSurface.attach` on the Mac and `TerminalSession.open` on the
        // phone), and tmux has reflowed the pane by the time that call returns.
        // What the sleep actually waited for is the program's repaint after
        // SIGWINCH, which is not needed either: tmux re-renders the capture at
        // the pane's current size, and the program's own repaint arrives on the
        // live pipe below and simply paints over it.
        //
        // It was pure latency, and it was the largest single component of it:
        // every pane opened — every layout switch, every tab, every reconnect —
        // sat blank for those 120ms before a byte could be sent.

        // Asked for together, written in order.
        //
        // These are three separate `tmux` processes, and they used to be awaited
        // one after another purely because that is the order their answers are
        // written in. Nothing in the second depends on the first, so the wait
        // was three process spawns and three connects end to end when it only
        // ever needed to be one — and on a busy server, where each of those
        // queues behind whatever the sampler is doing, three queues hurt three
        // times as much as one.
        //
        // Only the READS overlap. `replay` below orders the writes, because the
        // order is the whole meaning: modes, then scrollback, then clear, then
        // contents, then cursor.
        let (modes, scrollback, screen, cursor) = tokio::join!(
            self.tmux.pane_modes(&pane.pane_id),
            self.tmux.capture_scrollback(&pane.pane_id),
            self.tmux.capture_screen(&pane.pane_id),
            self.tmux.cursor_position(&pane.pane_id),
        );

        let scrollback = scrollback.ok();
        let screen = screen.ok();
        let bytes =
            replay(modes.ok(), scrollback.as_deref(), screen.as_deref(), cursor.ok());
        let _ = stdout.write_all(&bytes).await;
        let _ = stdout.flush().await;

        // 2. Live bytes, shared with every other watcher of this pane.
        //
        // Subscribing rather than starting a pipe of our own: tmux allows one
        // `pipe-pane` per pane, so a second watcher used to replace the first
        // one's pipe and silently end its stream. See `fanout`.
        let mut reader = self.attach_to_fanout(&pane.pane_id).await?;
        let mut buf = vec![0u8; 16 * 1024];

        // Stop when whoever asked for this stops listening.
        //
        // A write failing is the obvious signal and it is not enough: a quiet
        // pane produces nothing to write, so a stream whose ssh channel closed
        // hours ago would sit in `read` forever with nothing to discover. Two
        // of those were found still running from sessions whose app had been
        // relaunched, and under the fanout they are worse than untidy — a
        // watcher that never leaves keeps the pane's pipe alive for nobody.
        //
        // Closed stdin is how the other end says it is gone. ssh closes it when
        // the channel ends, and nothing writes to this process's stdin
        // otherwise, so there is no other meaning to compete with.
        //
        // Only when stdin is a pipe or a socket, though, which is what makes
        // this safe. A terminal would mean a person ran this command in their
        // shell, and reading it would swallow the keystrokes they typed at the
        // shell instead. `/dev/null` — what a GUI application hands a child it
        // did not mean to talk to — is at end of stream from the very first
        // read, and treating that as a hangup would end every stream instantly.
        // Neither carries the "my peer is gone" meaning that a pipe does.
        let mut hangup = Box::pin(async {
            use tokio::io::AsyncReadExt;
            if !Self::stdin_can_hang_up() {
                // Never resolves, so the select below is left with one arm.
                std::future::pending::<()>().await;
            }
            let mut ignored = [0u8; 64];
            let mut stdin = tokio::io::stdin();
            loop {
                match stdin.read(&mut ignored).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => continue,
                }
            }
        });

        loop {
            use tokio::io::AsyncReadExt;
            tokio::select! {
                read = reader.read(&mut buf) => match read {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if stdout.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                        let _ = stdout.flush().await;
                    }
                },
                _ = &mut hangup => break,
            }
        }

        // Nothing to stop. The pipe belongs to the fanout, which is still
        // serving whoever else is watching, and which ends itself once nobody
        // is — a watcher leaving must not take the others' output with it.
        Ok(())
    }

    /// Whether closed stdin would mean anything here.
    ///
    /// True for a pipe or a socket, which is what an ssh channel and a parent
    /// process's pipe both are, and where end of stream genuinely means the
    /// other end is gone. False for everything else — a terminal, where reading
    /// would take a person's keystrokes, and `/dev/null`, which is at end of
    /// stream before it is ever read.
    fn stdin_can_hang_up() -> bool {
        // SAFETY: a zeroed stat is a valid out parameter, and fd 0 is always a
        // valid descriptor number to ask about — fstat reports if it is closed.
        unsafe {
            let mut info: libc::stat = std::mem::zeroed();
            if libc::fstat(0, &mut info) != 0 {
                return false;
            }
            let kind = info.st_mode & libc::S_IFMT;
            kind == libc::S_IFIFO || kind == libc::S_IFSOCK
        }
    }

    /// Get a connection to this pane's fanout, starting one if there is none.
    ///
    /// Connect first, ask questions later: a running fanout is the common case
    /// once anything is watching, and a connection succeeding is a better test
    /// that one is alive than any amount of asking tmux, which would happily
    /// report a pipe into a process that has since died.
    async fn attach_to_fanout(&self, pane_id: &str) -> Result<tokio::net::UnixStream> {
        // Which install is asking. The tmux socket is already named for it —
        // `farcooler-<install id>` — so this is the identity we already hold
        // rather than a second answer to the same question, which could
        // disagree with the first.
        //
        // It is load-bearing. Without it, two daemons on one host share a
        // fanout socket per pane NUMBER, and every tmux server numbers from
        // `%0`: the second daemon connects to the first one's fanout, never
        // starts a pipe of its own, and reads a stranger's pane. See
        // `fanout::socket_path`.
        let install = self.tmux.socket().to_string();
        if let Some(socket) = crate::fanout::subscribe(&install, pane_id).await {
            return Ok(socket);
        }

        let exe = fanout_binary().ok_or_else(|| {
            tracing::warn!("cannot find farcoolerd to pipe this pane into");
            DomainError::OperationFailed
        })?;
        // The pane NUMBER, not the pane id, because tmux expands this command
        // as a format string before running it and `%` starts an expansion
        // there. A pane id is `%0`, so passing one whole handed tmux an escape
        // sequence: `%15` arrived as `15` by luck, and `%0` arrived as an
        // environment variable's contents. The fanout then listened on a socket
        // named after nonsense, the watcher that started it could never
        // connect, and after a second of trying the stream gave up and exited —
        // which a client cannot tell apart from a pane that finished. The
        // socket name strips `%` on both sides, so the number is the whole id.
        //
        // The install goes with it for the same reason the pane number does:
        // the fanout has to bind the socket this daemon will look for, and only
        // this daemon knows which install it is. An id is hex, so tmux has
        // nothing in it to expand.
        let command = format!(
            "'{}' --fanout '{}' --install '{}'",
            exe.display(),
            pane_id.trim_start_matches('%'),
            install,
        );
        self.tmux.pipe_pane_start(pane_id, &command).await?;

        // Retried rather than slept through: the fanout has a process to spawn
        // and a socket to bind before it can be connected to, and how long that
        // takes belongs to the machine, not to a number chosen here.
        //
        // Probed BEFORE the first sleep, which is the difference between "as
        // fast as the machine can manage" and "never faster than 10ms". The
        // socket is frequently already there: `pipe_pane_start` above has
        // already awaited tmux, and within the fanout's five-second idle grace
        // a pane that was open moments ago still has a live one to connect to —
        // the common case when someone switches away from a layout and back.
        // Sleeping first put a floor under every one of those.
        for attempt in 0..100 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            if let Some(socket) = crate::fanout::subscribe(&install, pane_id).await {
                return Ok(socket);
            }
        }
        tracing::warn!(pane = pane_id, "the pane fanout never came up");
        Err(DomainError::OperationFailed)
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

        // Size the PANE, not the window.
        //
        // A window's layout divides it between its panes, so resizing the
        // window to a client's viewport gave a client showing one pane of a
        // six-pane window a pane of 27x8 — a quarter of what it asked for. The
        // viewport a client describes is the area it will draw ONE terminal
        // into, and that is a pane.
        //
        // Which means the window has to grow by whatever the pane gains, and
        // the siblings have to be put back afterwards. `resize-pane` takes the
        // difference from whichever sibling is adjacent, so sizing the target
        // alone squashed two neighbours from eight columns to one and three.
        // Restoring the siblings first and sizing the target last spends the
        // window's new space on the pane that asked for it and leaves everyone
        // else where they were.
        //
        // Nobody renders a tmux window — clients render panes — so a window
        // wider than any one screen costs nothing.
        let siblings: Vec<_> = snapshot
            .panes
            .iter()
            .filter(|p| p.window_id == pane.window_id && p.pane_id != pane.pane_id)
            .map(|p| (p.pane_id.clone(), p.columns, p.rows))
            .collect();

        let window = self.tmux.window_size(&pane.window_id).await?;
        self.tmux
            .resize_window(
                &pane.window_id,
                (window.0 + columns).saturating_sub(pane.columns).max(columns),
                (window.1 + rows).saturating_sub(pane.rows).max(rows),
            )
            .await?;
        for (id, was_columns, was_rows) in siblings {
            let _ = self.tmux.set_pane_size(&id, was_columns, was_rows).await;
        }
        self.tmux.set_pane_size(&pane.pane_id, columns, rows).await
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

/// The binary that serves a pane fanout: `farcoolerd`, never `farcooler`.
///
/// NOT `current_exe()`. `--fanout` is the daemon's flag, but this code also
/// runs inside the CLI — `farcooler terminal stream` opens a Runtime of its
/// own — so `current_exe()` there is the CLI, which pipes the pane into
/// `farcooler --fanout N`, gets "unexpected argument", and never binds the
/// socket. The stream then times out after a second and exits, which a client
/// cannot tell apart from a pane that finished: keystrokes land, the pane
/// changes, and the screen simply never updates.
///
/// Same shape as `service::shim_binary`, and for the same reason: two binaries
/// come out of this workspace and only one of them answers any given flag.
///
/// By candidate name rather than the bare literal, because the name on disk
/// depends on the channel. A preview daemon that piped a pane into the release
/// `farcoolerd` standing beside it in `~/.local/bin` would hand this pane's
/// bytes to a daemon on the other side of the isolation — and the fanout
/// socket it then bound would be one this daemon never looks at.
pub fn fanout_binary() -> Option<std::path::PathBuf> {
    fanout_binary_beside(&std::env::current_exe().ok()?)
}

/// capture-pane separates lines with a bare LF. To a terminal that is line feed
/// WITHOUT carriage return, so every line starts where the previous one ended
/// and the capture arrives as a staircase. The live pipe does not need this
/// because a pty already emits CRLF.
fn normalized(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// A pane's retained history, as the bytes that put it into a fresh emulator
/// ABOVE whatever screen is written next. Empty when there is nothing to write.
///
/// Shared rather than written twice. `replay` writes this, then the clear, then
/// the screen, all down one stream; `Runtime::history` hands the identical bytes
/// to a client that is polling and writes those three things itself. Every rule
/// below was learned from a bug that reached a screen, and a second copy of them
/// is a second place for one to be quietly dropped.
///
/// NOT the clear. `\x1b[H\x1b[2J` belongs between this and the screen, and it is
/// the caller that knows whether a screen is coming — a polling client that was
/// told `unchanged` has its screen already and must not have it erased.
fn history_bytes(
    modes: Option<farcooler_tmux::windows::PaneModes>,
    scrollback: Option<&str>,
) -> Vec<u8> {
    // Only on the primary screen, and only when tmux said so rather than merely
    // failing to say otherwise. An alternate screen has no history of its own,
    // so tmux answers that question with a copy of the visible screen — which
    // written here would sit directly above itself, and would still be sitting
    // there, stale, after the program exited the alternate screen.
    if !matches!(modes, Some(m) if !m.alternate_screen) {
        return Vec::new();
    }
    let Some(history) = scrollback.map(str::trim_end).filter(|h| !h.is_empty()) else {
        return Vec::new();
    };

    let mut out = normalized(history).into_bytes();
    // A trailing newline HERE, unlike after the screen. These lines belong above
    // the screen, so the last of them has to be finished before the clear that
    // pushes them all into history.
    //
    // And a reset with it. `capture-pane -e` states the color a line starts in
    // and leaves it set, because the line after it states its own — but the last
    // line of the history has no line after it, only the clear, and a clear
    // paints every cell it erases in whatever background is current. Left off,
    // opening a pane whose scrollback happened to end mid-color painted the
    // whole screen in it.
    out.extend_from_slice(b"\x1b[m\r\n");
    out
}

/// The bytes that put a fresh emulator into the state a pane is already in.
///
/// Split out from `stream` because it is the whole of what a client sees when
/// it opens a pane, and none of it needs tmux to be running to check.
///
/// Every part is optional and every part is skipped rather than guessed at: a
/// tmux that could not answer leaves that piece out, which costs the client one
/// property, where inventing a value would cost it the truth about all of them.
fn replay(
    modes: Option<farcooler_tmux::windows::PaneModes>,
    scrollback: Option<&str>,
    screen: Option<&str>,
    cursor: Option<(u32, u32)>,
) -> Vec<u8> {
    let mut out = Vec::new();

    // The modes first, because a capture is contents and modes are not
    // contents. Whether the program wants the mouse, whether it is on the
    // alternate screen, whether an arrow key should send an application
    // sequence — all of it was decided by escape sequences the program sent
    // once, before any of today's clients existed. A client that replays
    // only the screen is therefore wrong about every one of them, which is
    // what made a full-screen program's own scroll area dead on both the
    // Mac and the phone: the emulator believed no one wanted the wheel, so
    // it scrolled a scrollback that an alternate screen does not have.
    //
    // Emitted before everything, because switching to the alternate screen is
    // what decides which screen the rest of this lands on.
    if let Some(modes) = modes {
        out.extend_from_slice(modes.restore_sequence().as_bytes());
    }

    // Then the scrollback, so the client opens able to scroll back. See
    // `history_bytes`, which is the same assembly a polling client gets when it
    // asks for scrollback without a stream to carry it.
    out.extend_from_slice(&history_bytes(modes, scrollback));

    if let Some(screen) = screen {
        // Home the cursor and clear, so the replay paints a clean screen.
        //
        // The clear is also what turns the scrollback just written into
        // scrollback: erasing the display scrolls what was on it into history
        // rather than discarding it, which is the behavior that lets the two
        // captures be replayed one after the other and add up to the whole
        // pane.
        out.extend_from_slice(b"\x1b[H\x1b[2J");
        out.extend_from_slice(normalized(screen.trim_end()).as_bytes());

        // No trailing newline.
        //
        // The capture is exactly as many lines as the screen is tall
        // whenever the program fills it — which a full-screen agent always
        // does. One more line feed at the bottom row scrolls the whole
        // screen up by one: the top line is pushed into history, everything
        // appears one row too high, and the caret is left on a blank bottom
        // row. Both symptoms, one newline.

        // A captured screen is text and carries no cursor, so tmux was
        // asked where it actually is. Without this the caret sits wherever
        // the last replayed character ended, which is the bottom-left
        // corner, not the prompt the user is typing into.
        if let Some((column, row)) = cursor {
            // The wire format is one-based.
            out.extend_from_slice(format!("\x1b[{};{}H", row + 1, column + 1).as_bytes());
        }
    }

    out
}

#[cfg(test)]
mod replay_tests {
    use super::replay;
    use farcooler_tmux::windows::PaneModes;
    use farcooler_vt::Terminal;

    fn primary() -> PaneModes {
        PaneModes { cursor_visible: true, wrap: true, ..Default::default() }
    }

    fn alternate() -> PaneModes {
        PaneModes {
            alternate_screen: true,
            mouse_any: true,
            mouse_sgr: true,
            cursor_visible: true,
            wrap: true,
            ..Default::default()
        }
    }

    /// Replay into a real emulator, because the property under test is what the
    /// client ends up holding, not which bytes were written.
    fn opened(bytes: &[u8], rows: u16) -> Terminal {
        let mut t = Terminal::new(80, rows);
        t.feed(bytes);
        t
    }

    fn row(t: &Terminal, index: usize) -> String {
        let snapshot = farcooler_vt::grid::snapshot(t);
        snapshot.rows[index].cells.iter().map(|c| c.ch).collect::<String>().trim_end().to_string()
    }

    fn history_size(t: &Terminal) -> u32 {
        farcooler_vt::grid::snapshot(t).history_size
    }

    /// The one that matters. A pane that has been running for an hour opens
    /// with the hour in it.
    ///
    /// This used to replay the visible screen and nothing else, so a client
    /// opened with an empty history and the wheel had one line to move through
    /// — the line `ESC[2J` pushes — before it stopped. It read as a broken
    /// scroll, and it was a missing capture.
    #[test]
    fn a_pane_opens_with_the_scrollback_it_already_had() {
        let history: Vec<String> = (1..=40).map(|i| format!("old{i}")).collect();
        let screen: Vec<String> = (1..=10).map(|i| format!("now{i}")).collect();
        let bytes = replay(
            Some(primary()),
            Some(&history.join("\n")),
            Some(&screen.join("\n")),
            Some((0, 9)),
        );

        let mut t = opened(&bytes, 10);
        assert_eq!(history_size(&t), 40, "every retained line, above the screen");
        assert_eq!(row(&t, 0), "now1", "the live screen is still the live screen");

        t.scroll(40);
        assert_eq!(row(&t, 0), "old1", "scrolling back reaches the oldest line tmux kept");
    }

    /// The scrollback is not the screen, and must not arrive as both.
    #[test]
    fn the_visible_screen_is_not_replayed_twice() {
        let bytes = replay(Some(primary()), Some("old1\nold2"), Some("now1\nnow2"), Some((0, 1)));
        let mut t = opened(&bytes, 2);
        t.scroll(2);
        assert_eq!(row(&t, 0), "old1");
        assert_eq!(row(&t, 1), "old2", "not a second copy of the screen");
    }

    /// An alternate screen has no history, and tmux answers that question with
    /// the visible screen. Replaying it would put a copy of a TUI above itself.
    #[test]
    fn a_full_screen_program_gets_no_scrollback() {
        let bytes =
            replay(Some(alternate()), Some("copy1\ncopy2"), Some("tui1\ntui2"), Some((0, 1)));
        let t = opened(&bytes, 2);
        assert_eq!(history_size(&t), 0, "an alternate screen has nothing above it");
        assert_eq!(row(&t, 0), "tui1");
    }

    /// Modes are what says which screen this is, so without them the scrollback
    /// is not replayed at all. Guessing primary would be right most of the time
    /// and would put a duplicate TUI into the history the rest of it.
    #[test]
    fn unknown_modes_replay_no_scrollback() {
        let offered = replay(None, Some("old1\nold2"), Some("now1\nnow2"), Some((0, 1)));
        let withheld = replay(None, None, Some("now1\nnow2"), Some((0, 1)));
        assert_eq!(offered, withheld);
    }

    /// A pane with nothing above its screen answers with one empty line, and
    /// replaying that would shift the screen down by a row it does not have.
    #[test]
    fn an_empty_scrollback_writes_nothing() {
        let with = replay(Some(primary()), Some("\n"), Some("now1\nnow2"), Some((0, 1)));
        let without = replay(Some(primary()), None, Some("now1\nnow2"), Some((0, 1)));
        assert_eq!(with, without);
    }

    /// `capture-pane -e` writes the color a line starts in and leaves it set,
    /// because the next line it writes will state its own. The last line of the
    /// scrollback has no next line — the clear comes instead, and a clear paints
    /// every cell it erases in whatever background is current.
    #[test]
    fn a_color_left_on_by_the_scrollback_does_not_paint_the_screen() {
        let default = farcooler_vt::grid::Palette::default().background;
        let bytes = replay(
            Some(primary()),
            Some("\x1b[41mred and never turned off"),
            Some("now"),
            Some((0, 0)),
        );
        let t = opened(&bytes, 3);
        let snapshot = farcooler_vt::grid::snapshot(&t);
        let blank = &snapshot.rows[2].cells[0];
        assert_eq!(blank.bg, default, "the cleared screen is not still wearing the last line");
    }

    /// The cursor lands where tmux said, not where the last replayed byte did.
    #[test]
    fn the_caret_survives_the_scrollback() {
        let bytes = replay(Some(primary()), Some("old1"), Some("now1\nnow2"), Some((3, 0)));
        let t = opened(&bytes, 2);
        let snapshot = farcooler_vt::grid::snapshot(&t);
        assert_eq!((snapshot.cursor_column, snapshot.cursor_row), (3, 0));
    }
}

#[cfg(test)]
mod history_bytes_tests {
    use super::history_bytes;
    use farcooler_tmux::windows::PaneModes;

    fn primary() -> PaneModes {
        PaneModes { cursor_visible: true, wrap: true, ..Default::default() }
    }

    fn alternate() -> PaneModes {
        PaneModes { alternate_screen: true, cursor_visible: true, wrap: true, ..Default::default() }
    }

    /// On the bytes, not on an emulator, unlike `replay_tests` above. These are
    /// handed to a client that writes them itself, between a clear this code
    /// does not emit and a screen it never sees, so what is under test is the
    /// bytes leaving here rather than the picture some particular caller builds
    /// from them.
    #[test]
    fn a_bare_line_feed_becomes_a_carriage_return_pair() {
        // `capture-pane` separates lines with a bare LF, which to a terminal is
        // a line feed without a carriage return: fed as captured, every line
        // starts where the last one ended and the history arrives as a
        // staircase running off the right edge.
        let out = history_bytes(Some(primary()), Some("one\ntwo\nthree"));
        let text = String::from_utf8(out).expect("utf-8");
        assert_eq!(text, "one\r\ntwo\r\nthree\x1b[m\r\n");
    }

    /// The last line is finished, and finished with the color turned off.
    ///
    /// The newline because these lines belong ABOVE the screen, so the last of
    /// them has to end before the clear that pushes them into history. The reset
    /// because `capture-pane -e` leaves the last line's color set — the line
    /// after it would have stated its own, and there is no line after it — and
    /// the caller's clear paints every cell it erases in whatever background is
    /// current.
    #[test]
    fn the_history_ends_reset_and_finished() {
        let out = history_bytes(Some(primary()), Some("\x1b[41mred and never turned off"));
        assert!(out.ends_with(b"\x1b[m\r\n"), "{}", String::from_utf8_lossy(&out));
    }

    /// An alternate screen has no history of its own, so tmux answers the
    /// question with a copy of the visible screen. Sent, it would sit directly
    /// above itself — and would still be sitting there, stale, after the program
    /// left the alternate screen.
    #[test]
    fn a_full_screen_program_has_no_history_to_send() {
        assert!(history_bytes(Some(alternate()), Some("copy1\ncopy2")).is_empty());
    }

    /// And only when tmux SAID which screen it is. A pane whose modes could not
    /// be read is a pane that might be on the alternate screen, and the cost of
    /// guessing wrong is the duplicate above.
    #[test]
    fn modes_that_could_not_be_read_send_nothing() {
        assert!(history_bytes(None, Some("old1\nold2")).is_empty());
    }

    /// A pane that has not scrolled yet. tmux answers with a run of newlines
    /// rather than nothing, and sent as-is they are blank lines a client would
    /// have to scroll up through to reach a scrollback that is not there.
    #[test]
    fn a_pane_with_nothing_behind_it_sends_nothing() {
        assert!(history_bytes(Some(primary()), Some("\n\n\n")).is_empty());
        assert!(history_bytes(Some(primary()), None).is_empty());
    }

    /// The clear is the caller's. `replay` writes one because a screen follows
    /// it down the same stream; a polling client told `unchanged` already holds
    /// its screen and must not have it erased by the scrollback it asked for.
    #[test]
    fn the_clear_is_not_in_here() {
        let out = history_bytes(Some(primary()), Some("old1\nold2"));
        assert!(
            !out.windows(4).any(|w| w == b"\x1b[2J".as_slice()),
            "the history erased a screen it does not own"
        );
    }
}

/// The choice `fanout_binary` makes, given the executable it is made from, so
/// it can be tested without one.
fn fanout_binary_beside(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    let candidates = farcooler_protocol::CHANNEL.daemon_binary_candidates();
    if exe.file_name().is_some_and(|n| candidates.iter().any(|c| n == *c)) {
        return Some(exe.to_path_buf());
    }
    let dir = exe.parent()?;
    candidates.iter().map(|name| dir.join(name)).find(|p| p.exists())
}

#[cfg(test)]
mod fanout_binary_tests {
    use super::fanout_binary_beside;
    use farcooler_protocol::CHANNEL;

    /// A `~/.local/bin` with two channels installed in it holds two daemons,
    /// and the pane has to go to this one. The other one's fanout listens on a
    /// socket this daemon never subscribes to, so the stream would time out
    /// after a second and exit — which a client cannot tell apart from a pane
    /// that finished.
    #[test]
    fn the_pane_goes_to_this_channels_daemon() {
        let dir = tempfile::tempdir().unwrap();
        for name in CHANNEL.daemon_binary_candidates() {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        // Asked from the CLI, which is where this matters: the CLI opens a
        // Runtime of its own for `terminal stream` and must not pipe the pane
        // into itself.
        let cli = dir.path().join(CHANNEL.cli_binary_name());
        assert_eq!(
            fanout_binary_beside(&cli),
            Some(dir.path().join(CHANNEL.daemon_binary_name()))
        );
    }

    /// Cargo renames nothing, so a checkout still finds what it just built.
    #[test]
    fn a_cargo_target_directory_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("farcoolerd"), b"").unwrap();
        assert_eq!(
            fanout_binary_beside(&dir.path().join("farcooler")),
            Some(dir.path().join("farcoolerd"))
        );
    }

    /// The daemon asking about itself answers with itself, without touching
    /// the filesystem — `--fanout` is its own flag.
    #[test]
    fn the_daemon_is_its_own_fanout() {
        let exe = std::path::Path::new("/nowhere").join(CHANNEL.daemon_binary_name());
        assert_eq!(fanout_binary_beside(&exe), Some(exe));
    }
}

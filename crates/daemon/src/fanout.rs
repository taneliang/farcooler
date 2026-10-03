//! One pane's output, to as many watchers as ask for it.
//!
//! tmux allows exactly one `pipe-pane` per pane. Every watcher used to start
//! its own, so the second one silently stole the first one's output: tmux
//! replaced the pipe, the first watcher's fifo went quiet, and — because it
//! held a write handle to keep the fifo open — it never saw end-of-stream
//! either. It just stopped, forever, with no error anywhere. Two clients
//! looking at one terminal is not an edge case for a tool whose entire premise
//! is a fleet you check from wherever you are, and "the Mac and the phone
//! cannot watch the same pane" is the kind of failure that reads as the whole
//! product being broken.
//!
//! So the pipe is started once and its bytes are handed to everyone. tmux pipes
//! into `farcoolerd --fanout <pane>`, which listens on a unix socket named for
//! that pane; every watcher connects to it and gets the same bytes.
//!
//! Deliberately a process rather than something the daemon owns:
//!
//! - `farcoolerd --stream` runs over ssh with no daemon necessarily running,
//!   and making streaming depend on one would make a phone's terminal fail for
//!   a reason that has nothing to do with the phone.
//! - tmux already manages this process's lifetime perfectly. It starts when the
//!   pipe starts and dies when the pane does, which is exactly when the bytes
//!   stop being interesting.
//!
//! A stale socket file is harmless: connecting to one whose owner is gone fails
//! with a refusal, which is the same signal as no socket at all, and both mean
//! "start a fanout".

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

/// How long a fanout with nobody listening waits before giving up.
///
/// Not zero, because there is always a gap between tmux starting this process
/// and the watcher that caused it connecting. Not long, because every byte the
/// pane writes while this runs costs tmux a write to a pipe nobody is reading.
const IDLE_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// How much output a single slow watcher may fall behind before it is dropped.
///
/// Dropped rather than stalled: the bytes are a terminal's, so a watcher that
/// misses some of them has a corrupt screen, not a late one, and the honest
/// repair is to disconnect it and let it re-attach onto a fresh replay. The
/// alternative — making everyone wait for the slowest — would let one phone on
/// a bad network stall the pane's output for the Mac sitting next to it.
const BACKLOG: usize = 1024;

/// Where a pane's fanout listens.
///
/// Named for the pane rather than the terminal, because the pipe belongs to
/// the pane: a terminal that gets restarted is a new pane, and its watchers
/// must not be handed the old one's bytes.
///
/// And named for the INSTALL as well as the pane, because a pane number is
/// only unique within one tmux server. Every server numbers its panes from
/// `%0`, so two daemons on one host — a stable install beside a canary, or
/// a local build beside either — both had a `%0`, and by number alone both
/// resolved to one socket here. The second daemon did not fail: it connected,
/// to the first one's fanout, and so never started a pipe of its own. It then
/// read a stranger's pane forever. What a person saw was a terminal whose
/// typing never appeared until something else forced a redraw.
///
/// The install is passed in rather than resolved here, and that is the point.
/// The subscriber is the daemon; the server is a process tmux spawns from the
/// pipe command. If each worked its own path out and they ever disagreed —
/// a different `FARCOOLER_HOME`, a different channel — they would never meet,
/// which is a quieter version of the same bug. One side decides and tells the
/// other.
pub fn socket_path(install: &str, pane_id: &str) -> PathBuf {
    // Stripped, so `%17` and `17` name the same socket. That is not cosmetic:
    // tmux expands `%` when it runs the pipe command, so the fanout is started
    // with the bare number while the watcher subscribing to it holds the whole
    // id. Both have to arrive at the same path or they never meet.
    let name = pane_id.trim_start_matches('%');
    // Short, and in the temp directory rather than beside the daemon's other
    // state: a unix socket address is 104 bytes on macOS, and the runtime
    // directory's own path spends most of that before a filename is added.
    //
    // The TAIL of the id, not the head. An install id is a v7 uuid, whose
    // leading 48 bits are a millisecond timestamp — two installs created on one
    // host minutes apart share their first eight characters, which is
    // exactly the case this whole function exists to separate. Observed:
    // `01a00ce67d5e…` and `01a00ce67e61…`. The tail is the random half.
    let install = install.trim_start_matches("farcooler-");
    let install: String =
        install.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect();
    std::env::temp_dir().join(format!("farcooler-pane-{install}-{name}.sock"))
}

/// Connect to a pane's fanout, if one is running.
pub async fn subscribe(install: &str, pane_id: &str) -> Option<UnixStream> {
    UnixStream::connect(socket_path(install, pane_id)).await.ok()
}

/// The command tmux pipes a pane into, to start its fanout.
///
/// One function for the daemon and the tests, so a test of the fanout runs
/// the same command a pane does: a quoting slip here would otherwise leave
/// every stream without sizes and every test still green.
///
/// The pane NUMBER, not the pane id, because tmux expands this command as a
/// format string before running it and `%` starts an expansion there. A pane
/// id is `%0`, so passing one whole handed tmux an escape sequence: `%15`
/// arrived as `15` by luck, and `%0` arrived as an environment variable's
/// contents. The fanout then listened on a socket named after nonsense, the
/// watcher that started it could never connect, and after a second of trying
/// the stream gave up and exited — which a client cannot tell apart from a
/// pane that finished. The socket name strips `%` on both sides, so the
/// number is the whole id.
///
/// The install goes with it for the same reason the pane number does: the
/// fanout has to bind the socket this daemon will look for, and only this
/// daemon knows which install it is. An id is hex, so tmux has nothing in it
/// to expand. It is also the tmux socket's name, which is how the fanout asks
/// tmux which tty is its pane's (see `PaneSize`).
///
/// `#{pane_tty}` IS meant for tmux to expand: it is the pane's terminal device
/// when the pipe starts, where the fanout reads the pane's size so the stream
/// can say what size its bytes were written for.
pub fn pipe_command(exe: &std::path::Path, pane_id: &str, install: &str) -> String {
    format!(
        "'{}' --fanout '{}' --install '{}' --tty '#{{pane_tty}}'",
        exe.display(),
        pane_id.trim_start_matches('%'),
        install,
    )
}

/// Read this process's stdin — which tmux has connected to a pane — and give
/// every byte to every watcher.
///
/// `tty` is the pane's terminal device when the pipe started, which is where
/// its size can be read. `None` — a pipe command written by a daemon that
/// predates size markers — serves the bytes alone, exactly as before, apart
/// from removing any marker a program printed.
pub async fn serve(install: &str, pane_id: &str, tty: Option<&str>) -> std::io::Result<()> {
    let path = socket_path(install, pane_id);
    // Last binder wins. Two watchers can race into starting a fanout each; the
    // second `pipe-pane` replaces the first, so the first process is about to
    // lose its stdin and exit anyway. Refusing to bind here would leave the
    // survivor without a socket.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let size = tty.map(|tty| PaneSize::of_pane(tty.into(), install, pane_id));
    serve_on(tokio::io::stdin(), listener, size).await
}

/// Where a fanout reads its pane's size.
///
/// The pane's size, so the stream can say what size its bytes were written
/// for. tmux's `pipe-pane` hands over raw program output and nothing else, so
/// when a pane is resized the program's repaint arrives looking exactly like
/// any other output — and it arrives first, ahead of any layout reply a client
/// could size its emulator from. A client that grew its grid when the reply
/// landed had already drawn the repaint into the old one: every row of it
/// wrapped, and the grow then reflowed the wreckage. See
/// `farcooler_vt::size_marker`.
///
/// Read from the pane's tty rather than asked of tmux: the kernel's window
/// size IS what tmux told the program, set at the moment it sent the SIGWINCH,
/// and reading it is one ioctl rather than a tmux process per read.
///
/// But which tty is the pane's can change under a running fanout, and that is
/// asked of tmux. `respawn-pane -k` — a pane switching between its terminal
/// and its chat — gives the pane a new pty and keeps the pipe, so the path
/// tmux expanded when the pipe started goes stale: closed, or reused by the
/// next terminal anyone opens, whose size would then be announced as this
/// pane's. So a size is trusted without asking only when it is the size
/// already announced. A different size, or a tty that cannot be read, is
/// checked against tmux's `#{pane_tty}` for this pane first, and the fanout
/// follows the pane to its new tty; if tmux cannot say, nothing is announced,
/// and a client falls back to its layout replies.
pub struct PaneSize {
    source: SizeSource,
}

enum SizeSource {
    /// A test's.
    Probe(Box<dyn Fn() -> Option<(u16, u16)> + Send + Sync>),
    /// A real pane: its tty as last confirmed, and how to ask tmux for it.
    Pane { tty: PathBuf, socket: String, pane: String },
}

impl PaneSize {
    pub fn new(probe: impl Fn() -> Option<(u16, u16)> + Send + Sync + 'static) -> Self {
        Self { source: SizeSource::Probe(Box::new(probe)) }
    }

    /// Pane `pane_id` (with or without its `%`) of the tmux server on socket
    /// `socket`, whose tty was `tty` when the pipe started.
    pub fn of_pane(tty: PathBuf, socket: &str, pane_id: &str) -> Self {
        let pane = format!("%{}", pane_id.trim_start_matches('%'));
        Self { source: SizeSource::Pane { tty, socket: socket.to_string(), pane } }
    }

    /// The pane's size if it can be trusted, given the size last announced.
    async fn read(&mut self, last: Option<(u16, u16)>) -> Option<(u16, u16)> {
        let (tty, socket, pane) = match &mut self.source {
            SizeSource::Probe(probe) => return probe(),
            SizeSource::Pane { tty, socket, pane } => (tty, socket, pane),
        };
        let seen = tty_size(tty);
        if seen.is_some() && seen == last {
            return seen;
        }
        // Changed, or unreadable: is this still the pane's tty?
        let owner = pane_tty(socket, pane).await?;
        if owner != *tty {
            let (from, to) = (tty.display(), owner.display());
            tracing::debug!(pane = %pane, %from, %to, "the pane has a new tty");
            *tty = owner;
        }
        tty_size(tty)
    }
}

/// The size of the terminal device at `path`.
///
/// Opened for each look and closed straight after, never held. A pty reports
/// end-of-file to tmux only once every handle on its other side is closed, so
/// a fanout keeping one open could stop tmux noticing that the pane's program
/// had exited — and the fanout itself only exits when the pane does.
/// `NOCTTY`, so opening a terminal can never make it this process's
/// controlling one.
fn tty_size(path: &std::path::Path) -> Option<(u16, u16)> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let fd = rustix::fs::open(path, flags, Mode::empty()).ok()?;
    let size = rustix::termios::tcgetwinsize(&fd).ok()?;
    (size.ws_col > 0 && size.ws_row > 0).then_some((size.ws_col, size.ws_row))
}

/// Which tty tmux says `pane` has now. One tmux process, so only asked when a
/// size changes or cannot be read — a resize, not a read.
async fn pane_tty(socket: &str, pane: &str) -> Option<PathBuf> {
    let tmux = farcooler_core::programs::find("tmux")?;
    let output = tokio::process::Command::new(tmux)
        .args(["-L", socket, "display-message", "-p", "-t", pane, "#{pane_tty}"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(1), output).await.ok()?.ok()?;
    let tty = String::from_utf8(output.stdout).ok()?;
    let tty = tty.trim();
    (output.status.success() && !tty.is_empty()).then(|| PathBuf::from(tty))
}

/// `farcooler_vt::size_marker`, written out rather than called.
///
/// The emulator is only a dev-dependency here, on purpose: the daemon never
/// parses a screen, and linking a terminal emulator into it to format one
/// string would make it one. The tests below compare every marker this sends
/// against the emulator's own, so the two cannot drift apart unnoticed.
fn size_marker(columns: u16, rows: u16) -> Vec<u8> {
    format!("\x1bP{}{columns};{rows}\x1b\\", MARKER_TAG).into_bytes()
}

/// Everything in a marker before its numbers.
const MARKER_TAG: &str = ">farcooler-size;";

/// Removes anything shaped like a size marker from bytes a program wrote.
///
/// Only the runner may say what size a pane is. A program can print the
/// marker's bytes as easily as any others — `farcooler terminal stream` run
/// inside a pane prints a stream of them, and so does `cat` of a saved one —
/// and a client that believed them would size itself to some other pane and
/// stay there. So the fanout passes every program byte through this before
/// putting its own markers in, and a stream sent to a client that did not ask
/// for sizes passes the runner's through it too.
///
/// Across reads, because a pipe splits wherever it likes. A partial match at
/// the end of a read is passed on rather than held (holding would delay a
/// trailing escape until the next write, which may be never); if the next read
/// completes it, a string terminator is written in place of the rest, which
/// ends the control string the client has started without a size in it.
///
/// `holding` is for the other side of the fanout, removing the RUNNER's
/// markers for a client that did not ask for them. There a partial match is
/// held until the next read instead: the fanout sends each marker whole, so
/// the rest is already on its way, and passing the start on would leave an
/// old client — or a person's terminal — an empty control string.
#[derive(Default)]
pub struct MarkerStrip {
    matched: usize,
    carried: bool,
    dropping: Option<Dropping>,
    hold: bool,
    held: Vec<u8>,
}

#[derive(Default)]
struct Dropping {
    length: usize,
    escape: bool,
}

/// The bytes that make a marker, up to its numbers.
const FORGED_PREFIX: &[u8] = b"\x1bP>farcooler-size";

/// How much of a marker-shaped string is dropped before giving up on finding
/// its terminator. Far longer than any real marker.
const FORGED_LIMIT: usize = 64;

impl MarkerStrip {
    /// A strip that holds a partial match across reads. See the type's docs.
    pub fn holding() -> Self {
        Self { hold: true, ..Self::default() }
    }

    pub fn strip(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        let mut held = std::mem::take(&mut self.held);
        for &byte in input {
            if let Some(dropping) = &mut self.dropping {
                dropping.length += 1;
                if dropping.escape && byte == b'\\' {
                    self.dropping = None;
                    continue;
                }
                dropping.escape = byte == 0x1b;
                if dropping.length > FORGED_LIMIT {
                    self.dropping = None;
                }
                continue;
            }
            if byte == FORGED_PREFIX[self.matched] {
                self.matched += 1;
                held.push(byte);
                if self.matched == FORGED_PREFIX.len() {
                    if self.carried {
                        out.extend_from_slice(b"\x1b\\");
                    }
                    held.clear();
                    self.matched = 0;
                    self.carried = false;
                    self.dropping = Some(Dropping::default());
                }
                continue;
            }
            out.append(&mut held);
            self.matched = 0;
            self.carried = false;
            // The prefix has one escape, at its start, so a mismatch can only
            // begin a new match on an escape.
            if byte == FORGED_PREFIX[0] {
                self.matched = 1;
                held.push(byte);
            } else {
                out.push(byte);
            }
        }
        if self.hold {
            self.held = held;
        } else if !held.is_empty() {
            out.append(&mut held);
            self.carried = true;
        }
        out
    }
}

/// Where in a program's output a size marker may go.
///
/// Only between whole things: an ESC that lands inside a CSI aborts it and its
/// remaining parameters print as text, and one inside a multi-byte UTF-8
/// character prints U+FFFD. Pipe reads are cut wherever the bytes happened to
/// be, so the fanout follows the output just closely enough to know whether it
/// is at ground — outside any escape, control string or character — after
/// each byte. A small DEC-style parser: escapes with intermediates, CSI, the
/// string-carrying OSC, DCS, SOS, PM and APC (ended by ST, BEL for OSC, or
/// CAN/SUB), and UTF-8.
#[derive(Default)]
struct Boundary {
    state: Parse,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Parse {
    #[default]
    Ground,
    /// Continuation bytes still owed by a UTF-8 character.
    Utf8(u8),
    Escape,
    EscapeIntermediate,
    Csi,
    /// OSC, DCS, SOS, PM or APC.
    String,
    StringEscape,
}

impl Boundary {
    fn at_ground(&self) -> bool {
        self.state == Parse::Ground
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.step(byte);
        }
    }

    /// Advance by one byte; true if the output is at ground after it.
    fn step(&mut self, byte: u8) -> bool {
        const ESC: u8 = 0x1b;
        const CAN: u8 = 0x18;
        const SUB: u8 = 0x1a;
        self.state = match (self.state, byte) {
            // Cancels whatever was in progress, everywhere.
            (_, CAN | SUB) => Parse::Ground,
            (Parse::Utf8(owed), 0x80..=0xbf) => {
                if owed > 1 { Parse::Utf8(owed - 1) } else { Parse::Ground }
            }
            // A character cut short: the byte starts afresh.
            (Parse::Utf8(_), _) => {
                self.state = Parse::Ground;
                return self.step(byte);
            }
            (Parse::Ground, ESC) => Parse::Escape,
            (Parse::Ground, 0xc2..=0xdf) => Parse::Utf8(1),
            (Parse::Ground, 0xe0..=0xef) => Parse::Utf8(2),
            (Parse::Ground, 0xf0..=0xf4) => Parse::Utf8(3),
            (Parse::Ground, _) => Parse::Ground,
            (Parse::Escape, b'[') => Parse::Csi,
            (Parse::Escape, b']' | b'P' | b'X' | b'^' | b'_') => Parse::String,
            (Parse::Escape | Parse::EscapeIntermediate, ESC) => Parse::Escape,
            (Parse::Escape | Parse::EscapeIntermediate, 0x20..=0x2f) => Parse::EscapeIntermediate,
            (Parse::Escape | Parse::EscapeIntermediate, 0x00..=0x1f) => self.state,
            (Parse::Escape | Parse::EscapeIntermediate, _) => Parse::Ground,
            (Parse::Csi, ESC) => Parse::Escape,
            (Parse::Csi, 0x40..=0x7e) => Parse::Ground,
            (Parse::Csi, _) => Parse::Csi,
            (Parse::String, ESC) => Parse::StringEscape,
            (Parse::String, 0x07) => Parse::Ground,
            (Parse::String, _) => Parse::String,
            (Parse::StringEscape, b'\\') => Parse::Ground,
            // An escape inside a string that is not its terminator starts a
            // new sequence, as it does in the client's parser.
            (Parse::StringEscape, _) => {
                self.state = Parse::Escape;
                return self.step(byte);
            }
        };
        self.at_ground()
    }
}

/// The part that has nothing to do with processes, so a test can drive it.
///
/// With a `size`, the pane's size is announced in the stream: to each watcher
/// as it arrives, and to everyone whenever it changes. A change is checked for
/// after every read, and the marker goes in before the bytes read — or, when
/// the output so far stopped inside an escape sequence or a character, at the
/// first point it is whole again (`Boundary`) — which is what puts it ahead of
/// the program's repaint: a program cannot answer a SIGWINCH it has not been
/// sent, and by the time its answer has been read the new size is already on
/// the tty.
///
/// Only after a read: a quiet pane costs nothing. A resize that nobody writes
/// after is not announced until somebody does, and a client covers that gap by
/// applying its own layout reply when no marker follows it (see the Mac's
/// `TerminalRenderView.streamSizesCore`).
///
/// What this cannot order is output already in flight when the pane was
/// resized: bytes the program wrote for the old size that tmux had read but not
/// yet passed down the pipe. They arrive in the same read as the repaint, or
/// before it, after the size has changed, so they are drawn at the new size.
/// On a grow that is harmless. On a shrink a full-width line among them wraps,
/// until the program's repaint for the new size, which follows, redraws over
/// it. No placement of the check narrows this: which bytes were written
/// before the resize is not something a pipe records.
pub async fn serve_on<R>(
    mut source: R,
    listener: UnixListener,
    size: Option<PaneSize>,
) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let (tx, _) = tokio::sync::broadcast::channel::<bytes::Bytes>(BACKLOG);
    let watchers = Arc::new(AtomicUsize::new(0));
    let size = size.map(|s| Arc::new(tokio::sync::Mutex::new(s)));
    let mut last_size = match &size {
        Some(size) => size.lock().await.read(None).await,
        None => None,
    };

    let accepting = tokio::spawn({
        let tx = tx.clone();
        let size = size.clone();
        let watchers = watchers.clone();
        async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let rx = tx.subscribe();
                // Subscribed first, then told the size, so a change between the
                // two is announced again on the channel rather than lost.
                //
                // Read afresh rather than taken from the last announcement: a
                // pane resized while quiet has not announced it yet, and this
                // watcher's client is about to trust this size over its own.
                let now = match &size {
                    Some(size) => size.lock().await.read(None).await,
                    None => None,
                };
                if let Some((columns, rows)) = now
                    && socket.write_all(&size_marker(columns, rows)).await.is_err()
                {
                    continue;
                }
                watchers.fetch_add(1, Ordering::Relaxed);
                let watchers = watchers.clone();
                tokio::spawn(async move {
                    feed(socket, rx).await;
                    watchers.fetch_sub(1, Ordering::Relaxed);
                });
            }
        }
    });

    // Nobody is watching and nobody has been for a while: tmux is writing this
    // pane's output into a pipe for no one. Exiting ends the pipe, and the next
    // watcher starts a new one.
    let mut idle = tokio::spawn({
        let watchers = watchers.clone();
        async move {
            let mut empty_since = Some(std::time::Instant::now());
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if watchers.load(Ordering::Relaxed) > 0 {
                    empty_since = None;
                    continue;
                }
                let since = *empty_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() >= IDLE_GRACE {
                    return;
                }
            }
        }
    });

    let mut strip = MarkerStrip::default();
    let mut boundary = Boundary::default();
    // A size waiting for the output to reach a boundary. See `Boundary`.
    let mut pending: Option<(u16, u16)> = None;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        tokio::select! {
            read = source.read(&mut buf) => match read {
                // The pane closed its pipe: tmux is done with us.
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    // A send with no receivers is not a failure. It is an
                    // ordinary moment between one watcher leaving and the next
                    // arriving, and the bytes are genuinely nobody's.
                    let mut output = strip.strip(&buf[..n]);
                    if let Some(size) = &size
                        && let Some(now) = size.lock().await.read(last_size).await
                        && Some(now) != last_size
                    {
                        last_size = Some(now);
                        pending = Some(now);
                    }
                    // Not inside a sequence or a character the program had
                    // started: the marker goes at the first byte boundary where
                    // the output is back at ground. What comes before it is the
                    // end of something written before the resize, so this is
                    // also the right order.
                    if let Some((columns, rows)) = pending {
                        let at = if boundary.at_ground() {
                            Some(0)
                        } else {
                            output.iter().position(|&b| boundary.step(b)).map(|i| i + 1)
                        };
                        if let Some(at) = at {
                            let tail = output.split_off(at);
                            boundary.feed(&tail);
                            if !output.is_empty() {
                                let _ = tx.send(bytes::Bytes::from(output));
                            }
                            let _ = tx.send(bytes::Bytes::from(size_marker(columns, rows)));
                            pending = None;
                            output = tail;
                        }
                    } else {
                        boundary.feed(&output);
                    }
                    if !output.is_empty() {
                        let _ = tx.send(bytes::Bytes::from(output));
                    }
                }
            },
            _ = &mut idle => break,
        }
    }

    accepting.abort();
    idle.abort();
    Ok(())
}

/// One watcher, until it stops reading or falls too far behind.
async fn feed(mut socket: UnixStream, mut rx: tokio::sync::broadcast::Receiver<bytes::Bytes>) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match rx.recv().await {
            Ok(chunk) => {
                if socket.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            // Too far behind to be shown a correct screen. Hanging up is what
            // tells the client to re-attach, which is the only way it gets one.
            Err(RecvError::Lagged(_)) => return,
            Err(RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two daemons on one host must not share a pane's fanout.
    ///
    /// Every tmux server numbers its panes from `%0`, so a second install's
    /// first pane has the same number as the first install's. Named by number
    /// alone, both resolved to one socket in the shared temp directory — and
    /// the loser did not fail. It CONNECTED, to the other daemon's fanout, so
    /// it never started a pipe of its own and sat reading a stranger's pane
    /// forever. Typing showed nothing until something forced a redraw.
    ///
    /// The module's own note that "a stale socket file is harmless" is still
    /// true and was never the problem: this socket's owner was alive.
    #[test]
    fn two_installs_do_not_share_a_pane_socket() {
        let one = socket_path("01a00995", "%0");
        let two = socket_path("01a00cb1", "%0");
        assert_ne!(one, two, "two installs collided on pane 0");
    }

    /// Two installs made minutes apart must still separate.
    ///
    /// These are real ids from two daemons started seconds apart. An install id
    /// is a v7 uuid and its leading 48 bits are a millisecond timestamp, so
    /// they agree for the first NINE characters. A short prefix of the id looks
    /// like it identifies an install and does not — the first version of this
    /// fix used one, and both daemons landed on the same socket again.
    #[test]
    fn installs_created_moments_apart_still_separate() {
        let one = socket_path("01a00ce67d5e7c0191bea16539c08d62", "%0");
        let two = socket_path("01a00ce67e617a8090a5f0300313b7f3", "%0");
        assert_ne!(one, two, "a timestamp prefix is not an identity");
    }

    /// A unix socket address is 104 bytes on macOS, and the whole reason this
    /// socket lives in the temp directory rather than beside the daemon's other
    /// state is that the runtime directory's path is long enough to threaten
    /// that. Adding the install to the NAME keeps it short; moving it into the
    /// runtime directory would not have.
    #[test]
    fn the_socket_path_stays_short_enough_to_bind() {
        let path = socket_path("01a00995fd2f7f238b65ac553bd23298", "%999");
        assert!(
            path.as_os_str().len() < 100,
            "{} is too long to bind as a unix socket",
            path.display()
        );
    }

    /// The whole point: two watchers, the same bytes.
    #[tokio::test]
    async fn every_watcher_gets_every_byte() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");

        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, None).await });

        let mut one = UnixStream::connect(&path).await.expect("first watcher");
        let mut two = UnixStream::connect(&path).await.expect("second watcher");

        // Both connections have to be accepted before the bytes are sent, or
        // this test would be asserting something about timing rather than
        // about fanout.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        writer.write_all(b"hello pane").await.expect("write");
        writer.flush().await.expect("flush");

        let mut a = [0u8; 10];
        let mut b = [0u8; 10];
        one.read_exact(&mut a).await.expect("first read");
        two.read_exact(&mut b).await.expect("second read");
        assert_eq!(&a, b"hello pane");
        assert_eq!(&b, b"hello pane");

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A watcher leaving must not take the other one's stream with it — the
    /// exact failure this module exists to remove, in its second form.
    #[tokio::test]
    async fn one_watcher_leaving_leaves_the_others_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");

        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, None).await });

        let one = UnixStream::connect(&path).await.expect("first watcher");
        let mut two = UnixStream::connect(&path).await.expect("second watcher");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        drop(one);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        writer.write_all(b"still here").await.expect("write");
        writer.flush().await.expect("flush");

        let mut b = [0u8; 10];
        two.read_exact(&mut b).await.expect("survivor read");
        assert_eq!(&b, b"still here");

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A pane size a test can change, as the fanout reads it.
    fn adjustable(columns: u16, rows: u16) -> (Arc<std::sync::Mutex<(u16, u16)>>, PaneSize) {
        let size = Arc::new(std::sync::Mutex::new((columns, rows)));
        let read = size.clone();
        let probe = PaneSize::new(move || Some(*read.lock().expect("size lock")));
        (size, probe)
    }

    /// Read exactly `expected.len()` bytes and compare, with a deadline so a
    /// missing marker fails instead of hanging.
    async fn expect_bytes(socket: &mut UnixStream, expected: &[u8]) {
        let mut got = vec![0u8; expected.len()];
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.read_exact(&mut got))
            .await
            .expect("the bytes never came")
            .expect("read");
        assert_eq!(String::from_utf8_lossy(&got), String::from_utf8_lossy(expected));
    }

    /// The bug the marker exists for, at the layer that can see it: the bytes a
    /// program writes after its pane is resized must arrive BEHIND the news of
    /// the resize, or the client paints them into a grid of the old size.
    #[tokio::test]
    async fn a_resize_is_announced_ahead_of_the_bytes_written_after_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let (size, probe) = adjustable(80, 24);

        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, Some(probe)).await });

        let mut watcher = UnixStream::connect(&path).await.expect("watcher");
        // A new watcher is told the size before anything else.
        expect_bytes(&mut watcher, &farcooler_vt::size_marker(80, 24)).await;

        writer.write_all(b"old").await.expect("write");
        writer.flush().await.expect("flush");
        expect_bytes(&mut watcher, b"old").await;

        *size.lock().expect("size lock") = (120, 30);
        writer.write_all(b"repaint").await.expect("write");
        writer.flush().await.expect("flush");
        let mut wanted = farcooler_vt::size_marker(120, 30);
        wanted.extend_from_slice(b"repaint");
        expect_bytes(&mut watcher, &wanted).await;

        // And only on a change: the same size again says nothing.
        writer.write_all(b"more").await.expect("write");
        writer.flush().await.expect("flush");
        expect_bytes(&mut watcher, b"more").await;

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A quiet pane is not polled, so a resize nobody writes after is not
    /// announced on the channel — but a watcher arriving afterwards is told
    /// the size as it is now, not as it was last announced: its client is
    /// about to trust that size over its own.
    #[tokio::test]
    async fn a_watcher_arriving_after_a_quiet_resize_is_told_the_size_now() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let (size, probe) = adjustable(80, 24);

        let (writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, Some(probe)).await });

        let mut first = UnixStream::connect(&path).await.expect("first watcher");
        expect_bytes(&mut first, &farcooler_vt::size_marker(80, 24)).await;
        *size.lock().expect("size lock") = (100, 40);
        let mut second = UnixStream::connect(&path).await.expect("second watcher");
        expect_bytes(&mut second, &farcooler_vt::size_marker(100, 40)).await;

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A marker a program prints never reaches a watcher: only the runner may
    /// say what size a pane is. Here the program prints one for a size the
    /// pane does not have, and the watcher sees the text around it and nothing
    /// else.
    #[tokio::test]
    async fn a_marker_a_program_prints_is_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let (_size, probe) = adjustable(80, 24);

        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, Some(probe)).await });

        let mut watcher = UnixStream::connect(&path).await.expect("watcher");
        expect_bytes(&mut watcher, &farcooler_vt::size_marker(80, 24)).await;

        let mut forged = b"before".to_vec();
        forged.extend_from_slice(&farcooler_vt::size_marker(300, 100));
        forged.extend_from_slice(b"after");
        writer.write_all(&forged).await.expect("write");
        writer.flush().await.expect("flush");
        expect_bytes(&mut watcher, b"beforeafter").await;

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A resize that lands while the program is halfway through an escape
    /// sequence or a UTF-8 character must not put the marker inside it: an
    /// ESC in the middle of a CSI aborts it, and the rest of its parameters
    /// print as text; in the middle of a character it prints U+FFFD. Pipe
    /// reads cut wherever they like, and a resize is when output is densest.
    #[tokio::test]
    async fn a_resize_mid_sequence_is_announced_at_the_next_boundary() {
        for (head, tail, shown) in [
            (&b"\x1b[3"[..], &b"8;5;1mX"[..], "X"),
            (&b"\xc3"[..], &b"\xa9Y"[..], "\u{e9}Y"),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("fanout.sock");
            let listener = UnixListener::bind(&path).expect("bind");
            let (size, probe) = adjustable(20, 4);

            let (mut writer, reader) = tokio::io::duplex(64 * 1024);
            let served = tokio::spawn(async move { serve_on(reader, listener, Some(probe)).await });

            let mut watcher = UnixStream::connect(&path).await.expect("watcher");
            let mut seen = farcooler_vt::size_marker(20, 4);
            expect_bytes(&mut watcher, &seen).await;

            writer.write_all(head).await.expect("write");
            writer.flush().await.expect("flush");
            expect_bytes(&mut watcher, head).await;
            seen.extend_from_slice(head);

            *size.lock().expect("size lock") = (40, 6);
            writer.write_all(tail).await.expect("write");
            writer.flush().await.expect("flush");
            let mut rest = vec![0u8; tail.len() + farcooler_vt::size_marker(40, 6).len()];
            tokio::time::timeout(std::time::Duration::from_secs(5), watcher.read_exact(&mut rest))
                .await
                .expect("the bytes never came")
                .expect("read");
            seen.extend_from_slice(&rest);

            let mut t = farcooler_vt::Terminal::new(20, 4);
            t.set_accept_stream_sizes(true);
            t.feed(&seen);
            assert_eq!((t.columns(), t.rows()), (40, 6), "{seen:?}");
            let row: String =
                farcooler_vt::grid::snapshot(&t).rows[0].cells.iter().map(|c| c.ch).collect();
            assert_eq!(row.trim_end(), shown, "the marker was spliced inside: {seen:?}");

            drop(writer);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
        }
    }

    /// Where the output is at ground, byte by byte, for the shapes a program
    /// actually writes.
    #[test]
    fn the_boundary_follows_sequences_strings_and_characters() {
        let cases: &[(&[u8], &[bool])] = &[
            (b"a", &[true]),
            (b"\x1b[1;2m", &[false, false, false, false, false, true]),
            (b"\x1b(B", &[false, false, true]),
            (b"\x1b]0;t\x07", &[false, false, false, false, false, true]),
            (b"\x1b]0;t\x1b\\", &[false, false, false, false, false, false, true]),
            (b"\x1bPq#\x1b\\", &[false, false, false, false, false, true]),
            ("é".as_bytes(), &[false, true]),
            ("─".as_bytes(), &[false, false, true]),
            (b"\x1b[3\x18", &[false, false, false, true]),
            (b"\x1b[3\x1b[m", &[false, false, false, false, false, true]),
        ];
        for (bytes, wanted) in cases {
            let mut boundary = Boundary::default();
            let got: Vec<bool> = bytes.iter().map(|&b| boundary.step(b)).collect();
            assert_eq!(&got, wanted, "{bytes:?}");
        }
    }

    /// However a pipe splits a printed marker, no size gets through — checked
    /// by what an emulator that trusts sizes ends up holding, which is the
    /// only thing that matters.
    #[test]
    fn a_printed_marker_split_anywhere_is_still_removed() {
        let mut printed = b"a".to_vec();
        printed.extend_from_slice(&farcooler_vt::size_marker(300, 100));
        printed.extend_from_slice(b"b");
        for split in 1..printed.len() {
            let mut strip = MarkerStrip::default();
            let mut out = strip.strip(&printed[..split]);
            out.extend(strip.strip(&printed[split..]));
            let mut t = farcooler_vt::Terminal::new(20, 4);
            t.set_accept_stream_sizes(true);
            t.feed(&out);
            assert_eq!((t.columns(), t.rows()), (20, 4), "split at {split}: {out:?}");
            let row: String =
                farcooler_vt::grid::snapshot(&t).rows[0].cells.iter().map(|c| c.ch).collect();
            assert_eq!(row.trim_end(), "ab", "split at {split}: {out:?}");
        }
    }

    /// Removing the runner's own markers for a client that did not ask for
    /// them leaves exactly the program's bytes, wherever a read split one:
    /// not even an empty control string gets through.
    #[test]
    fn a_holding_strip_leaves_exactly_the_program_bytes() {
        let mut stream = b"a\x1b[1mb".to_vec();
        stream.extend_from_slice(&farcooler_vt::size_marker(120, 30));
        stream.extend_from_slice("c─".as_bytes());
        let wanted = [&b"a\x1b[1mb"[..], "c─".as_bytes()].concat();
        for split in 1..stream.len() {
            let mut strip = MarkerStrip::holding();
            let mut out = strip.strip(&stream[..split]);
            out.extend(strip.strip(&stream[split..]));
            assert_eq!(out, wanted, "split at {split}");
        }
    }

    /// Everything that is not a marker passes untouched, including other
    /// control strings and escapes that start the same way.
    #[test]
    fn other_output_passes_through_the_strip_untouched() {
        let output: &[u8] = b"\x1b[31mred\x1b[0m \x1bP>far away\x1b\\ \x1bPq#0\x1b\\ \x1b\x1bP>f";
        let mut strip = MarkerStrip::default();
        assert_eq!(strip.strip(output), output);
    }

    /// The socket is named for the pane, so two panes cannot collide.
    #[test]
    fn each_pane_gets_its_own_socket() {
        assert_ne!(socket_path("01a00995", "%1"), socket_path("01a00995", "%2"));
        assert!(socket_path("01a00995", "%17").to_string_lossy().contains("17"));
    }

    /// The watcher holds `%17` and the fanout is started with `17`, because
    /// tmux ate the `%` on the way. They have to meet at one path.
    #[test]
    fn a_pane_id_and_its_number_name_the_same_socket() {
        assert_eq!(socket_path("01a00995", "%17"), socket_path("01a00995", "17"));
        assert_eq!(socket_path("01a00995", "%0"), socket_path("01a00995", "0"));
    }
}

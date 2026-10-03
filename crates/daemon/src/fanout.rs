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

/// Read this process's stdin — which tmux has connected to a pane — and give
/// every byte to every watcher.
///
/// `tty` is the pane's terminal device, which is where its size can be read.
/// `None` — a pipe command written by a daemon that predates size markers —
/// serves the bytes alone, exactly as before.
pub async fn serve(install: &str, pane_id: &str, tty: Option<&str>) -> std::io::Result<()> {
    let path = socket_path(install, pane_id);
    // Last binder wins. Two watchers can race into starting a fanout each; the
    // second `pipe-pane` replaces the first, so the first process is about to
    // lose its stdin and exit anyway. Refusing to bind here would leave the
    // survivor without a socket.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    let size = tty.map(|tty| PaneSize::of_tty(tty.into(), QUIET_SIZE_CHECK));
    serve_on(tokio::io::stdin(), listener, size).await
}

/// How often a fanout looks at its pane's size when the pane is quiet.
///
/// Only for a resize nobody writes anything after — a shell that does not
/// repaint its prompt on SIGWINCH. Every resize that is followed by output is
/// announced by the check after each read, which is what orders the marker
/// ahead of the repaint; this only bounds how long a silent one goes unsaid.
const QUIET_SIZE_CHECK: std::time::Duration = std::time::Duration::from_millis(100);

/// Where a fanout reads its pane's size, and how often to look when nothing
/// is being written.
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
pub struct PaneSize {
    probe: Box<dyn Fn() -> Option<(u16, u16)> + Send + Sync>,
    every: std::time::Duration,
}

impl PaneSize {
    pub fn new(
        probe: impl Fn() -> Option<(u16, u16)> + Send + Sync + 'static,
        every: std::time::Duration,
    ) -> Self {
        Self { probe: Box::new(probe), every }
    }

    /// The size of the terminal device at `path`.
    ///
    /// Opened for each look and closed straight after, never held. A pty
    /// reports end-of-file to tmux only once every handle on its other side is
    /// closed, so a fanout keeping one open could stop tmux noticing that the
    /// pane's program had exited — and the fanout itself only exits when the
    /// pane does. `NOCTTY`, so opening a terminal can never make it this
    /// process's controlling one.
    ///
    /// `every` is how often to look while the pane is quiet; `QUIET_SIZE_CHECK`
    /// outside tests.
    pub fn of_tty(path: PathBuf, every: std::time::Duration) -> Self {
        Self::new(
            move || {
                use rustix::fs::{Mode, OFlags};
                let flags = OFlags::RDONLY | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC;
                let fd = rustix::fs::open(&path, flags, Mode::empty()).ok()?;
                let size = rustix::termios::tcgetwinsize(&fd).ok()?;
                (size.ws_col > 0 && size.ws_row > 0).then_some((size.ws_col, size.ws_row))
            },
            every,
        )
    }

    fn read(&self) -> Option<(u16, u16)> {
        (self.probe)()
    }
}

/// The marker for a size that changed since `last`, remembering it.
fn announce(size: Option<&PaneSize>, last: &mut Option<(u16, u16)>) -> Option<bytes::Bytes> {
    let now = size?.read()?;
    if *last == Some(now) {
        return None;
    }
    *last = Some(now);
    Some(bytes::Bytes::from(size_marker(now.0, now.1)))
}

/// `farcooler_vt::size_marker`, written out rather than called.
///
/// The emulator is only a dev-dependency here, on purpose: the daemon never
/// parses a screen, and linking a terminal emulator into it to format one
/// string would make it one. The tests below compare every marker this sends
/// against the emulator's own, so the two cannot drift apart unnoticed.
fn size_marker(columns: u16, rows: u16) -> Vec<u8> {
    format!("\x1bP>farcooler-size;{columns};{rows}\x1b\\").into_bytes()
}

/// The part that has nothing to do with processes, so a test can drive it.
///
/// With a `size`, the pane's size is announced in the stream: to each watcher
/// as it arrives, and to everyone whenever it changes. A change is checked for
/// after every read, BEFORE the bytes read are passed on, which is what puts
/// the marker ahead of the program's repaint: a program cannot answer a
/// SIGWINCH it has not been sent, and by the time its answer has been read the
/// new size is already on the tty. Bytes the program wrote just before the
/// resize can land behind the marker too, and then they are drawn at the new
/// size; that is the old-bytes-in-a-new-grid case a shrink always had, and it
/// mostly gets away with it.
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
    let size = size.map(Arc::new);
    let mut last_size = size.as_deref().and_then(PaneSize::read);
    let mut quiet = tokio::time::interval(
        size.as_deref().map_or(std::time::Duration::from_secs(3600), |s| s.every),
    );
    quiet.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

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
                // A watcher arrives after its client sized the pane and after a
                // replay captured at that size, so this usually confirms what it
                // knows. It is sent anyway because it is how a client learns
                // the stream speaks sizes at all — see
                // `farcooler_vt::Terminal::sized_by_stream`.
                if let Some(marker) = announce(size.as_deref(), &mut None)
                    && socket.write_all(&marker).await.is_err()
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
                    if let Some(marker) = announce(size.as_deref(), &mut last_size) {
                        let _ = tx.send(marker);
                    }
                    let _ = tx.send(bytes::Bytes::copy_from_slice(&buf[..n]));
                }
            },
            _ = quiet.tick() => {
                if let Some(marker) = announce(size.as_deref(), &mut last_size) {
                    let _ = tx.send(marker);
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
    fn adjustable(
        columns: u16,
        rows: u16,
        every: std::time::Duration,
    ) -> (Arc<std::sync::Mutex<(u16, u16)>>, PaneSize) {
        let size = Arc::new(std::sync::Mutex::new((columns, rows)));
        let read = size.clone();
        let probe = PaneSize::new(move || Some(*read.lock().expect("size lock")), every);
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
    ///
    /// The tick is an hour, so the only thing that can put the marker in front
    /// of the repaint is the check made after each read.
    #[tokio::test]
    async fn a_resize_is_announced_ahead_of_the_bytes_written_after_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let (size, probe) = adjustable(80, 24, std::time::Duration::from_secs(3600));

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

    /// A pane resized while its program says nothing is still announced: a
    /// client that has handed sizing to the stream would otherwise keep the old
    /// grid until the next byte.
    #[tokio::test]
    async fn a_resize_with_no_output_is_announced_anyway() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let (size, probe) = adjustable(80, 24, std::time::Duration::from_millis(20));

        let (writer, reader) = tokio::io::duplex(64 * 1024);
        let served = tokio::spawn(async move { serve_on(reader, listener, Some(probe)).await });

        let mut watcher = UnixStream::connect(&path).await.expect("watcher");
        expect_bytes(&mut watcher, &farcooler_vt::size_marker(80, 24)).await;
        *size.lock().expect("size lock") = (100, 40);
        expect_bytes(&mut watcher, &farcooler_vt::size_marker(100, 40)).await;

        drop(writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), served).await;
    }

    /// A private tmux server, killed however the test ends.
    struct ScratchTmux {
        tmux: PathBuf,
        socket: String,
    }

    impl ScratchTmux {
        fn run(&self, args: &[&str]) -> String {
            let out = std::process::Command::new(&self.tmux)
                .args(["-L", &self.socket, "-f", "/dev/null"])
                .args(args)
                .output()
                .expect("run tmux");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "tmux {args:?}: {stderr}");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
    }

    impl Drop for ScratchTmux {
        fn drop(&mut self) {
            let _ = std::process::Command::new(&self.tmux)
                .args(["-L", &self.socket, "kill-server"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    /// The ordering claim, against a real tmux and a real program: the marker
    /// for a resize reaches a watcher ahead of what the program printed in
    /// answer to it.
    ///
    /// The program prints `WINCH` when it gets the signal, which stands in for
    /// a full-screen program's repaint. The quiet check is an hour, so only the
    /// check after each read can put the marker in front of it — which is the
    /// one that has to, because a repaint is never quiet.
    #[tokio::test]
    async fn a_real_panes_resize_is_announced_before_its_programs_answer() {
        let tmux = farcooler_core::programs::find("tmux").expect("these tests need tmux");
        let dir = tempfile::tempdir().expect("tempdir");
        let server = ScratchTmux {
            tmux,
            socket: format!("fc-fanout-{}", uuid::Uuid::now_v7().simple()),
        };
        server.run(&[
            "new-session", "-d", "-s", "s", "-x", "80", "-y", "24",
            "sh -c 'trap \"echo WINCH\" WINCH; while :; do sleep 0.05; done'",
        ]);
        let tty = server.run(&["display-message", "-p", "-t", "s", "#{pane_tty}"]);

        let fifo = dir.path().join("pane.fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
        assert!(made.success());
        let source =
            tokio::net::unix::pipe::OpenOptions::new().open_receiver(&fifo).expect("open the fifo");
        server.run(&["pipe-pane", "-O", "-t", "s", &format!("cat > '{}'", fifo.display())]);

        let path = dir.path().join("fanout.sock");
        let listener = UnixListener::bind(&path).expect("bind");
        let size = PaneSize::of_tty(tty.into(), std::time::Duration::from_secs(3600));
        let served = tokio::spawn(async move { serve_on(source, listener, Some(size)).await });

        let mut watcher = UnixStream::connect(&path).await.expect("watcher");
        expect_bytes(&mut watcher, &farcooler_vt::size_marker(80, 24)).await;

        server.run(&["resize-window", "-t", "s", "-x", "120", "-y", "30"]);
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !String::from_utf8_lossy(&seen).contains("WINCH") {
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), watcher.read(&mut buf))
                .await
                .expect("the program never answered the resize")
                .expect("read");
            assert!(n > 0, "the fanout hung up");
            seen.extend_from_slice(&buf[..n]);
        }
        let seen = String::from_utf8_lossy(&seen).into_owned();
        let marker = String::from_utf8_lossy(&farcooler_vt::size_marker(120, 30)).into_owned();
        let at = seen.find(&marker).unwrap_or_else(|| panic!("no marker in {seen:?}"));
        let answer = seen.find("WINCH").expect("WINCH");
        assert!(at < answer, "the marker came after the answer: {seen:?}");

        served.abort();
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

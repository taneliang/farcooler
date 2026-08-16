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
/// `%0`, so two daemons on one machine — a stable install beside a canary, or
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
    // machine minutes apart share their first eight characters, which is
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
pub async fn serve(install: &str, pane_id: &str) -> std::io::Result<()> {
    let path = socket_path(install, pane_id);
    // Last binder wins. Two watchers can race into starting a fanout each; the
    // second `pipe-pane` replaces the first, so the first process is about to
    // lose its stdin and exit anyway. Refusing to bind here would leave the
    // survivor without a socket.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    serve_on(tokio::io::stdin(), listener).await
}

/// The part that has nothing to do with processes, so a test can drive it.
pub async fn serve_on<R>(mut source: R, listener: UnixListener) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let (tx, _) = tokio::sync::broadcast::channel::<bytes::Bytes>(BACKLOG);
    let watchers = Arc::new(AtomicUsize::new(0));

    let accepting = tokio::spawn({
        let tx = tx.clone();
        let watchers = watchers.clone();
        async move {
            while let Ok((socket, _)) = listener.accept().await {
                let rx = tx.subscribe();
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
                    let _ = tx.send(bytes::Bytes::copy_from_slice(&buf[..n]));
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

    /// Two daemons on one machine must not share a pane's fanout.
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
        let served = tokio::spawn(async move { serve_on(reader, listener).await });

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
        let served = tokio::spawn(async move { serve_on(reader, listener).await });

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

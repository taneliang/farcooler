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
//! into `overnightd --fanout <pane>`, which listens on a unix socket named for
//! that pane; every watcher connects to it and gets the same bytes.
//!
//! Deliberately a process rather than something the daemon owns:
//!
//! - `overnightd --stream` runs over ssh with no daemon necessarily running,
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
pub fn socket_path(pane_id: &str) -> PathBuf {
    // Pane ids are `%17`, and `%` is fine in a filename but noisy in the logs
    // and error messages this path shows up in.
    let name = pane_id.trim_start_matches('%');
    std::env::temp_dir().join(format!("overnight-pane-{name}.sock"))
}

/// Connect to a pane's fanout, if one is running.
pub async fn subscribe(pane_id: &str) -> Option<UnixStream> {
    UnixStream::connect(socket_path(pane_id)).await.ok()
}

/// Read this process's stdin — which tmux has connected to a pane — and give
/// every byte to every watcher.
pub async fn serve(pane_id: &str) -> std::io::Result<()> {
    let path = socket_path(pane_id);
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
        assert_ne!(socket_path("%1"), socket_path("%2"));
        assert!(socket_path("%17").to_string_lossy().contains("17"));
    }
}

//! A fanout takes its socket file down when it ends (ov-541), and only its own.

use super::BoundSocket;
use std::os::unix::net::UnixListener;

#[test]
fn a_fanouts_socket_is_removed_when_it_ends() {
    let dir = farcooler_core::scratch::dir("fanout-socket");
    let path = dir.join("p.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    drop(BoundSocket::of(path.clone()));
    assert!(!path.exists(), "the socket outlived its fanout");
}

#[test]
fn a_newer_fanouts_socket_is_left_alone() {
    let dir = farcooler_core::scratch::dir("fanout-socket");
    let path = dir.join("p.sock");
    let _first = UnixListener::bind(&path).unwrap();
    let older = BoundSocket::of(path.clone());
    // The second fanout for the pane removes the path and binds its own.
    std::fs::remove_file(&path).unwrap();
    let _second = UnixListener::bind(&path).unwrap();
    drop(older);
    assert!(path.exists(), "the older fanout took down the newer one's socket");
}

#[tokio::test]
async fn serve_holds_its_socket_while_the_pipe_is_open_and_removes_it_after() {
    // An install of its own, so no other test's socket is this path.
    let install = format!("test-{}", uuid::Uuid::now_v7().simple());
    let path = super::socket_path(&install, "%7");
    let (mut tmux, pipe) = tokio::io::duplex(64);
    let serving = {
        let install = install.clone();
        tokio::spawn(async move { super::serve_from(pipe, &install, "%7", None).await })
    };
    // Open: the socket is there, and stays there while serve reads.
    let mut seen = false;
    for _ in 0..300 {
        if path.exists() {
            seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(seen, "no socket at {} while the pipe was open", path.display());
    tokio::io::AsyncWriteExt::write_all(&mut tmux, b"x").await.unwrap();
    assert!(path.exists(), "the socket went away with the pipe still open");
    // Closed: tmux has nothing more to send.
    drop(tmux);
    serving.await.unwrap().unwrap();
    assert!(!path.exists(), "serve left {} behind", path.display());
}

/// A pipe that ends hands its watchers everything read before it ended
/// (ov-554). On tmux 3.8 it ends when the pane's program dies, so that is the
/// program's last output, and the fanout exits straight after: a feed still
/// writing when `serve` returned was cut off by the process ending.
///
/// The process is a runtime of its own on a thread, dropped as `serve`
/// returns, as `farcoolerd --fanout`'s is. More is sent than a socket holds,
/// and the watcher reads none of it until the pipe has ended.
#[test]
fn a_watcher_gets_everything_read_before_the_pipe_ended() {
    use std::io::{Read, Write};
    let install = format!("test-{}", uuid::Uuid::now_v7().simple());
    let path = super::socket_path(&install, "%9");
    let (pipe, source) = std::os::unix::net::UnixStream::pair().unwrap();
    let process = std::thread::spawn({
        let install = install.clone();
        move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async move {
                source.set_nonblocking(true).unwrap();
                let source = tokio::net::UnixStream::from_std(source).unwrap();
                super::serve_from(source, &install, "%9", None).await.unwrap();
            });
            // The process exits here, taking every task still running.
            drop(runtime);
        }
    });

    let mut watcher = None;
    for _ in 0..600 {
        if let Ok(socket) = std::os::unix::net::UnixStream::connect(&path) {
            watcher = Some(socket);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut watcher = watcher.unwrap_or_else(|| panic!("no fanout at {}", path.display()));
    // Subscribed once a byte sent after connecting comes back. Sent again
    // until one does: a connection is made before the fanout accepts it, and
    // a byte sent in between is nobody's.
    let mut pipe = pipe;
    watcher.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
    let subscribed = (0..600).any(|_| {
        pipe.write_all(b"x").unwrap();
        watcher.read_exact(&mut [0u8; 1]).is_ok()
    });
    assert!(subscribed, "the fanout never passed a byte on");
    watcher.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();

    // 2 MB: more than a socket holds, so a cut-off feed shows, and at most
    // 2 MB / 16 KB reads = 128 chunks (more if reads come smaller), well under
    // `BACKLOG`'s 1024, so the unread watcher never lags out instead. Lower
    // `BACKLOG` toward this and the test fails for that reason, not the drain.
    let last_words: Vec<u8> = (0..2 * 1024 * 1024).map(|i| b'a' + (i % 26) as u8).chain(*b"exit 3").collect();
    pipe.write_all(&last_words).unwrap();
    // Read from the moment the pipe ends, as a client would be.
    let reading = std::thread::spawn(move || {
        let mut got = Vec::new();
        watcher.read_to_end(&mut got).unwrap();
        got
    });
    drop(pipe);
    process.join().unwrap();
    let got = reading.join().unwrap();
    // Less any `x` still on its way when the first came back.
    let got = &got[got.iter().take_while(|&&b| b == b'x').count()..];
    assert_eq!(got.len(), last_words.len(), "the watcher was cut off");
    assert!(got == last_words.as_slice(), "the watcher got other bytes");
}

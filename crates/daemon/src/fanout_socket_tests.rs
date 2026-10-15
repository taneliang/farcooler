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

//! Writing to a `runner pipe` child that has already exited is an error, not
//! a signal (ov-800).
//!
//! A Rust binary starts with SIGPIPE ignored; this core is linked into the
//! Mac and iOS apps as a static library, so it inherits whatever the host
//! process has, and a host that never ignored it (a test runner, the Mac
//! suite's `swiftpm-testing-helper`) is killed by the first write to a child
//! that is gone. This test is its own process, and puts SIGPIPE back to its
//! default first, so it is the host that does not ignore it. The pipe is
//! macOS-only, like `farcooler_client::pipe` itself.
#![cfg(target_os = "macos")]

use farcooler_client::pipe::PipeStream;
use tokio::io::AsyncWriteExt as _;

#[tokio::test]
async fn a_pipe_to_a_dead_child_fails_instead_of_signaling() {
    // SAFETY: restoring the default disposition, before anything else runs.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("farcooler");
    // Exits at once without reading its stdin, as the Mac suite's stand-in does.
    std::fs::write(&shim, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&shim, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let argv = [shim.to_str().unwrap().to_string(), "runner".into(), "pipe".into(), "x".into()];
    let mut stream = PipeStream::spawn(&argv).unwrap();
    let mut failed = false;
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if stream.write_all(b"SSH-2.0-hello\n").await.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "writes to an exited child never failed");
}

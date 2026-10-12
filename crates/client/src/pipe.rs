//! A spawned `farcooler runner pipe <id>` as the byte stream SSH runs over.
//!
//! This is what a `ProxyCommand` is, done by the client core instead of by
//! `ssh`. The Mac app links this core without the tunnel archive (the archive
//! is 47 MB and the CLI already carries it), so the app cannot dial a tunnel
//! itself. The CLI can, and `~/.ssh/config` already names it for exactly this:
//! `ProxyCommand <cli> runner pipe <id>`. Spawning that same command and
//! speaking SSH over its stdin and stdout is the narrowest way to give the
//! conversation view the route the config gives `ssh`, with nothing about the
//! tunnel copied into the app.
//!
//! **Not a general ProxyCommand.** [`argv_is_a_runner_pipe`] is the only shape
//! the core will spawn, so a config line, a pasted JSON or a bug upstream can
//! name a program but never a different subcommand, and the id is held to the
//! same alphabet the CLI itself accepts.

use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Whether `argv` is `<path>/farcooler runner pipe <id>`, the one thing this
/// spawns: an absolute path to a program named `farcooler`, so a caller cannot
/// get `env`, `sh` or an interpreter run with `runner` as its first argument.
pub fn argv_is_a_runner_pipe(argv: &[String]) -> bool {
    let [program, runner, pipe, id] = argv else { return false };
    let path = std::path::Path::new(program);
    path.is_absolute()
        && path.file_name().is_some_and(|name| name == "farcooler")
        && runner == "runner"
        && pipe == "pipe"
        && !id.is_empty()
        && id.len() <= 64
        && !id.starts_with(['-', '.'])
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The child and its two pipes, as one duplex stream.
///
/// The child dies with the stream (`kill_on_drop`), so a session the app drops
/// leaves no `runner pipe` behind holding a tunnel open. Its stderr is
/// discarded: it is the CLI's own sentence for a terminal, and no raw error
/// string reaches a screen from here.
pub struct PipeStream {
    stdout: ChildStdout,
    stdin: ChildStdin,
    _child: Child,
}

impl PipeStream {
    pub fn spawn(argv: &[String]) -> std::io::Result<Self> {
        debug_assert!(argv_is_a_runner_pipe(argv));
        let mut command = Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // SAFETY: the closure runs between fork and exec and calls only
        // `fcntl`, which is async-signal-safe, and no allocator or lock.
        //
        // **Nothing above stderr is inherited.** Marked close-on-exec rather
        // than closed: std reports a failed exec to the parent over its own
        // descriptor, and closing that one would turn "no such program" into
        // a spawn that looks like it worked. Descriptors the app holds
        // (another child's pipe not yet spawned, the control socket) are not
        // close-on-exec, and a tunnel child lives for hours: it would hold a
        // pipe's write end open so its reader never sees EOF.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::getdtablesize().clamp(3, 65_536);
                for fd in 3..limit {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        let stdin = child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?;
        Ok(Self { stdout, stdin, _child: child })
    }
}

impl AsyncRead for PipeStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for PipeStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stdin).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn only_a_runner_pipe_with_a_plain_id_is_spawned() {
        assert!(argv_is_a_runner_pipe(&argv(&["/x/farcooler", "runner", "pipe", "fc-0123abcd"])));
        for bad in [
            argv(&["/usr/bin/env", "runner", "pipe", "x"]),
            argv(&["/bin/sh", "runner", "pipe", "x"]),
            argv(&["relative/farcooler", "runner", "pipe", "x"]),
            argv(&["farcooler", "runner", "pipe", "x"]),
            argv(&["/x/farcooler2", "runner", "pipe", "x"]),
            argv(&[]),
            argv(&["/x/farcooler", "runner", "pipe"]),
            argv(&["/x/farcooler", "runner", "pipe", "a", "b"]),
            argv(&["/x/farcooler", "runner", "store-runners", "a"]),
            argv(&["/x/farcooler", "daemon", "pipe", "a"]),
            argv(&["", "runner", "pipe", "a"]),
            argv(&["/x/farcooler", "runner", "pipe", ""]),
            argv(&["/x/farcooler", "runner", "pipe", "-o"]),
            argv(&["/x/farcooler", "runner", "pipe", ".hidden"]),
            argv(&["/x/farcooler", "runner", "pipe", "a b"]),
            argv(&["/x/farcooler", "runner", "pipe", "a;b"]),
            argv(&["/x/farcooler", "runner", "pipe", &"a".repeat(65)]),
        ] {
            assert!(!argv_is_a_runner_pipe(&bad), "{bad:?}");
        }
    }

    /// The stream is the child's stdio, both ways, and the child is not left
    /// behind when it drops. `cat` stands in for the CLI: it is the same shape
    /// (bytes in, bytes out), and the real one is the e2e's.
    #[tokio::test]
    async fn bytes_go_in_and_come_out_of_the_child() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("farcooler");
        std::fs::write(&shim, "#!/bin/sh\nexec /bin/cat\n").unwrap();
        std::fs::set_permissions(&shim, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let mut stream = PipeStream::spawn(&argv(&[shim.to_str().unwrap(), "runner", "pipe", "x"])).unwrap();
        stream.write_all(b"SSH-2.0-hello\n").await.unwrap();
        let mut back = [0u8; 14];
        stream.read_exact(&mut back).await.unwrap();
        assert_eq!(&back, b"SSH-2.0-hello\n");
    }

    /// A descriptor the app holds open is not the child's. The shim tries to
    /// duplicate the leaked descriptor onto stdout and says whether it could.
    #[tokio::test]
    async fn the_child_inherits_nothing_above_stderr() {
        use std::os::fd::AsRawFd as _;
        use tokio::io::AsyncReadExt as _;
        let dir = tempfile::tempdir().unwrap();
        // `File::open` is close-on-exec; clear it, as an `NSPipe` end is not.
        let leaked = std::fs::File::open(dir.path()).unwrap();
        let fd = leaked.as_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
        let shim = dir.path().join("farcooler");
        std::fs::write(&shim, format!("#!/bin/sh\nif {{ : >&{fd}; }} 2>/dev/null; then echo leaked; else echo closed; fi\n")).unwrap();
        std::fs::set_permissions(&shim, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let mut stream = PipeStream::spawn(&argv(&[shim.to_str().unwrap(), "runner", "pipe", "x"])).unwrap();
        let mut said = String::new();
        stream.read_to_string(&mut said).await.unwrap();
        assert_eq!(said.trim(), "closed", "fd {fd} reached the child");
    }

    #[tokio::test]
    async fn a_program_that_is_not_there_is_an_io_error() {
        assert!(PipeStream::spawn(&argv(&["/nonexistent/farcooler", "runner", "pipe", "x"])).is_err());
    }
}

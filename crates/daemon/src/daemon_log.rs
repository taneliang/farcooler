//! The daemon's own log file (ov-798).
//!
//! `farcoolerd` writes its warnings to stderr, and nothing that starts it keeps
//! stderr: `farcooler daemon ensure` hands the child `/dev/null`, and launchd
//! does the same for a job whose plist names no `StandardErrorPath`. So the
//! reason a paste was dropped, a tunnel did not start or the database was
//! refused used to be said once and lost. The daemon is the one place that is
//! true of both launch paths (and of a Linux runner, which has no app at all),
//! so it keeps its own log: a few small files, rotated by size, with stderr
//! itself pointed at the current one so a panic or an `eprintln!` lands there
//! too.
//!
//! What goes in is what the daemon already logs: reasons, ids and counts, no
//! message or prompt text and no credential (the tunnel token is logged as a
//! length). Some lines carry paths, branch names and a git command's own
//! stderr. This module only decides where those lines go.
//!
//! The bound is on what this module writes. Anything else holding stderr (a
//! panic, `eprintln!`, a child that inherited it) writes the same file
//! directly, so a timer checks the size too (`SWEEP_EVERY`); the log is bounded
//! to roughly its limit, not to the byte.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The current file's name; older ones are `farcoolerd.log.1`, `.2`.
pub const FILE_NAME: &str = "farcoolerd.log";

/// A file may reach this size before it is rotated out.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// How often the file's size is checked when nothing is going through the
/// writer, so a flood from a panic or a child cannot grow it unseen.
pub const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// Files kept, the current one included. With [`MAX_BYTES`] this bounds the
/// log to about six megabytes.
pub const KEEP: usize = 3;

/// Where this daemon's log lives.
///
/// An isolated home (`FARCOOLER_HOME`: a test, a scratch daemon) keeps its log
/// inside that home, so nothing outside it is written. On a Mac it is the
/// folder Console and `open ~/Library/Logs` already show, named for the
/// channel's bundle id; elsewhere a `logs` folder in the runtime directory.
pub fn log_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("FARCOOLER_HOME") {
        return Some(PathBuf::from(home).join("logs"));
    }
    let runtime = crate::paths::default_runtime_dir_for(farcooler_protocol::CHANNEL).ok()?;
    if cfg!(target_os = "macos") {
        let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
        // `com.farcooler.FarCooler`, `...FarCoolerCanary`: the data folder's own name.
        return Some(home.join("Library/Logs").join(runtime.file_name()?));
    }
    Some(runtime.join("logs"))
}

/// A size-bounded rotating log file, usable as a `tracing` writer.
#[derive(Clone)]
pub struct RotatingLog(Arc<Inner>);

struct Inner {
    dir: PathBuf,
    max: u64,
    keep: usize,
    /// Point stderr at the new file after every rotation.
    redirect_stderr: bool,
    file: Mutex<File>,
}

impl RotatingLog {
    /// Open (creating the folder, mode 0700) and rotate at once if the file is
    /// already full, so a daemon that restarts in a loop cannot grow it.
    ///
    /// A symlink where the folder or the file should be is refused rather than
    /// followed, and a folder or file left with looser modes is tightened.
    /// With `sweep` set, a thread checks the size on that interval.
    pub fn open(
        dir: &Path,
        max: u64,
        keep: usize,
        redirect_stderr: bool,
        sweep: Option<std::time::Duration>,
    ) -> io::Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let meta = std::fs::symlink_metadata(dir)?;
        if !meta.is_dir() {
            return Err(io::Error::other("the log folder is a symlink or not a folder"));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let log = Self(Arc::new(Inner {
            dir: dir.to_path_buf(),
            max,
            keep: keep.max(1),
            redirect_stderr,
            file: Mutex::new(open_append(&dir.join(FILE_NAME))?),
        }));
        {
            let mut file = log.0.file.lock().unwrap_or_else(|e| e.into_inner());
            if file.metadata()?.len() >= max {
                log.0.rotate(&mut file)?;
            }
            log.0.redirect(&file);
        }
        if let Some(every) = sweep {
            let weak = Arc::downgrade(&log.0);
            std::thread::Builder::new().name("daemon-log-sweep".into()).spawn(move || {
                while let Some(inner) = weak.upgrade() {
                    let _ = inner.rotate_if_full(0);
                    drop(inner);
                    std::thread::sleep(every);
                }
            })?;
        }
        Ok(log)
    }
}

impl Inner {
    /// Shift `.N-1` out, every older file up one, the current one to `.1`, and
    /// start a fresh current file.
    fn rotate(&self, file: &mut File) -> io::Result<()> {
        let current = self.dir.join(FILE_NAME);
        let numbered = |n: usize| self.dir.join(format!("{FILE_NAME}.{n}"));
        if self.keep > 1 {
            let _ = std::fs::remove_file(numbered(self.keep - 1));
            for n in (1..self.keep - 1).rev() {
                let _ = std::fs::rename(numbered(n), numbered(n + 1));
            }
            std::fs::rename(&current, numbered(1))?;
        } else {
            std::fs::remove_file(&current)?;
        }
        *file = open_append(&current)?;
        self.redirect(file);
        Ok(())
    }

    /// Rotate if `extra` more bytes would take the file past the bound. The
    /// file's own length is read, not a running count: stderr shares this file.
    fn rotate_if_full(&self, extra: u64) -> io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if file.metadata().map(|m| m.len()).unwrap_or(0) + extra > self.max {
            self.rotate(&mut file)?;
        }
        Ok(())
    }

    fn redirect(&self, file: &File) {
        if self.redirect_stderr {
            let _ = rustix::stdio::dup2_stderr(file);
        }
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if file.metadata()?.permissions().mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

impl Write for &RotatingLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let inner = &self.0;
        inner.rotate_if_full(buf.len() as u64)?;
        let mut file = inner.file.lock().unwrap_or_else(|e| e.into_inner());
        file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RotatingLog {
    type Writer = &'a RotatingLog;
    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

#[cfg(test)]
#[path = "daemon_log_tests.rs"]
mod tests;

//! A git whose output is read only up to a cap (ov-519, review 2a L7).
//!
//! `git::git_bytes_by` keeps everything a git prints, which is right for the
//! daemon's own plumbing and wrong for a listing a repository decides the
//! size of: `ls-files --others` in a checkout whose `node_modules` isn't
//! ignored prints millions of paths. This reads stdout as it comes, stops at
//! `cap` bytes, and kills the git, under the same pins, exec allowlist and
//! deadline as every other git the daemon runs.

use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::time::{Instant, timeout_at};

/// What a capped git printed, and whether the cap cut it short.
#[derive(Debug)]
pub struct Capped {
    pub stdout: Vec<u8>,
    pub cut: bool,
}

/// `args` in `cwd`, its stdout read up to `cap` bytes, all by `deadline`.
/// `None` when git couldn't start, failed, or ran out of time; a git the cap
/// stopped is not a failure.
pub async fn git_capped(deadline: Instant, cwd: &Path, args: &[&str], cap: usize) -> Option<Capped> {
    let launch = crate::git::git_launch().ok()?;
    let pins = timeout_at(deadline, crate::git::guard_pins(cwd)).await.ok()?.ok()?;
    let mut cmd = launch.command().ok()?;
    crate::git_guard::apply(&mut cmd, &pins);
    crate::git_guard::pin_work_tree(&mut cmd, cwd);
    if let Some(path) = crate::git_lfs::path(crate::git_guard::child_path()) {
        cmd.env("PATH", path);
    }
    let mut child = tokio::process::Command::from(cmd)
        .current_dir(cwd)
        .args(crate::git_guard::args(args))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let read = async {
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = stdout.read(&mut chunk).await?;
            if n == 0 {
                return Ok::<_, std::io::Error>(Capped { stdout: buf, cut: false });
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() >= cap {
                buf.truncate(cap);
                return Ok(Capped { stdout: buf, cut: true });
            }
        }
    };
    let capped = timeout_at(deadline, read).await.ok()?.ok()?;
    if capped.cut {
        let _ = child.start_kill();
        let _ = timeout_at(deadline, child.wait()).await;
        return Some(capped);
    }
    let status = timeout_at(deadline, child.wait()).await.ok()?.ok()?;
    status.success().then_some(capped)
}

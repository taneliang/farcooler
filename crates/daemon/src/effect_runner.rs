//! A supervised process group for workflow fixtures and configured commands.
//! A leader exiting is not proof that its background writers have stopped.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub(crate) struct Group(Option<u32>);
impl Group {
    pub(crate) fn new(pid: u32) -> Self { Self(Some(pid)) }

    /// Caller retains its unreaped leader until this signal, pinning the PGID.
    /// Disarm after one attempt, even on error, before the caller may reap.
    /// An error requires independent disappearance proof, never a late retry.
    pub(crate) fn kill(&mut self) -> std::io::Result<()> {
        let Some(pid) = self.0.take() else { return Ok(()); };
        if unsafe { libc::killpg(pid as i32, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error); }
        }
        Ok(())
    }

    pub(crate) fn gone(pid: u32) -> std::io::Result<bool> {
        if unsafe { libc::killpg(pid as i32, 0) } == 0 { return Ok(false); }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) { Ok(true) } else { Err(error) }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        // SAFETY: this is the group created for our child, never a discovered PID.
        if let Some(pid) = self.0 { unsafe { libc::killpg(pid as i32, libc::SIGKILL); } }
    }
}

/// Peek only: the retained Child remains the sole reaper and pins the owned
/// group identity until its one signal, even when the leader already exited.
pub(crate) async fn exited(pid: u32) -> std::io::Result<()> {
    loop {
        if has_exited(pid)? { return Ok(()); }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn has_exited(pid: u32) -> std::io::Result<bool> {
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    if unsafe { libc::waitid(libc::P_PID, pid, &mut info,
        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted { return Err(error); }
        return Ok(false);
    }
    Ok(unsafe { info.si_pid() } == pid as i32)
}

pub(crate) async fn fixture(tree: &Path, script: &str, log: &Path, limit: Duration,
    environment: &[(&str, &str)]) -> std::io::Result<bool>
{
    supervised(tree, script, log, limit, Some(environment)).await
}
async fn supervised(tree: &Path, script: &str, log: &Path, limit: Duration,
    environment: Option<&[(&str, &str)]>) -> std::io::Result<bool>
{
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);
    if let Some(environment) = environment {
        command.env_clear().env("PATH", "/usr/bin:/bin").envs(environment.iter().copied());
    }
    supervise(command, tree, log, limit, None, None).await.map(|status| status == Some(0))
}

/// Configured argv uses the same group ownership/quiescence boundary as gates.
/// No inherited credentials/environment; callers explicitly pin every variable.
pub(crate) async fn configured(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>) -> std::io::Result<Option<i32>>
{
    owned(tree, argv, environment, log, limit, cancel, None).await
}

/// Like `configured`, but records who the process is in `owner` as soon as it exists, so that a
/// later run that finds the process still going (the daemon died and left it) can end it. The
/// record is a pid and the process's start time; see `Owner::reap`.
pub(crate) async fn owned(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>, owner: Option<&Path>) -> std::io::Result<Option<i32>>
{
    let program = argv.first().ok_or_else(|| std::io::Error::other("empty argv"))?;
    let mut command = Command::new(program);
    command.args(&argv[1..]).env_clear().envs(environment);
    supervise(command, tree, log, limit, Some(cancel), owner).await
}

/// A process this runtime started and may have outlived: its pid, which is also its process group,
/// and when it started, so a recycled pid is never taken for it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Owner { pub pid: u32, pub started: String }
impl Owner {
    fn started_at(pid: u32) -> String {
        std::process::Command::new("/bin/ps").args(["-o", "lstart=", "-p", &pid.to_string()]).env_clear().output()
            .ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }
    pub(crate) fn write(path: &Path, pid: u32) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&Owner { pid, started: Self::started_at(pid) }).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::File::open(&temporary)?.sync_all()?;
        std::fs::rename(&temporary, path)
    }
    /// End whatever the record at `path` still owns. True when nothing of it is left. A group that is
    /// alive with its leader gone is ours (a pid is never reused while its group lives); a live leader
    /// that started at another time is someone else's, and left alone.
    pub(crate) async fn reap(path: &Path) -> std::io::Result<bool> {
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(error),
        };
        let owner: Owner = serde_json::from_slice(&raw).map_err(std::io::Error::other)?;
        // macOS answers EPERM for a group holding only a zombie; that is a group on its way out, not an error.
        if matches!(Group::gone(owner.pid), Ok(true)) { return Ok(true); }
        let leader_alive = unsafe { libc::kill(owner.pid as i32, 0) } == 0;
        if leader_alive && (owner.started.is_empty() || Self::started_at(owner.pid) != owner.started) { return Ok(true); }
        if let Err(error) = Group::new(owner.pid).kill() { tracing::warn!(pid = owner.pid, %error, "signalling an old group failed; waiting to see it go"); }
        for _ in 0..100 {
            if matches!(Group::gone(owner.pid), Ok(true)) { return Ok(true); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(false)
    }
}

async fn supervise(mut command: Command, tree: &Path, log: &Path, limit: Duration,
    mut cancel: Option<tokio::sync::watch::Receiver<bool>>, owner: Option<&Path>) -> std::io::Result<Option<i32>>
{
    let file = std::fs::OpenOptions::new().append(true).create(true).open(log)?;
    command.current_dir(tree)
        .stdin(Stdio::null()).stdout(file.try_clone()?).stderr(file).kill_on_drop(true);
    command.process_group(0);
    let mut child = command.spawn()?;
    let pid = child.id().expect("newly spawned child has a pid");
    let mut group = Group(Some(pid));
    if let Some(owner) = owner { Owner::write(owner, pid)?; }
    let result = tokio::select! {
        result = tokio::time::timeout(limit, exited(pid)) => Some(result),
        _ = async {
            if let Some(cancel) = cancel.as_mut() {
                while !*cancel.borrow_and_update() {
                    if cancel.changed().await.is_err() { break; }
                }
            } else { std::future::pending::<()>().await; }
        } => None,
    };
    // Signal before releasing the leader identity. Disarm even on signal error:
    // no retry may target a recycled PGID after the child has been reaped.
    #[cfg(test)]
    let _ = TEST_SIGNAL_PINNED.try_with(|pinned| {
        let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
        pinned.set(unsafe { libc::waitid(libc::P_PID, pid, &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT) } == 0);
    });
    let signal_error = group.kill().err();
    let timed_out = matches!(result, Some(Err(_)));
    let observed = matches!(result, Some(Ok(Ok(()))));
    let status = tokio::time::timeout(Duration::from_secs(1), child.wait()).await
        .map_err(|_| std::io::Error::other("owned process leader reap unproved"))??;
    // A surviving group (including unreaped grandchildren) is not quiescence.
    for _ in 0..100 {
        if matches!(Group::gone(pid), Ok(true)) {
            if let Some(Ok(Err(error))) = result { return Err(error); }
            return if timed_out { Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "process deadline exceeded")) }
                else { Ok(if observed { status.code() } else { None }) };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if let Some(error) = signal_error {
        tracing::warn!(pid, %error, "owned process signal failed and disappearance remains unproved");
    }
    Err(std::io::Error::other("process group did not become quiescent"))
}

#[cfg(test)]
tokio::task_local! { static TEST_SIGNAL_PINNED: std::cell::Cell<bool>; }

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn workflow_runtime_exited_leader_keeps_group_owned_until_descendant_stops() {
        let dir = tempfile::tempdir().unwrap();
        let result = TEST_SIGNAL_PINNED.scope(std::cell::Cell::new(false), async {
            let result = supervised(dir.path(), "sleep 30 & echo $! > descendant.pid; exit 0",
                &dir.path().join("leader.log"), Duration::from_secs(2), None).await.unwrap();
            TEST_SIGNAL_PINNED.with(|pinned| assert!(pinned.get(), "leader was reaped before group signal"));
            result
        }).await;
        assert!(result);
        let descendant: i32 = std::fs::read_to_string(dir.path().join("descendant.pid"))
            .unwrap().trim().parse().unwrap();
        assert_eq!(unsafe { libc::kill(descendant, 0) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
    #[tokio::test]
    async fn workflow_runtime_process_timeout_and_cancel_kill_owned_group() {
        let dir = tempfile::tempdir().unwrap();
        let script = "echo $$ > leader.pid; sleep 30 & wait";
        assert_eq!(supervised(dir.path(), script, &dir.path().join("timeout.log"), Duration::from_millis(50), None)
            .await.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        let pid: i32 = std::fs::read_to_string(dir.path().join("leader.pid")).unwrap().trim().parse().unwrap();
        assert_ne!(unsafe { libc::killpg(pid, 0) }, 0);
        let tree = dir.path().to_path_buf();
        let task = tokio::spawn(async move {
            supervised(&tree, script, &tree.join("cancel.log"), Duration::from_secs(30), None).await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let pid: i32 = std::fs::read_to_string(dir.path().join("leader.pid")).unwrap().trim().parse().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        for _ in 0..100 {
            if unsafe { libc::killpg(pid, 0) } != 0 { return; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("owned group survived cancellation");
    }
}

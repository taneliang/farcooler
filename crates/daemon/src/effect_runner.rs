//! A supervised process group for workflow fixtures and the retained landing gate.
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

pub(crate) async fn run(tree: &Path, script: &str, log: &Path, limit: Duration)
    -> std::io::Result<bool>
{
    supervised(tree, script, log, limit, None).await
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
    supervise(command, tree, log, limit, None).await.map(|status| status == Some(0))
}

/// Configured argv uses the same group ownership/quiescence boundary as gates.
/// No inherited credentials/environment; callers explicitly pin every variable.
pub(crate) async fn configured(tree: &Path, argv: &[String], environment: &std::collections::BTreeMap<String, String>,
    log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>) -> std::io::Result<Option<i32>>
{
    let program = argv.first().ok_or_else(|| std::io::Error::other("empty argv"))?;
    let mut command = Command::new(program);
    command.args(&argv[1..]).env_clear().envs(environment);
    supervise(command, tree, log, limit, Some(cancel)).await
}

async fn supervise(mut command: Command, tree: &Path, log: &Path, limit: Duration,
    mut cancel: Option<tokio::sync::watch::Receiver<bool>>) -> std::io::Result<Option<i32>>
{
    let file = std::fs::OpenOptions::new().append(true).create(true).open(log)?;
    command.current_dir(tree)
        .stdin(Stdio::null()).stdout(file.try_clone()?).stderr(file).kill_on_drop(true);
    command.process_group(0);
    let mut child = command.spawn()?;
    let pid = child.id().expect("newly spawned child has a pid");
    let mut group = Group(Some(pid));
    let result = tokio::select! {
        result = tokio::time::timeout(limit, child.wait()) => Some(result),
        _ = async {
            if let Some(cancel) = cancel.as_mut() {
                while !*cancel.borrow_and_update() {
                    if cancel.changed().await.is_err() { break; }
                }
            } else { std::future::pending::<()>().await; }
        } => None,
    };
    // Always stop descendants, including on success and timeout. Keep the group
    // guard across every await so cancellation also kills the group.
    unsafe { libc::killpg(pid as i32, libc::SIGKILL); }
    let timed_out = matches!(result, Some(Err(_)));
    let status = match result {
        Some(Ok(status)) => status?.code(),
        Some(Err(_)) | None => { child.wait().await?; None }
    };
    // A surviving group (including unreaped grandchildren) is not quiescence.
    for _ in 0..100 {
        let alive = unsafe { libc::killpg(pid as i32, 0) } == 0;
        if !alive {
            group.0 = None;
            return if timed_out { Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "process deadline exceeded")) }
                else { Ok(status) };
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(std::io::Error::other("process group did not become quiescent"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn workflow_runtime_process_timeout_and_cancel_kill_owned_group() {
        let dir = tempfile::tempdir().unwrap();
        let script = "echo $$ > leader.pid; sleep 30 & wait";
        assert_eq!(run(dir.path(), script, &dir.path().join("timeout.log"), Duration::from_millis(50))
            .await.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        let pid: i32 = std::fs::read_to_string(dir.path().join("leader.pid")).unwrap().trim().parse().unwrap();
        assert_ne!(unsafe { libc::killpg(pid, 0) }, 0);
        let tree = dir.path().to_path_buf();
        let task = tokio::spawn(async move {
            run(&tree, script, &tree.join("cancel.log"), Duration::from_secs(30)).await
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

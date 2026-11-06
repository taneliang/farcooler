//! A supervised process group for workflow fixtures and the retained landing gate.
//! A leader exiting is not proof that its background writers have stopped.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

struct Group(Option<u32>);
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
    let file = std::fs::OpenOptions::new().append(true).create(true).open(log)?;
    let mut command = Command::new("/bin/sh");
    if let Some(environment) = environment {
        command.env_clear().env("PATH", "/usr/bin:/bin").envs(environment.iter().copied());
    }
    command.arg("-c").arg(script).current_dir(tree)
        .stdin(Stdio::null()).stdout(file.try_clone()?).stderr(file).kill_on_drop(true);
    command.process_group(0);
    let mut child = command.spawn()?;
    let pid = child.id().expect("newly spawned child has a pid");
    let mut group = Group(Some(pid));
    let result = tokio::time::timeout(limit, child.wait()).await;
    // Always stop descendants, including on success and timeout. Keep the group
    // guard across every await so cancellation also kills the group.
    unsafe { libc::killpg(pid as i32, libc::SIGKILL); }
    let timed_out = result.is_err();
    let passed = match result {
        Ok(status) => status?.success(),
        Err(_) => { child.wait().await?; false }
    };
    // A surviving group (including unreaped grandchildren) is not quiescence.
    for _ in 0..100 {
        let alive = unsafe { libc::killpg(pid as i32, 0) } == 0;
        if !alive {
            group.0 = None;
            return if timed_out { Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "process deadline exceeded")) }
                else { Ok(passed) };
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
